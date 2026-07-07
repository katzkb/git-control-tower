use super::MainFilter;

/// A unit of work requested by the UI and executed by the main loop.
///
/// `handle_key` (and async-result handling in `run()`) pushes commands onto
/// `App::commands` via [`App::push_command`]; `run()` drains the queue once
/// per loop iteration and dispatches each variant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    // Read/fetch intents
    FetchPrs(MainFilter),
    FetchPrDetail(crate::git::types::RepoId, u64),
    /// Load `git status` for the worktree at this path.
    LoadGitStatus(String),
    LoadWorktreeList(crate::git::types::RepoId),
    ReloadBranches,
    ReloadCommits,
    // Mutating intents
    DeleteWorktree(String),
    ForceDeleteWorktree(String),
    /// `(repo_id, branch_name)` — carries `RepoId` so the main-loop lookup
    /// matches the correct repo when branch names collide.
    CreateWorktree(crate::git::types::RepoId, String),
    DeleteBranches(Vec<String>),
    CreateBranch {
        source: String,
        name: String,
    },
    OpenPrInBrowser(crate::git::types::RepoId, u64),
    CopyBranchName(String),
    /// Quit the TUI and emit this path for the shell-integration `cd`.
    CdAndQuit(String),
}
