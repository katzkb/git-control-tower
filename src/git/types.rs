use serde::Deserialize;

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
    #[allow(dead_code)]
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
    #[allow(dead_code)]
    pub head_ref: String,
    #[serde(rename = "updatedAt")]
    #[allow(dead_code)]
    pub updated_at: String,
    #[serde(rename = "reviewRequests", default)]
    pub review_requests: Vec<ReviewRequest>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ReviewRequest {
    pub login: String,
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
    #[allow(dead_code)]
    pub title: String,
    #[serde(deserialize_with = "deserialize_author")]
    #[allow(dead_code)]
    pub author: String,
    #[allow(dead_code)]
    pub state: String,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub additions: u64,
    #[serde(default)]
    pub deletions: u64,
    #[serde(rename = "headRefName")]
    #[allow(dead_code)]
    pub head_ref: String,
}

#[derive(Debug, Clone)]
pub struct Worktree {
    pub path: String,
    #[allow(dead_code)]
    pub head: String,
    pub branch: Option<String>,
    #[allow(dead_code)]
    pub is_bare: bool,
}

/// Unified entry keyed by branch name, aggregating local branch, worktree, and PR data.
#[derive(Debug, Clone)]
pub struct BranchEntry {
    pub name: String,
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
            || self
                .pull_request
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
