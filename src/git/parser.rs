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

/// Parse output of `git branch -vv`
pub fn parse_branches(output: &str, merged_output: &str) -> Vec<Branch> {
    let merged_names: Vec<&str> = merged_output
        .lines()
        .map(|l| l.trim().trim_start_matches("* "))
        .collect();

    output
        .lines()
        .filter_map(|line| {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                return None;
            }

            let is_current = trimmed.starts_with('*');
            let rest = if is_current {
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

            let is_merged = merged_names.contains(&name.as_str());

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
        assert_eq!(commits[0].graph, "* ");
        assert_eq!(commits[1].hash, "def5678");
        assert_eq!(commits[1].message, "add feature");
        assert_eq!(commits[1].graph, "* ");
    }

    #[test]
    fn test_parse_log_with_graph_branches() {
        let output = "* abc1234\x00merge branch\x00Alice\x002024-01-15\n\
                       |\\ \n\
                       | * def5678\x00feature work\x00Bob\x002024-01-14\n\
                       |/ \n\
                       * ghi9012\x00initial commit\x00Alice\x002024-01-13\n";
        let commits = parse_log(output);
        // Graph-only lines (|\ and |/) are skipped, only commit lines are parsed
        assert_eq!(commits.len(), 3);
        assert_eq!(commits[0].graph, "* ");
        assert_eq!(commits[0].hash, "abc1234");
        assert_eq!(commits[1].graph, "| * ");
        assert_eq!(commits[1].hash, "def5678");
        assert_eq!(commits[2].graph, "* ");
        assert_eq!(commits[2].hash, "ghi9012");
    }

    #[test]
    fn test_parse_branches() {
        let output = "* main       abc1234 [origin/main] latest commit\n\
                         feature-a  def5678 [origin/feature-a: ahead 1] wip\n\
                         old-branch ghi9012 some old work\n";
        let merged = "  old-branch\n";
        let branches = parse_branches(output, merged);

        assert_eq!(branches.len(), 3);

        assert_eq!(branches[0].name, "main");
        assert!(branches[0].is_current);
        assert_eq!(branches[0].upstream.as_deref(), Some("origin/main"));
        assert!(!branches[0].is_merged);

        assert_eq!(branches[1].name, "feature-a");
        assert!(!branches[1].is_current);
        assert_eq!(branches[1].upstream.as_deref(), Some("origin/feature-a"));

        assert_eq!(branches[2].name, "old-branch");
        assert!(branches[2].is_merged);
        assert!(branches[2].upstream.is_none());
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
