use std::collections::HashMap;

use crate::git::command::run_git;
use crate::git::types::{Branch, BranchEntry, GitStatus, PullRequest, Worktree};

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
}
