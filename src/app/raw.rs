use crate::git::types::{Branch, Commit, Worktree};

/// Raw data fetched from `git`/`gh`: inputs to `rebuild_entries` and the Log
/// view, plus the gh identity used for review-status computation (issue #220).
#[derive(Default)]
pub struct RawData {
    pub branches: Vec<Branch>,
    pub worktrees: Vec<Worktree>,
    pub commits: Vec<Commit>,
    pub gh_user: String,
    pub gh_user_load_failed: bool,
}
