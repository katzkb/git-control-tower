use std::collections::HashSet;

use crate::git::types::{Branch, Commit, Worktree};

/// Parse output of `git log --format="%h%x00%s%x00%an%x00%ad" --date=short`
/// with optional `--graph` prefix per line.
pub fn parse_log(output: &str) -> Vec<Commit> {
    output
        .lines()
        .filter_map(|line| {
            // Graph characters are everything before the first field separator
            let (graph, fields) = match line.find('\x00') {
                Some(pos) => (line[..pos].to_string(), &line[pos..]),
                None => return None,
            };

            // Extract the hash from graph prefix (last non-whitespace, non-graph-char token)
            let hash_start = graph
                .rfind(|c: char| "*|\\/_ ".contains(c))
                .map_or(0, |i| i + 1);
            let hash = graph[hash_start..].trim().to_string();
            let graph_prefix = graph[..hash_start].to_string();

            let parts: Vec<&str> = fields.split('\x00').collect();
            if parts.len() < 4 {
                return None;
            }

            Some(Commit {
                hash,
                message: parts[1].to_string(),
                author: parts[2].to_string(),
                date: parts[3].to_string(),
                graph: graph_prefix,
            })
        })
        .collect()
}

/// Parse output of `git branch -vv --no-abbrev`
/// `first_parent_output` is the output of `git rev-list --first-parent <default>`:
/// the full hashes on the default branch's first-parent chain, newest first.
pub fn parse_branches(output: &str, merged_output: &str, first_parent_output: &str) -> Vec<Branch> {
    // `git branch --merged` prefixes the current branch with `* ` and branches
    // checked out in linked worktrees with `+ `; strip either so the name matches.
    let merged_names: Vec<&str> = merged_output
        .lines()
        .filter_map(|l| {
            let t = l.trim();
            let name = t
                .strip_prefix("* ")
                .or_else(|| t.strip_prefix("+ "))
                .unwrap_or(t);
            (!name.is_empty()).then_some(name)
        })
        .collect();
    let first_parent: HashSet<&str> = first_parent_output
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();

    output
        .lines()
        .filter_map(|line| {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                return None;
            }

            let is_current = trimmed.starts_with('*');
            let rest = if is_current || trimmed.starts_with('+') {
                trimmed[1..].trim()
            } else {
                trimmed
            };

            // Branch name is the first token
            let name = rest.split_whitespace().next()?.to_string();

            // Extract upstream from [origin/xxx] or [origin/xxx: ahead N]
            let upstream = rest.find('[').and_then(|start| {
                let after = &rest[start + 1..];
                let close = after.find(']')?;
                let inner = &after[..close];
                // Strip trailing status like ": ahead 1, behind 2"
                let name = inner.split(':').next().unwrap_or(inner).trim();
                Some(name.to_string())
            });

            // A branch is merged if it's in --merged output AND its commit is not
            // on the default branch's first-parent chain. A tip on that chain is
            // just a point the default branch passed through (e.g., a branch
            // created and never committed to), not something merged into it.
            // Both `branch -vv --no-abbrev` and `rev-list` print full hashes.
            let commit_hash = rest.split_whitespace().nth(1).unwrap_or_default();
            let is_merged = merged_names.contains(&name.as_str())
                && !commit_hash.is_empty()
                && !first_parent.is_empty()
                && !first_parent.contains(commit_hash);

            Some(Branch {
                name,
                is_current,
                upstream,
                is_merged,
            })
        })
        .collect()
}

/// Parse output of `git worktree list --porcelain`
pub fn parse_worktrees(output: &str) -> Vec<Worktree> {
    let mut worktrees = Vec::new();
    let mut path = String::new();
    let mut head = String::new();
    let mut branch = None;
    let mut is_bare = false;

    for line in output.lines() {
        if let Some(p) = line.strip_prefix("worktree ") {
            path = p.to_string();
        } else if let Some(h) = line.strip_prefix("HEAD ") {
            head = h.to_string();
        } else if let Some(b) = line.strip_prefix("branch ") {
            branch = Some(b.trim_start_matches("refs/heads/").to_string());
        } else if line == "bare" {
            is_bare = true;
        } else if line.is_empty() && !path.is_empty() {
            worktrees.push(Worktree {
                path: path.clone(),
                head: head.clone(),
                branch: branch.take(),
                is_bare,
            });
            path.clear();
            head.clear();
            is_bare = false;
        }
    }

    // Handle last entry without trailing newline
    if !path.is_empty() {
        worktrees.push(Worktree {
            path,
            head,
            branch,
            is_bare,
        });
    }

    worktrees
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_log() {
        let output = "* abc1234\x00fix bug\x00Alice\x002024-01-15\n\
                       * def5678\x00add feature\x00Bob\x002024-01-14\n";
        let commits = parse_log(output);
        assert_eq!(commits.len(), 2);
        assert_eq!(commits[0].hash, "abc1234");
        assert_eq!(commits[0].message, "fix bug");
        assert_eq!(commits[0].author, "Alice");
        assert_eq!(commits[0].date, "2024-01-15");
        assert_eq!(commits[1].hash, "def5678");
        assert_eq!(commits[1].message, "add feature");
    }

    // Full hashes, as printed by `branch -vv --no-abbrev` and `rev-list`.
    const H_MAIN: &str = "1111111111111111111111111111111111111111";
    const H_OLDER_MAIN: &str = "2222222222222222222222222222222222222222";
    const H_FEATURE: &str = "3333333333333333333333333333333333333333";
    const H_MERGED: &str = "4444444444444444444444444444444444444444";

    #[test]
    fn test_parse_branches() {
        let output = format!(
            "* main       {H_MAIN} [origin/main] latest commit\n  \
               feature-a  {H_FEATURE} [origin/feature-a: ahead 1] wip\n  \
               old-branch {H_MERGED} some old work\n"
        );
        let merged = "  old-branch\n";
        let first_parent = format!("{H_MAIN}\n");
        let branches = parse_branches(&output, merged, &first_parent);

        assert_eq!(branches.len(), 3);

        assert_eq!(branches[0].name, "main");
        assert!(branches[0].is_current);
        assert_eq!(branches[0].upstream.as_deref(), Some("origin/main"));
        assert!(!branches[0].is_merged);

        assert_eq!(branches[1].name, "feature-a");
        assert!(!branches[1].is_current);
        assert_eq!(branches[1].upstream.as_deref(), Some("origin/feature-a"));

        // old-branch is not on main's first-parent chain → merged
        assert_eq!(branches[2].name, "old-branch");
        assert!(branches[2].is_merged);
        assert!(branches[2].upstream.is_none());
    }

    #[test]
    fn test_merged_branch_checked_out_in_worktree() {
        // A merged branch checked out in a linked worktree is prefixed with `+`
        // in `git branch --merged` output; it must still be detected as merged.
        let output = format!(
            "* main       {H_MAIN} [origin/main] latest commit\n\
             + wt-branch  {H_MERGED} (/path/wt) merged work\n"
        );
        let merged = "* main\n+ wt-branch\n";
        let first_parent = format!("{H_MAIN}\n");
        let branches = parse_branches(&output, merged, &first_parent);

        assert_eq!(branches[1].name, "wt-branch");
        assert!(!branches[1].is_current);
        assert!(branches[1].is_merged);
    }

    #[test]
    fn test_branch_at_default_not_marked_merged() {
        // The default branch tip heads its own first-parent chain.
        let output = format!(
            "* main       {H_MAIN} [origin/main] latest commit\n  \
               new-branch {H_MAIN} starting work\n"
        );
        let merged = "* main\n  new-branch\n";
        let first_parent = format!("{H_MAIN}\n");
        let branches = parse_branches(&output, merged, &first_parent);

        assert_eq!(branches.len(), 2);
        assert!(!branches[0].is_merged); // main (same commit as default branch)
        assert!(!branches[1].is_merged); // new-branch (same commit as default branch)
    }

    #[test]
    fn test_branch_left_behind_on_first_parent_chain_not_merged() {
        // `idle` was created from main and never committed to; main has since
        // advanced. It is an ancestor of main (listed by --merged) but sits on
        // main's first-parent chain, so it was never merged. `feature` was
        // joined through a merge commit (off the chain) → truly merged.
        let output = format!(
            "* main    {H_MAIN} [origin/main] latest commit\n  \
               idle    {H_OLDER_MAIN} older main commit\n  \
               feature {H_MERGED} feature work\n"
        );
        let merged = "* main\n  idle\n  feature\n";
        let first_parent = format!("{H_MAIN}\n{H_OLDER_MAIN}\n");
        let branches = parse_branches(&output, merged, &first_parent);

        assert!(!branches[0].is_merged);
        assert!(!branches[1].is_merged);
        assert!(branches[2].is_merged);
    }

    #[test]
    fn test_no_first_parent_output_marks_nothing_merged() {
        // `git rev-list` failed: without the chain, stay conservative.
        let output = format!(
            "* main       {H_MAIN} latest commit\n  \
               old-branch {H_MERGED} some old work\n"
        );
        let merged = "* main\n  old-branch\n";
        let branches = parse_branches(&output, merged, "");

        assert!(!branches[1].is_merged);
    }

    #[test]
    fn test_parse_worktrees() {
        let output = "worktree /home/user/repo\n\
                       HEAD abc1234567890\n\
                       branch refs/heads/main\n\
                       \n\
                       worktree /home/user/repo-feature\n\
                       HEAD def5678901234\n\
                       branch refs/heads/feature-x\n\
                       \n";
        let worktrees = parse_worktrees(output);

        assert_eq!(worktrees.len(), 2);
        assert_eq!(worktrees[0].path, "/home/user/repo");
        assert_eq!(worktrees[0].head, "abc1234567890");
        assert_eq!(worktrees[0].branch.as_deref(), Some("main"));
        assert!(!worktrees[0].is_bare);

        assert_eq!(worktrees[1].path, "/home/user/repo-feature");
        assert_eq!(worktrees[1].branch.as_deref(), Some("feature-x"));
    }

    #[test]
    fn test_parse_worktrees_bare() {
        let output = "worktree /home/user/repo.git\n\
                       HEAD abc1234567890\n\
                       bare\n\
                       \n";
        let worktrees = parse_worktrees(output);

        assert_eq!(worktrees.len(), 1);
        assert!(worktrees[0].is_bare);
        assert!(worktrees[0].branch.is_none());
    }
}
