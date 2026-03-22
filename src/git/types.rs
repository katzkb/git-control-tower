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
    pub title: String,
    #[serde(deserialize_with = "deserialize_author")]
    pub author: String,
    pub state: String,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub additions: u64,
    #[serde(default)]
    pub deletions: u64,
    #[serde(rename = "headRefName")]
    pub head_ref: String,
}

#[derive(Debug, Clone)]
pub struct Worktree {
    pub path: String,
    pub head: String,
    pub branch: Option<String>,
    pub is_bare: bool,
}
