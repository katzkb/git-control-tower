use std::collections::{HashMap, HashSet};

use crate::git::command::{debug_log, run_gh, run_git};
use crate::git::types::{Branch, BranchEntry, GitStatus, PullRequest, ReviewRequest, Worktree};

/// Merge local branches, worktrees, and PRs into unified BranchEntry list.
pub fn merge_entries(
    active_repo: &crate::git::types::RepoId,
    branches: &[Branch],
    worktrees: &[Worktree],
    pull_requests: &[PullRequest],
    wt_lists_per_repo: &std::collections::HashMap<
        crate::git::types::RepoId,
        Vec<crate::git::types::Worktree>,
    >,
) -> Vec<BranchEntry> {
    use crate::git::types::RepoId;
    let mut map: HashMap<(RepoId, String), BranchEntry> = HashMap::new();

    // Local branches → all keyed under active_repo
    for branch in branches {
        let key = (active_repo.clone(), branch.name.clone());
        map.entry(key.clone())
            .or_insert_with(|| BranchEntry {
                name: branch.name.clone(),
                repo_id: active_repo.clone(),
                local_branch: None,
                worktree: None,
                pull_request: None,
                git_status: None,
            })
            .local_branch = Some(branch.clone());
    }

    // Active-repo worktrees
    for wt in worktrees {
        if let Some(branch_name) = &wt.branch {
            let key = (active_repo.clone(), branch_name.clone());
            map.entry(key.clone())
                .or_insert_with(|| BranchEntry {
                    name: branch_name.clone(),
                    repo_id: active_repo.clone(),
                    local_branch: None,
                    worktree: None,
                    pull_request: None,
                    git_status: None,
                })
                .worktree = Some(wt.clone());
        }
    }

    // PRs (active repo merges with branches/worktrees, other repos are PR-only)
    for pr in pull_requests {
        let key = (pr.repo_id.clone(), pr.head_ref.clone());
        let entry = map.entry(key.clone()).or_insert_with(|| BranchEntry {
            name: pr.head_ref.clone(),
            repo_id: pr.repo_id.clone(),
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

        // Cross-repo worktree injection from wt_lists_per_repo
        if pr.repo_id != *active_repo
            && entry.worktree.is_none()
            && let Some(wts) = wt_lists_per_repo.get(&pr.repo_id)
            && let Some(wt) = wts
                .iter()
                .find(|w| w.branch.as_deref() == Some(&pr.head_ref))
        {
            entry.worktree = Some(wt.clone());
        }
    }

    // Sort: active repo first, then RepoId string order; current branch first within active repo
    let mut entries: Vec<BranchEntry> = map.into_values().collect();
    entries.sort_by(|a, b| {
        let a_active = a.repo_id == *active_repo;
        let b_active = b.repo_id == *active_repo;
        b_active
            .cmp(&a_active)
            .then_with(|| a.repo_id.to_string().cmp(&b.repo_id.to_string()))
            .then_with(|| {
                let a_cur = a.is_current();
                let b_cur = b.is_current();
                b_cur.cmp(&a_cur)
            })
            .then_with(|| a.name.cmp(&b.name))
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
/// Returns (prs, errors) where errors contains any fetch/parse failures.
pub async fn fetch_local_prs(
    branch_names: &[String],
    owner: &str,
    repo: &str,
    hostname: Option<&str>,
) -> (Vec<PullRequest>, Vec<String>) {
    if branch_names.is_empty() {
        return (Vec::new(), Vec::new());
    }

    let mut all_prs = Vec::new();
    let mut errors = Vec::new();

    // Process in chunks of 200 (GraphQL query size limit)
    for chunk in branch_names.chunks(200) {
        let mut aliases = String::new();
        for (i, name) in chunk.iter().enumerate() {
            let alias = graphql_alias(i);
            let escaped_name = name.replace('\\', "\\\\").replace('"', "\\\"");
            aliases.push_str(&format!(
                r#"{alias}: pullRequests(first: 2, headRefName: "{escaped_name}", states: [OPEN, MERGED], orderBy: {{field: UPDATED_AT, direction: DESC}}) {{
  nodes {{ number title state headRefName updatedAt isDraft author {{ login }}
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

        match run_gh(&args).await {
            Ok(output) => match serde_json::from_str::<serde_json::Value>(&output) {
                Ok(json) => {
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
                Err(e) => {
                    debug_log(&format!("  → GraphQL JSON parse error: {e}"));
                    errors.push(format!("GraphQL parse error: {e}"));
                }
            },
            Err(e) => {
                debug_log(&format!("  → GraphQL fetch error: {e}"));
                errors.push(format!("GraphQL fetch failed: {e}"));
            }
        }
    }

    let repo_id_for_local = crate::git::types::RepoId {
        host: hostname.map(|h| h.to_string()),
        owner: owner.to_string(),
        name: repo.to_string(),
    };
    for pr in &mut all_prs {
        if pr.repo_id.owner.is_empty() {
            pr.repo_id = repo_id_for_local.clone();
        }
    }

    (all_prs, errors)
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

    let is_draft = node["isDraft"].as_bool().unwrap_or(false);

    let review_requests = node["reviewRequests"]["nodes"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .map(|r| ReviewRequest {
                    login: r["requestedReviewer"]["login"]
                        .as_str()
                        .map(|s| s.to_string()),
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
        is_draft,
        review_requests,
        latest_reviews: Vec::new(),
        review_status: None,
        repo_id: Default::default(),
    })
}

/// Parse a GraphQL `search.nodes[]` PR (with `repository` field) into PullRequest.
fn parse_graphql_search_pr(node: &serde_json::Value) -> Option<PullRequest> {
    let mut pr = parse_graphql_pr(node)?;
    let repo_owner = node["repository"]["owner"]["login"].as_str()?.to_string();
    let repo_name = node["repository"]["name"].as_str()?.to_string();
    let repo_url = node["repository"]["url"].as_str().unwrap_or("");
    let host = host_from_url(repo_url);
    pr.repo_id = crate::git::types::RepoId {
        host,
        owner: repo_owner,
        name: repo_name,
    };
    if let Some(arr) = node["latestReviews"]["nodes"].as_array() {
        pr.latest_reviews = arr
            .iter()
            .filter_map(|r| {
                Some(crate::git::types::LatestReview {
                    author: r["author"]["login"].as_str()?.to_string(),
                    state: r["state"].as_str()?.to_string(),
                })
            })
            .collect();
    }
    Some(pr)
}

fn host_from_url(url: &str) -> Option<String> {
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))?;
    let host = rest.split('/').next()?;
    if host == "github.com" {
        None
    } else {
        Some(host.to_string())
    }
}

// gh api graphql does not accept --hostname here; v1 supports github.com only.
// Cross-host (GHE) PR aggregation will require routing through per-host gh config.
// TODO(future): GitHub's search index supports up to 1000 results; if very active
// reviewers report missing PRs, paginate via search.pageInfo.endCursor.
async fn fetch_search_prs(query_str: &str, limit: u32) -> (Vec<PullRequest>, Vec<String>) {
    let escaped_query = query_str.replace('\\', "\\\\").replace('"', "\\\"");
    let query = format!(
        r#"{{ search(query: "{escaped_query}", type: ISSUE, first: {limit}) {{ nodes {{ ... on PullRequest {{ number title state headRefName updatedAt isDraft author {{ login }} repository {{ name url owner {{ login }} }} reviewRequests(first: 10) {{ nodes {{ requestedReviewer {{ ... on User {{ login }} }} }} }} latestReviews(first: 20) {{ nodes {{ author {{ login }} state }} }} }} }} }} }}"#
    );
    let query_arg = format!("query={query}");
    let args = vec!["api", "graphql", "-f", &query_arg];
    match run_gh(&args).await {
        Ok(output) => match serde_json::from_str::<serde_json::Value>(&output) {
            Ok(json) => {
                let mut prs = Vec::new();
                if let Some(arr) = json["data"]["search"]["nodes"].as_array() {
                    for node in arr {
                        if let Some(pr) = parse_graphql_search_pr(node) {
                            prs.push(pr);
                        }
                    }
                }
                (prs, Vec::new())
            }
            Err(e) => {
                debug_log(&format!("  → GraphQL search parse error: {e}"));
                (Vec::new(), vec![format!("search parse error: {e}")])
            }
        },
        Err(e) => {
            debug_log(&format!("  → GraphQL search fetch error: {e}"));
            (Vec::new(), vec![format!("search fetch failed: {e}")])
        }
    }
}

/// Fetch PRs authored by the current user via cross-repo GraphQL search.
pub async fn fetch_my_prs(show_merged: bool) -> (Vec<PullRequest>, Vec<String>) {
    let mut q = String::from("is:pr author:@me");
    if !show_merged {
        q.push_str(" is:open");
    }
    // When show_merged is on we lose the OPEN-only narrowing, so widen the limit
    // to compensate (the previous fetch_pr_list returned 100 open + 50 merged).
    // TODO(future): GitHub's search index supports up to 1000 results; if very active
    // reviewers report missing PRs, paginate via search.pageInfo.endCursor.
    let limit = if show_merged { 150 } else { 100 };
    let (mut prs, errors) = fetch_search_prs(&q, limit).await;
    prs.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    (prs, errors)
}

/// Fetch PRs with review requested from the current user via cross-repo GraphQL search.
/// Runs separate queries and merges results to avoid GHE `OR` incompatibility.
/// When `include_team` is true, also includes team review requests.
pub async fn fetch_review_prs(
    show_merged: bool,
    include_team: bool,
    gh_user: &str,
) -> (Vec<PullRequest>, Vec<String>) {
    let mut queries: Vec<String> = vec![
        "is:pr review-requested:@me".into(),
        "is:pr reviewed-by:@me".into(),
    ];
    if include_team {
        queries.push("is:pr team-review-requested:@me".into());
    }
    if !show_merged {
        for q in &mut queries {
            q.push_str(" is:open");
        }
    }

    let mut all_prs = Vec::new();
    let mut all_errors = Vec::new();
    let mut seen: HashSet<(crate::git::types::RepoId, u64)> = HashSet::new();
    let mut reviewed_keys: HashSet<(crate::git::types::RepoId, u64)> = HashSet::new();

    // Two separate sets:
    // - `seen` deduplicates PRs across queries (keyed by RepoId+number).
    // - `reviewed_keys` records every PR returned by the reviewed-by query, even
    //   if `seen` rejects it as a duplicate. This is intentional: the me-only
    //   filter below uses `reviewed_keys` as a predicate, not as a display list.
    const REVIEWED_BY_INDEX: usize = 1;
    for (idx, query) in queries.iter().enumerate() {
        let is_reviewed = idx == REVIEWED_BY_INDEX;
        let (prs, errors) = fetch_search_prs(query, 100).await;
        all_errors.extend(errors);
        for pr in prs {
            let key = (pr.repo_id.clone(), pr.number);
            if is_reviewed {
                reviewed_keys.insert(key.clone());
            }
            if seen.insert(key) {
                all_prs.push(pr);
            }
        }
    }

    // Exclude PRs authored by the current user. `reviewed-by:@me` matches PRs
    // where the user submitted any review, including COMMENT-type reviews the
    // user added on their own PR — those would otherwise leak into the Review tab.
    if !gh_user.is_empty() {
        all_prs.retain(|pr| pr.author != gh_user);
    }

    // When me-only, exclude PRs that only have team review requests
    if !include_team && !gh_user.is_empty() {
        all_prs.retain(|pr| {
            let key = (pr.repo_id.clone(), pr.number);
            reviewed_keys.contains(&key)
                || pr
                    .review_requests
                    .iter()
                    .any(|r| r.login.as_deref() == Some(gh_user))
        });
    }

    all_prs.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    (all_prs, all_errors)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_merge_entries_basic() {
        use crate::git::types::RepoId;
        let active = RepoId::default();
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

        let entries = merge_entries(&active, &branches, &worktrees, &prs, &Default::default());
        assert_eq!(entries.len(), 2);
        // Current branch should come first
        assert_eq!(entries[0].name, "main");
        assert!(entries[0].is_current());
        assert_eq!(entries[1].name, "feature-a");
    }

    #[test]
    fn test_merge_entries_with_pr() {
        use crate::git::types::RepoId;
        let active = RepoId::default();
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
            is_draft: false,
            latest_reviews: vec![],
            review_status: None,
            repo_id: RepoId::default(),
        }];

        let entries = merge_entries(&active, &branches, &worktrees, &prs, &Default::default());
        assert_eq!(entries.len(), 1);
        assert!(entries[0].local_branch.is_some());
        assert!(entries[0].pull_request.is_some());
        assert_eq!(entries[0].pr_number(), Some(42));
    }

    #[test]
    fn test_merge_entries_remote_only_pr() {
        use crate::git::types::RepoId;
        let active = RepoId::default();
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
            is_draft: false,
            latest_reviews: vec![],
            review_status: None,
            repo_id: RepoId::default(),
        }];

        let entries = merge_entries(&active, &branches, &worktrees, &prs, &Default::default());
        assert_eq!(entries.len(), 1);
        assert!(!entries[0].has_local());
        assert!(entries[0].pull_request.is_some());
    }

    #[test]
    fn test_pr_is_merged() {
        use crate::git::types::RepoId;
        let active = RepoId::default();
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
            is_draft: false,
            latest_reviews: vec![],
            review_status: None,
            repo_id: RepoId::default(),
        }];

        let entries = merge_entries(&active, &branches, &[], &prs, &Default::default());
        assert_eq!(entries.len(), 1);
        // git says not merged, but PR says merged
        assert!(!entries[0].is_merged());
        assert!(entries[0].pr_is_merged());
    }

    #[test]
    fn test_open_pr_preferred_over_merged() {
        use crate::git::types::RepoId;
        let active = RepoId::default();
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
                is_draft: false,
                latest_reviews: vec![],
                review_status: None,
                repo_id: RepoId::default(),
            },
            PullRequest {
                number: 10,
                title: "New open PR".to_string(),
                author: "alice".to_string(),
                state: "OPEN".to_string(),
                head_ref: "feature-a".to_string(),
                updated_at: "2024-01-15".to_string(),
                review_requests: vec![],
                is_draft: false,
                latest_reviews: vec![],
                review_status: None,
                repo_id: RepoId::default(),
            },
        ];

        let entries = merge_entries(&active, &branches, &[], &prs, &Default::default());
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
        assert_eq!(pr.review_requests[0].login.as_deref(), Some("bob"));
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

    #[test]
    fn test_parse_graphql_pr_team_reviewer() {
        // GraphQL query uses `... on User { login }`, so Team reviewers
        // appear as empty objects or null (no login field in response)
        let node = serde_json::json!({
            "number": 50,
            "title": "With team review",
            "state": "OPEN",
            "headRefName": "feat-branch",
            "updatedAt": "2024-01-15T00:00:00Z",
            "author": { "login": "alice" },
            "reviewRequests": {
                "nodes": [
                    { "requestedReviewer": { "login": "bob" } },
                    { "requestedReviewer": {} },
                    { "requestedReviewer": null }
                ]
            }
        });
        let pr = parse_graphql_pr(&node).unwrap();
        assert_eq!(pr.review_requests.len(), 3);
        assert_eq!(pr.review_requests[0].login.as_deref(), Some("bob"));
        assert_eq!(pr.review_requests[1].login, None);
        assert_eq!(pr.review_requests[2].login, None);
    }

    #[test]
    fn parse_graphql_search_pr_with_repo() {
        let node = serde_json::json!({
            "number": 42,
            "title": "Cross repo",
            "state": "OPEN",
            "headRefName": "feat",
            "updatedAt": "2024-01-15T00:00:00Z",
            "isDraft": false,
            "author": { "login": "alice" },
            "repository": {
                "name": "repo",
                "owner": { "login": "owner" },
                "url": "https://github.com/owner/repo"
            },
            "reviewRequests": { "nodes": [] },
            "latestReviews": { "nodes": [] }
        });
        let pr = parse_graphql_search_pr(&node).unwrap();
        assert_eq!(pr.number, 42);
        assert_eq!(pr.repo_id.owner, "owner");
        assert_eq!(pr.repo_id.name, "repo");
        assert!(pr.repo_id.host.is_none());
    }

    #[test]
    fn parse_graphql_search_pr_with_ghe_repo() {
        let node = serde_json::json!({
            "number": 1,
            "title": "GHE",
            "state": "OPEN",
            "headRefName": "x",
            "updatedAt": "2024-01-15T00:00:00Z",
            "isDraft": false,
            "author": { "login": "a" },
            "repository": {
                "name": "svc",
                "owner": { "login": "team" },
                "url": "https://ghe.company.com/team/svc"
            },
            "reviewRequests": { "nodes": [] },
            "latestReviews": { "nodes": [] }
        });
        let pr = parse_graphql_search_pr(&node).unwrap();
        assert_eq!(pr.repo_id.host.as_deref(), Some("ghe.company.com"));
    }

    #[test]
    fn merge_entries_cross_repo_collision() {
        use crate::git::types::RepoId;
        let active = RepoId {
            host: None,
            owner: "active".into(),
            name: "repo".into(),
        };
        let other = RepoId {
            host: None,
            owner: "other".into(),
            name: "repo".into(),
        };

        let branches = vec![Branch {
            name: "feature/auth".to_string(),
            is_current: false,
            upstream: None,
            is_merged: false,
        }];
        let prs = vec![
            PullRequest {
                number: 1,
                title: "active PR".into(),
                author: "a".into(),
                state: "OPEN".into(),
                head_ref: "feature/auth".into(),
                updated_at: "2024".into(),
                is_draft: false,
                review_requests: vec![],
                latest_reviews: vec![],
                review_status: None,
                repo_id: active.clone(),
            },
            PullRequest {
                number: 99,
                title: "other PR".into(),
                author: "b".into(),
                state: "OPEN".into(),
                head_ref: "feature/auth".into(),
                updated_at: "2024".into(),
                is_draft: false,
                review_requests: vec![],
                latest_reviews: vec![],
                review_status: None,
                repo_id: other.clone(),
            },
        ];
        let entries = merge_entries(&active, &branches, &[], &prs, &Default::default());
        assert_eq!(entries.len(), 2);
        let active_entry = entries.iter().find(|e| e.repo_id == active).unwrap();
        assert!(active_entry.local_branch.is_some());
        assert_eq!(active_entry.pr_number(), Some(1));
        let other_entry = entries.iter().find(|e| e.repo_id == other).unwrap();
        assert!(other_entry.local_branch.is_none());
        assert_eq!(other_entry.pr_number(), Some(99));
    }

    #[test]
    fn merge_entries_injects_worktree_from_cross_repo_list() {
        use crate::git::types::{RepoId, Worktree};
        let active = RepoId {
            host: None,
            owner: "active".into(),
            name: "repo".into(),
        };
        let other = RepoId {
            host: None,
            owner: "other".into(),
            name: "repo".into(),
        };

        let prs = vec![PullRequest {
            number: 7,
            title: "x".into(),
            author: "u".into(),
            state: "OPEN".into(),
            head_ref: "feat/x".into(),
            updated_at: "2024".into(),
            is_draft: false,
            review_requests: vec![],
            latest_reviews: vec![],
            review_status: None,
            repo_id: other.clone(),
        }];
        let mut wt_lists = std::collections::HashMap::new();
        wt_lists.insert(
            other.clone(),
            vec![Worktree {
                path: "/tmp/clones/other/repo/feat/x".into(),
                head: "abc".into(),
                branch: Some("feat/x".into()),
                is_bare: false,
            }],
        );

        let entries = merge_entries(&active, &[], &[], &prs, &wt_lists);
        assert_eq!(entries.len(), 1);
        let entry = &entries[0];
        assert_eq!(entry.repo_id, other);
        assert!(entry.worktree.is_some());
        assert_eq!(entry.worktree_path(), Some("/tmp/clones/other/repo/feat/x"));
    }

    #[test]
    fn merge_entries_single_repo_unchanged_order() {
        use crate::git::types::RepoId;
        let active = RepoId {
            host: None,
            owner: "a".into(),
            name: "r".into(),
        };
        let branches = vec![
            Branch {
                name: "main".to_string(),
                is_current: true,
                upstream: None,
                is_merged: false,
            },
            Branch {
                name: "feature".to_string(),
                is_current: false,
                upstream: None,
                is_merged: false,
            },
        ];
        let entries = merge_entries(&active, &branches, &[], &[], &Default::default());
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].name, "main");
        assert_eq!(entries[1].name, "feature");
        assert!(entries.iter().all(|e| e.repo_id == active));
    }

    #[test]
    fn test_deserialize_pr_with_team_reviewer() {
        // Simulate gh pr list --json output with Team reviewer
        let json = r#"[{
            "number": 1,
            "title": "Test PR",
            "author": {"login": "alice"},
            "state": "OPEN",
            "headRefName": "test-branch",
            "updatedAt": "2024-01-15T00:00:00Z",
            "reviewRequests": [
                {"__typename": "User", "login": "bob"},
                {"__typename": "Team", "name": "backend", "slug": "backend"}
            ]
        }]"#;
        let prs: Vec<PullRequest> = serde_json::from_str(json).unwrap();
        assert_eq!(prs.len(), 1);
        assert_eq!(prs[0].review_requests.len(), 2);
        assert_eq!(prs[0].review_requests[0].login.as_deref(), Some("bob"));
        assert_eq!(prs[0].review_requests[1].login, None);
    }
}
