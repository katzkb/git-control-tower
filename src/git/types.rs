use std::fmt;

use serde::Deserialize;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct RepoId {
    /// `None` means github.com (canonical default for GitHub Enterprise omitted).
    pub host: Option<String>,
    pub owner: String,
    pub name: String,
}

impl fmt::Display for RepoId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.host {
            None => write!(f, "{}/{}", self.owner, self.name),
            Some(host) => write!(f, "{}/{}@{}", self.owner, self.name, host),
        }
    }
}

impl RepoId {
    /// `[HOST/]OWNER/REPO` form accepted by `gh --repo`.
    /// Use this whenever building a `gh` command — `gh pr <sub>` does not
    /// accept `--hostname`, so the host must be embedded in `--repo` instead.
    pub fn repo_arg(&self) -> String {
        match &self.host {
            Some(h) => format!("{h}/{}/{}", self.owner, self.name),
            None => format!("{}/{}", self.owner, self.name),
        }
    }
}

#[derive(Debug, Clone)]
pub struct RepoMeta {
    /// Resolved lazily on first selection. `None` after `local_path_resolved == true`
    /// means we tried but the clone path doesn't exist.
    pub local_path: Option<std::path::PathBuf>,
    pub local_path_resolved: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ReviewStatus {
    NeedsReview,
    Approved,
    ChangesRequested,
    Commented,
}

// label() methods are on the UI side (sidebar.rs, detail_pane.rs) to keep
// display logic separate from data types.

#[derive(Debug, Clone)]
pub struct Commit {
    pub hash: String,
    pub message: String,
    pub author: String,
    pub date: String,
    #[allow(dead_code)]
    pub graph: String,
}

#[derive(Debug, Clone)]
pub struct Branch {
    pub name: String,
    pub is_current: bool,
    pub upstream: Option<String>,
    pub is_merged: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PullRequest {
    pub number: u64,
    pub title: String,
    #[serde(deserialize_with = "deserialize_author")]
    pub author: String,
    pub state: String,
    #[serde(rename = "headRefName")]
    pub head_ref: String,
    #[serde(rename = "updatedAt")]
    pub updated_at: String,
    #[serde(rename = "reviewRequests", default)]
    pub review_requests: Vec<ReviewRequest>,
    #[serde(rename = "isDraft", default)]
    pub is_draft: bool,
    #[serde(rename = "latestReviews", default)]
    pub latest_reviews: Vec<LatestReview>,
    #[serde(skip)]
    pub review_status: Option<ReviewStatus>,
    #[serde(skip)]
    pub repo_id: RepoId,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ReviewRequest {
    #[serde(default)]
    pub login: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LatestReview {
    #[serde(deserialize_with = "deserialize_author")]
    pub author: String,
    pub state: String,
}

fn deserialize_author<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    struct Author {
        login: String,
    }
    let author = Author::deserialize(deserializer)?;
    Ok(author.login)
}

#[derive(Debug, Clone, Deserialize)]
pub struct PrDetail {
    pub number: u64,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub additions: u64,
    #[serde(default)]
    pub deletions: u64,
}

#[derive(Debug, Clone)]
pub struct Worktree {
    pub path: String,
    pub head: String,
    pub branch: Option<String>,
    pub is_bare: bool,
}

/// Unified entry keyed by branch name, aggregating local branch, worktree, and PR data.
#[derive(Debug, Clone)]
pub struct BranchEntry {
    pub name: String,
    pub repo_id: RepoId,
    pub local_branch: Option<Branch>,
    pub worktree: Option<Worktree>,
    pub pull_request: Option<PullRequest>,
    pub git_status: Option<GitStatus>,
}

impl BranchEntry {
    pub fn has_local(&self) -> bool {
        self.local_branch.is_some() || self.worktree.is_some()
    }

    pub fn is_current(&self) -> bool {
        self.local_branch.as_ref().is_some_and(|b| b.is_current)
    }

    pub fn is_merged(&self) -> bool {
        self.local_branch.as_ref().is_some_and(|b| b.is_merged)
    }

    pub fn pr_is_merged(&self) -> bool {
        self.pull_request
            .as_ref()
            .is_some_and(|pr| pr.state == "MERGED")
    }

    pub fn worktree_path(&self) -> Option<&str> {
        self.worktree.as_ref().map(|w| w.path.as_str())
    }

    pub fn pr_number(&self) -> Option<u64> {
        self.pull_request.as_ref().map(|pr| pr.number)
    }
}

#[derive(Debug, Clone, Default)]
pub struct GitStatus {
    pub untracked: Vec<String>,
    pub unstaged: Vec<String>,
    pub staged: Vec<String>,
    pub ahead: u32,
    pub behind: u32,
}

#[cfg(test)]
mod repo_id_tests {
    use super::*;

    #[test]
    fn display_github_dot_com() {
        let id = RepoId {
            host: None,
            owner: "katzkb".into(),
            name: "git-control-tower".into(),
        };
        assert_eq!(id.to_string(), "katzkb/git-control-tower");
    }

    #[test]
    fn display_ghe_host() {
        let id = RepoId {
            host: Some("ghe.company.com".into()),
            owner: "org".into(),
            name: "repo".into(),
        };
        assert_eq!(id.to_string(), "org/repo@ghe.company.com");
    }

    #[test]
    fn repo_arg_omits_host_for_github_com() {
        let id = RepoId {
            host: None,
            owner: "katzkb".into(),
            name: "git-control-tower".into(),
        };
        assert_eq!(id.repo_arg(), "katzkb/git-control-tower");
    }

    #[test]
    fn repo_arg_includes_host_for_ghe() {
        let id = RepoId {
            host: Some("ghe.company.com".into()),
            owner: "org".into(),
            name: "repo".into(),
        };
        assert_eq!(id.repo_arg(), "ghe.company.com/org/repo");
    }

    #[test]
    fn equality_and_hash() {
        use std::collections::HashSet;
        let a = RepoId {
            host: None,
            owner: "a".into(),
            name: "b".into(),
        };
        let b = RepoId {
            host: None,
            owner: "a".into(),
            name: "b".into(),
        };
        let c = RepoId {
            host: None,
            owner: "a".into(),
            name: "c".into(),
        };
        let mut set = HashSet::new();
        set.insert(a);
        assert!(set.contains(&b));
        assert!(!set.contains(&c));
    }
}
