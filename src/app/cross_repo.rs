/// Cross-repo context: active repo, clone root, lazily-populated per-repo
/// metadata and worktree lists, and the merged global config layers used to
/// resolve per-repo effective configs (issue #220).
#[derive(Default)]
pub struct CrossRepoState {
    /// Set at startup; `None` when startup couldn't infer a repo.
    pub active_repo: Option<crate::git::types::RepoId>,
    pub clone_root: Option<std::path::PathBuf>,
    /// Per-repo metadata (populated lazily as repos are selected).
    pub repos: std::collections::HashMap<crate::git::types::RepoId, crate::git::types::RepoMeta>,
    /// Worktree lists per repo (populated lazily as cross-repo PRs are selected).
    pub wt_lists_per_repo:
        std::collections::HashMap<crate::git::types::RepoId, Vec<crate::git::types::Worktree>>,
    /// Merged global config layers (home-dir files only), used to resolve a
    /// per-repo effective config for cross-repo worktree operations.
    pub global_layers: toml::Table,
}

impl CrossRepoState {
    /// Hosts the user can reasonably be expected to have PRs on, derived from
    /// the origin remotes of every repo we have metadata for. Empty repo map
    /// falls back to `[None]` (default host = github.com) so cross-repo
    /// aggregation behaves identically to the single-host case before any
    /// repo metadata is collected. The output is unique-by-host and sorted
    /// (None first) for deterministic ordering.
    pub fn known_hosts(&self) -> Vec<Option<String>> {
        use std::collections::BTreeSet;
        let set: BTreeSet<Option<String>> = self.repos.keys().map(|id| id.host.clone()).collect();
        if set.is_empty() {
            vec![None]
        } else {
            set.into_iter().collect()
        }
    }

    /// Effective config for a specific repo root: the global layers overlaid
    /// with `<repo_root>/.gct.toml`. Used for cross-repo worktree operations so
    /// the target repo's own `.gct.toml` applies (not the launching repo's).
    pub fn resolve_repo_config(&self, repo_root: &std::path::Path) -> crate::config::Config {
        crate::config::resolve_config(&self.global_layers, Some(repo_root))
    }

    /// Resolve a repo's local clone path under `clone_root`. Idempotent: only
    /// hits the filesystem once per repo. Sets `local_path_resolved = true`
    /// regardless of outcome to prevent re-tries.
    pub fn resolve_local_path(&mut self, id: &crate::git::types::RepoId) {
        // Snapshot clone_root first (no borrow on self.repos held).
        let root = self.clone_root.clone();
        let Some(meta) = self.repos.get_mut(id) else {
            return;
        };
        if meta.local_path_resolved {
            return;
        }
        meta.local_path_resolved = true;
        let Some(root) = root else {
            return;
        };
        let host = id.host.as_deref().unwrap_or("github.com");
        let candidate = root.join(host).join(&id.owner).join(&id.name);
        if candidate.is_dir() {
            meta.local_path = Some(candidate);
        }
    }
}
