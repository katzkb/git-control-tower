use std::collections::HashMap;

use crate::git::command::{run_gh, run_git};
use crate::git::types::{Branch, BranchEntry, GitStatus, PullRequest, ReviewRequest, Worktree};

/// Merge local branches, worktrees, and PRs into unified BranchEntry list.
pub fn merge_entries(
    branches: &[Branch],
    worktrees: &[Worktree],
    pull_requests: &[PullRequest],
) -> Vec<BranchEntry> {
    let mut map: HashMap<String, BranchEntry> = HashMap::new();

    // Add local branches
    for branch in branches {
        map.entry(branch.name.clone())
            .or_insert_with(|| BranchEntry {
                name: branch.name.clone(),
                local_branch: None,
                worktree: None,
                pull_request: None,
                git_status: None,
            })
            .local_branch = Some(branch.clone());
    }

    // Add worktrees (match by branch name)
    for wt in worktrees {
        if let Some(branch_name) = &wt.branch {
            map.entry(branch_name.clone())
                .or_insert_with(|| BranchEntry {
                    name: branch_name.clone(),
                    local_branch: None,
                    worktree: None,
                    pull_request: None,
                    git_status: None,
                })
                .worktree = Some(wt.clone());
        }
    }

    // Add PRs (match by head_ref, prefer OPEN over MERGED)
    for pr in pull_requests {
        let entry = map
            .entry(pr.head_ref.clone())
            .or_insert_with(|| BranchEntry {
                name: pr.head_ref.clone(),
                local_branch: None,
                worktree: None,
                pull_request: None,
                git_status: None,
            });
        match (&entry.pull_request, pr.state.as_str()) {
            (Some(existing), "MERGED") if existing.state == "OPEN" => {
                // Don't overwrite an OPEN PR with a MERGED one
            }
            _ => {
                entry.pull_request = Some(pr.clone());
            }
        }
    }

    // Sort: current branch first, then alphabetical
    let mut entries: Vec<BranchEntry> = map.into_values().collect();
    entries.sort_by(|a, b| {
        let a_current = a.is_current();
        let b_current = b.is_current();
        b_current.cmp(&a_current).then(a.name.cmp(&b.name))
    });

    entries
}

/// Parse `git status --porcelain=v1` output into GitStatus.
pub fn parse_git_status(output: &str) -> GitStatus {
    let mut status = GitStatus::default();

    for line in output.lines() {
        if line.len() < 3 {
            continue;
        }
        let index = line.as_bytes()[0];
        let work = line.as_bytes()[1];
        let file = line[3..].to_string();

        if index == b'?' && work == b'?' {
            status.untracked.push(file);
        } else {
            if work != b' ' && work != b'?' {
                status.unstaged.push(format!("{} {}", work as char, &file));
            }
            if index != b' ' && index != b'?' {
                status.staged.push(format!("{} {}", index as char, &file));
            }
        }
    }

    status
}

/// Parse ahead/behind from `git rev-list --left-right --count HEAD...@{u}` output.
pub fn parse_ahead_behind(output: &str) -> (u32, u32) {
    let parts: Vec<&str> = output.trim().split('\t').collect();
    if parts.len() == 2 {
        let ahead = parts[0].parse().unwrap_or(0);
        let behind = parts[1].parse().unwrap_or(0);
        (ahead, behind)
    } else {
        (0, 0)
    }
}

/// Load git status for a worktree directory.
pub async fn load_git_status(worktree_path: &str) -> Option<GitStatus> {
    let status_output = run_git(&["-C", worktree_path, "status", "--porcelain=v1"])
        .await
        .ok()?;

    let mut status = parse_git_status(&status_output);

    // Try to get ahead/behind
    if let Ok(ab_output) = run_git(&[
        "-C",
        worktree_path,
        "rev-list",
        "--left-right",
        "--count",
        "HEAD...@{u}",
    ])
    .await
    {
        let (ahead, behind) = parse_ahead_behind(&ab_output);
        status.ahead = ahead;
        status.behind = behind;
    }

    Some(status)
}

/// Generate a deterministic, index-based GraphQL alias for a branch query.
fn graphql_alias(index: usize) -> String {
    format!("b{index}")
}

/// Fetch PRs for local branches via GraphQL aliases (200 branches per request).
/// Each branch gets an exact-match query on `headRefName`.
pub async fn fetch_local_prs(
    branch_names: &[String],
    owner: &str,
    repo: &str,
    hostname: Option<&str>,
) -> Vec<PullRequest> {
    if branch_names.is_empty() {
        return Vec::new();
    }

    let mut all_prs = Vec::new();

    // Process in chunks of 200 (GraphQL query size limit)
    for chunk in branch_names.chunks(200) {
        let mut aliases = String::new();
        for (i, name) in chunk.iter().enumerate() {
            let alias = graphql_alias(i);
            let escaped_name = name.replace('\\', "\\\\").replace('"', "\\\"");
            aliases.push_str(&format!(
                r#"{alias}: pullRequests(first: 2, headRefName: "{escaped_name}", states: [OPEN, MERGED], orderBy: {{field: UPDATED_AT, direction: DESC}}) {{
  nodes {{ number title state headRefName updatedAt author {{ login }}
    reviewRequests(first: 10) {{ nodes {{ requestedReviewer {{ ... on User {{ login }} }} }} }}
  }}
}}
"#
            ));
        }

        let owner_escaped = owner.replace('\\', "\\\\").replace('"', "\\\"");
        let repo_escaped = repo.replace('\\', "\\\\").replace('"', "\\\"");
        let query = format!(
            r#"{{ repository(owner: "{owner_escaped}", name: "{repo_escaped}") {{ {aliases} }} }}"#
        );

        let query_arg = format!("query={query}");
        let mut args = vec!["api", "graphql", "-f", &query_arg];
        let hostname_owned;
        if let Some(h) = hostname {
            hostname_owned = h.to_string();
            args.push("--hostname");
            args.push(&hostname_owned);
        }

        if let Ok(output) = run_gh(&args).await
            && let Ok(json) = serde_json::from_str::<serde_json::Value>(&output)
        {
            let repo_data = &json["data"]["repository"];
            for (i, _) in chunk.iter().enumerate() {
                let alias = graphql_alias(i);
                if let Some(nodes) = repo_data[&alias]["nodes"].as_array() {
                    for node in nodes {
                        if let Some(pr) = parse_graphql_pr(node) {
                            all_prs.push(pr);
                        }
                    }
                }
            }
        }
    }

    all_prs
}

/// Parse a single PR node from GraphQL response into PullRequest.
fn parse_graphql_pr(node: &serde_json::Value) -> Option<PullRequest> {
    let number = node["number"].as_u64()?;
    let title = node["title"].as_str()?.to_string();
    let state = node["state"].as_str()?.to_string();
    let head_ref = node["headRefName"].as_str()?.to_string();
    let updated_at = node["updatedAt"].as_str().unwrap_or_default().to_string();
    let author = node["author"]["login"]
        .as_str()
        .unwrap_or_default()
        .to_string();

    let review_requests = node["reviewRequests"]["nodes"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|r| {
                    r["requestedReviewer"]["login"]
                        .as_str()
                        .map(|login| ReviewRequest {
                            login: login.to_string(),
                        })
                })
                .collect()
        })
        .unwrap_or_default();

    Some(PullRequest {
        number,
        title,
        author,
        state,
        head_ref,
        updated_at,
        review_requests,
    })
}

/// Fetch PRs authored by the current user (`gh pr list --author @me`).
pub async fn fetch_my_prs(show_merged: bool) -> Vec<PullRequest> {
    fetch_pr_list(&["--author", "@me"], show_merged).await
}

/// Fetch PRs with review requested from the current user
/// (`gh pr list --search "review-requested:@me"`).
pub async fn fetch_review_prs(show_merged: bool) -> Vec<PullRequest> {
    fetch_pr_list(&["--search", "review-requested:@me"], show_merged).await
}

/// Common helper for fetching PR lists with a filter.
/// `filter_args` is passed directly to `gh pr list` (e.g., `["--author", "@me"]`).
async fn fetch_pr_list(filter_args: &[&str], show_merged: bool) -> Vec<PullRequest> {
    let pr_fields = "number,title,author,state,headRefName,updatedAt,reviewRequests";
    let mut prs = Vec::new();

    // Always fetch open PRs
    let mut args = vec!["pr", "list"];
    args.extend_from_slice(filter_args);
    args.extend_from_slice(&["--json", pr_fields, "--limit", "100"]);
    if let Ok(output) = run_gh(&args).await
        && let Ok(open) = serde_json::from_str::<Vec<PullRequest>>(&output)
    {
        prs.extend(open);
    }

    // Optionally fetch merged PRs
    if show_merged {
        let mut args = vec!["pr", "list"];
        args.extend_from_slice(filter_args);
        args.extend_from_slice(&["--state", "merged", "--json", pr_fields, "--limit", "50"]);
        if let Ok(output) = run_gh(&args).await
            && let Ok(merged) = serde_json::from_str::<Vec<PullRequest>>(&output)
        {
            prs.extend(merged);
        }
    }

    prs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_merge_entries_basic() {
        let branches = vec![
            Branch {
                name: "main".to_string(),
                is_current: true,
                upstream: Some("origin/main".to_string()),
                is_merged: false,
            },
            Branch {
                name: "feature-a".to_string(),
                is_current: false,
                upstream: None,
                is_merged: false,
            },
        ];
        let worktrees = vec![];
        let prs = vec![];

        let entries = merge_entries(&branches, &worktrees, &prs);
        assert_eq!(entries.len(), 2);
        // Current branch should come first
        assert_eq!(entries[0].name, "main");
        assert!(entries[0].is_current());
        assert_eq!(entries[1].name, "feature-a");
    }

    #[test]
    fn test_merge_entries_with_pr() {
        let branches = vec![Branch {
            name: "feature-a".to_string(),
            is_current: false,
            upstream: None,
            is_merged: false,
        }];
        let worktrees = vec![];
        let prs = vec![PullRequest {
            number: 42,
            title: "Add feature A".to_string(),
            author: "alice".to_string(),
            state: "OPEN".to_string(),
            head_ref: "feature-a".to_string(),
            updated_at: "2024-01-15".to_string(),
            review_requests: vec![],
        }];

        let entries = merge_entries(&branches, &worktrees, &prs);
        assert_eq!(entries.len(), 1);
        assert!(entries[0].local_branch.is_some());
        assert!(entries[0].pull_request.is_some());
        assert_eq!(entries[0].pr_number(), Some(42));
    }

    #[test]
    fn test_merge_entries_remote_only_pr() {
        let branches = vec![];
        let worktrees = vec![];
        let prs = vec![PullRequest {
            number: 99,
            title: "Remote only PR".to_string(),
            author: "bob".to_string(),
            state: "OPEN".to_string(),
            head_ref: "remote-branch".to_string(),
            updated_at: "2024-01-15".to_string(),
            review_requests: vec![],
        }];

        let entries = merge_entries(&branches, &worktrees, &prs);
        assert_eq!(entries.len(), 1);
        assert!(!entries[0].has_local());
        assert!(entries[0].pull_request.is_some());
    }

    #[test]
    fn test_pr_is_merged() {
        let branches = vec![Branch {
            name: "feature-a".to_string(),
            is_current: false,
            upstream: None,
            is_merged: false,
        }];
        let prs = vec![PullRequest {
            number: 10,
            title: "Feature A".to_string(),
            author: "alice".to_string(),
            state: "MERGED".to_string(),
            head_ref: "feature-a".to_string(),
            updated_at: "2024-01-15".to_string(),
            review_requests: vec![],
        }];

        let entries = merge_entries(&branches, &[], &prs);
        assert_eq!(entries.len(), 1);
        // git says not merged, but PR says merged
        assert!(!entries[0].is_merged());
        assert!(entries[0].pr_is_merged());
    }

    #[test]
    fn test_open_pr_preferred_over_merged() {
        let branches = vec![Branch {
            name: "feature-a".to_string(),
            is_current: false,
            upstream: None,
            is_merged: false,
        }];
        let prs = vec![
            PullRequest {
                number: 5,
                title: "Old merged PR".to_string(),
                author: "alice".to_string(),
                state: "MERGED".to_string(),
                head_ref: "feature-a".to_string(),
                updated_at: "2024-01-01".to_string(),
                review_requests: vec![],
            },
            PullRequest {
                number: 10,
                title: "New open PR".to_string(),
                author: "alice".to_string(),
                state: "OPEN".to_string(),
                head_ref: "feature-a".to_string(),
                updated_at: "2024-01-15".to_string(),
                review_requests: vec![],
            },
        ];

        let entries = merge_entries(&branches, &[], &prs);
        assert_eq!(entries.len(), 1);
        // OPEN PR should win over MERGED
        assert_eq!(entries[0].pr_number(), Some(10));
        assert!(!entries[0].pr_is_merged());
    }

    #[test]
    fn test_parse_git_status() {
        let output = "?? new_file.txt\n M modified.txt\nA  staged.txt\nMM both.txt\n";
        let status = parse_git_status(output);
        assert_eq!(status.untracked, vec!["new_file.txt"]);
        assert_eq!(status.unstaged, vec!["M modified.txt", "M both.txt"]);
        assert_eq!(status.staged, vec!["A staged.txt", "M both.txt"]);
    }

    #[test]
    fn test_parse_ahead_behind() {
        assert_eq!(parse_ahead_behind("3\t1\n"), (3, 1));
        assert_eq!(parse_ahead_behind("0\t0\n"), (0, 0));
        assert_eq!(parse_ahead_behind(""), (0, 0));
    }

    #[test]
    fn test_parse_graphql_pr_valid() {
        let node = serde_json::json!({
            "number": 42,
            "title": "Add feature",
            "state": "OPEN",
            "headRefName": "feature-branch",
            "updatedAt": "2024-01-15T00:00:00Z",
            "author": { "login": "alice" },
            "reviewRequests": {
                "nodes": [
                    { "requestedReviewer": { "login": "bob" } }
                ]
            }
        });
        let pr = parse_graphql_pr(&node).unwrap();
        assert_eq!(pr.number, 42);
        assert_eq!(pr.title, "Add feature");
        assert_eq!(pr.state, "OPEN");
        assert_eq!(pr.head_ref, "feature-branch");
        assert_eq!(pr.author, "alice");
        assert_eq!(pr.review_requests.len(), 1);
        assert_eq!(pr.review_requests[0].login, "bob");
    }

    #[test]
    fn test_parse_graphql_pr_null_author() {
        let node = serde_json::json!({
            "number": 10,
            "title": "Bot PR",
            "state": "MERGED",
            "headRefName": "bot-branch",
            "updatedAt": "2024-01-01T00:00:00Z",
            "author": null,
            "reviewRequests": { "nodes": [] }
        });
        let pr = parse_graphql_pr(&node).unwrap();
        assert_eq!(pr.number, 10);
        assert_eq!(pr.author, "");
    }

    #[test]
    fn test_parse_graphql_pr_missing_required_field() {
        // Missing "number" field
        let node = serde_json::json!({
            "title": "Incomplete",
            "state": "OPEN",
            "headRefName": "some-branch"
        });
        assert!(parse_graphql_pr(&node).is_none());
    }
}
