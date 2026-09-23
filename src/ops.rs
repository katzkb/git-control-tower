//! Headless repository operations shared by the CLI subcommands (`wt`, `ls`,
//! `prune`) and the MCP server (`gct mcp`). Nothing in this module writes to
//! stdout or stderr: for MCP, stdout is the JSON-RPC channel, so all results
//! and errors are returned as data and rendered by the caller.

use crate::config;
use crate::data::{escape_graphql_string, graphql_alias};
use crate::git::command::{run_gh, run_git};
use crate::git::parser::{parse_branches, parse_worktrees};
use crate::git::types::{Branch, Worktree};

/// Outcome of `ensure_worktree`.
pub struct WorktreeOutcome {
    /// Path of the worktree (existing or newly created).
    pub path: String,
    /// `false` means an existing worktree was reused (idempotent hit).
    pub created: bool,
    /// The branch did not exist locally and was fetched from origin first.
    pub fetched_from_origin: bool,
    /// Non-fatal post-create hook failures (the worktree itself was created).
    pub hook_errors: Vec<String>,
}

/// Failure modes of `ensure_worktree`. Display strings match the text
/// `gct wt` has always printed after its "Error: " prefix, so the CLI
/// output stays byte-identical.
pub enum WtError {
    NotAGitRepo,
    RepoRoot(String),
    Fetch { branch: String, msg: String },
    WorktreeAdd(String),
}

impl std::fmt::Display for WtError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotAGitRepo => write!(f, "not a git repository."),
            Self::RepoRoot(e) => write!(f, "failed to resolve repository root: {e}"),
            Self::Fetch { branch, msg } => {
                write!(f, "failed to fetch '{branch}' from origin: {msg}")
            }
            Self::WorktreeAdd(e) => write!(f, "failed to create worktree: {e}"),
        }
    }
}

/// Find the worktree checked out for `branch` and return its path.
/// Pure helper over an already-parsed worktree list so it can be unit-tested.
pub fn worktree_path_for_branch(worktrees: &[Worktree], branch: &str) -> Option<String> {
    worktrees
        .iter()
        .find(|wt| wt.branch.as_deref() == Some(branch))
        .map(|wt| wt.path.clone())
}

/// Resolve the active repo's name for the `{repo}` token in `worktree_path_for`.
/// Prefers the origin remote's repo name, falling back to the toplevel dir name.
pub async fn active_repo_name(repo_root: &std::path::Path) -> String {
    if let Ok(url) = run_git(&["remote", "get-url", "origin"]).await
        && let Some(info) = crate::extract_repo_info(url.trim())
    {
        return info.name;
    }
    repo_root
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("repo")
        .to_string()
}

/// Reuse the worktree for `branch` or create one, applying the configured
/// path layout and post-create hooks. Idempotent: an existing worktree is
/// returned with `created: false`. Checks out a local branch directly;
/// otherwise fetches from origin first and lets `git worktree add` create a
/// tracking branch.
pub async fn ensure_worktree(branch: &str) -> Result<WorktreeOutcome, WtError> {
    if run_git(&["rev-parse", "--git-dir"]).await.is_err() {
        return Err(WtError::NotAGitRepo);
    }

    // Idempotent: if a worktree already exists for the branch, just return it.
    if let Ok(output) = run_git(&["worktree", "list", "--porcelain"]).await
        && let Some(path) = worktree_path_for_branch(&parse_worktrees(&output), branch)
    {
        return Ok(WorktreeOutcome {
            path,
            created: false,
            fetched_from_origin: false,
            hook_errors: Vec::new(),
        });
    }

    let repo_root = match run_git(&["rev-parse", "--show-toplevel"]).await {
        Ok(s) => std::path::PathBuf::from(s.trim()),
        Err(e) => return Err(WtError::RepoRoot(e.to_string())),
    };

    let cfg = config::load_config();
    let repo_name = active_repo_name(&repo_root).await;
    let wt_path = cfg.worktree_path_for(&repo_root, &repo_name, branch);
    if let Some(parent) = std::path::Path::new(&wt_path).parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    // Check out a local branch directly, otherwise fetch from origin first and
    // let `git worktree add` create a tracking branch.
    let has_local = run_git(&[
        "rev-parse",
        "--verify",
        "--quiet",
        &format!("refs/heads/{branch}"),
    ])
    .await
    .is_ok();
    if !has_local && let Err(e) = run_git(&["fetch", "origin", branch]).await {
        return Err(WtError::Fetch {
            branch: branch.to_string(),
            msg: e.to_string(),
        });
    }

    if let Err(e) = run_git(&["worktree", "add", &wt_path, branch]).await {
        return Err(WtError::WorktreeAdd(e.to_string()));
    }

    // Post-create hooks are non-fatal.
    let hook_errors = config::run_post_create(
        &cfg.worktree.post_create,
        &repo_root,
        std::path::Path::new(&wt_path),
    );

    Ok(WorktreeOutcome {
        path: wt_path,
        created: true,
        fetched_from_origin: !has_local,
        hook_errors,
    })
}

/// List the active repo's worktrees.
pub async fn list_worktrees() -> anyhow::Result<Vec<Worktree>> {
    let output = run_git(&["worktree", "list", "--porcelain"]).await?;
    Ok(parse_worktrees(&output))
}

/// Load the active repo's branches (no App state). Mirrors the git calls in
/// the TUI's `load_branches` but returns a plain `Vec<Branch>` or an error.
pub async fn list_branches() -> anyhow::Result<Vec<Branch>> {
    let branch_output = run_git(&["branch", "-vv"]).await?;
    let default_branch = crate::detect_default_branch().await;
    let merged_output = run_git(&["branch", "--merged", &default_branch])
        .await
        .unwrap_or_default();
    let base_hash = run_git(&["rev-parse", &default_branch])
        .await
        .unwrap_or_default();
    Ok(parse_branches(
        &branch_output,
        &merged_output,
        base_hash.trim(),
    ))
}

/// The merged PR found for a branch whose commits are not ancestors of the
/// default branch (squash or rebase merge).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MergedPr {
    pub number: u64,
    /// Commit the PR head pointed at when it was merged.
    pub head_oid: String,
}

/// Look up merged PRs for `branch_names` via `gh` (GraphQL, exact match on
/// `headRefName`). Returns branch name → most recently updated merged PR.
/// `Err` carries a one-line reason (no GitHub origin, gh missing or
/// unauthenticated, API failure) so the caller can fall back to git-only
/// detection.
pub async fn find_merged_prs(
    branch_names: &[String],
) -> Result<std::collections::HashMap<String, MergedPr>, String> {
    let mut found = std::collections::HashMap::new();
    if branch_names.is_empty() {
        return Ok(found);
    }
    let url = run_git(&["remote", "get-url", "origin"])
        .await
        .map_err(|_| "no origin remote".to_string())?;
    let repo = crate::extract_repo_info(url.trim())
        .ok_or_else(|| "origin is not a GitHub remote".to_string())?;

    for chunk in branch_names.chunks(200) {
        let query = merged_prs_query(&repo.owner, &repo.name, chunk);
        let query_arg = format!("query={query}");
        let mut args = vec!["api", "graphql", "-f", &query_arg];
        if let Some(h) = repo.host.as_deref() {
            args.push("--hostname");
            args.push(h);
        }
        let output = run_gh(&args)
            .await
            .map_err(|e| first_line(&e.to_string()))?;
        let json: serde_json::Value =
            serde_json::from_str(&output).map_err(|e| format!("unexpected gh output: {e}"))?;
        found.extend(parse_merged_prs(&json, chunk));
    }
    Ok(found)
}

/// Build the GraphQL query asking for each branch's latest merged PR.
fn merged_prs_query(owner: &str, name: &str, branches: &[String]) -> String {
    let mut aliases = String::new();
    for (i, branch) in branches.iter().enumerate() {
        aliases.push_str(&format!(
            r#"{}: pullRequests(first: 1, headRefName: "{}", states: [MERGED], orderBy: {{field: UPDATED_AT, direction: DESC}}) {{ nodes {{ number headRefOid }} }}
"#,
            graphql_alias(i),
            escape_graphql_string(branch)
        ));
    }
    format!(
        r#"{{ repository(owner: "{}", name: "{}") {{ {aliases} }} }}"#,
        escape_graphql_string(owner),
        escape_graphql_string(name)
    )
}

/// Map a `merged_prs_query` response back to branch names (each alias is the
/// branch's index within `branches`). Pure, so it can be unit-tested.
fn parse_merged_prs(
    json: &serde_json::Value,
    branches: &[String],
) -> std::collections::HashMap<String, MergedPr> {
    let repo = &json["data"]["repository"];
    branches
        .iter()
        .enumerate()
        .filter_map(|(i, branch)| {
            let node = &repo[graphql_alias(i)]["nodes"][0];
            let number = node["number"].as_u64()?;
            let head_oid = node["headRefOid"].as_str()?.to_string();
            Some((branch.clone(), MergedPr { number, head_oid }))
        })
        .collect()
}

/// Map each local branch to its tip commit.
pub async fn local_branch_tips() -> anyhow::Result<std::collections::HashMap<String, String>> {
    let output = run_git(&[
        "for-each-ref",
        "--format=%(refname) %(objectname)",
        "refs/heads",
    ])
    .await?;
    Ok(output
        .lines()
        .filter_map(|line| {
            let (refname, oid) = line.rsplit_once(' ')?;
            let name = refname.strip_prefix("refs/heads/")?;
            Some((name.to_string(), oid.to_string()))
        })
        .collect())
}

/// First line of a (possibly multi-line) error string.
fn first_line(s: &str) -> String {
    s.lines().next().unwrap_or(s).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wt(path: &str, branch: Option<&str>, is_bare: bool) -> Worktree {
        Worktree {
            path: path.to_string(),
            head: "abc123".to_string(),
            branch: branch.map(|s| s.to_string()),
            is_bare,
        }
    }

    #[test]
    fn worktree_path_for_branch_found() {
        let wts = vec![
            wt("/repo", Some("main"), false),
            wt("/repo-feature", Some("feature/x"), false),
        ];
        assert_eq!(
            worktree_path_for_branch(&wts, "feature/x").as_deref(),
            Some("/repo-feature")
        );
    }

    #[test]
    fn worktree_path_for_branch_not_found() {
        let wts = vec![wt("/repo", Some("main"), false)];
        assert!(worktree_path_for_branch(&wts, "feature/x").is_none());
    }

    #[test]
    fn worktree_path_for_branch_ignores_detached_and_bare() {
        // A detached/bare worktree has no branch and must never match.
        let wts = vec![
            wt("/repo-bare", None, true),
            wt("/repo-detached", None, false),
        ];
        assert!(worktree_path_for_branch(&wts, "feature/x").is_none());
    }

    #[test]
    fn parse_merged_prs_maps_aliases_to_branches() {
        let branches = vec![
            "feat/a".to_string(),
            "feat/b".to_string(),
            "feat/c".to_string(),
        ];
        let json = serde_json::json!({"data": {"repository": {
            "b0": {"nodes": [{"number": 12, "headRefOid": "aaa"}]},
            "b1": {"nodes": []},
            "b2": {"nodes": [{"number": 7, "headRefOid": "ccc"}]}
        }}});
        let got = parse_merged_prs(&json, &branches);
        assert_eq!(got.len(), 2);
        assert_eq!(
            got["feat/a"],
            MergedPr {
                number: 12,
                head_oid: "aaa".into()
            }
        );
        assert_eq!(got["feat/c"].number, 7);
        assert!(!got.contains_key("feat/b"));
    }

    #[test]
    fn merged_prs_query_escapes_and_aliases() {
        let q = merged_prs_query("o", "r", &["a\"b".to_string(), "c".to_string()]);
        assert!(q.contains(r#"repository(owner: "o", name: "r")"#));
        assert!(q.contains(r#"b0: pullRequests(first: 1, headRefName: "a\"b", states: [MERGED]"#));
        assert!(q.contains(r#"b1: pullRequests(first: 1, headRefName: "c""#));
        assert!(q.contains("headRefOid"));
    }

    // Lock the exact error text the CLI prints (after its "Error: " prefix)
    // so the run_wt refactor stays byte-compatible.
    #[test]
    fn wt_error_display_matches_cli_wording() {
        assert_eq!(WtError::NotAGitRepo.to_string(), "not a git repository.");
        assert_eq!(
            WtError::RepoRoot("boom".into()).to_string(),
            "failed to resolve repository root: boom"
        );
        assert_eq!(
            WtError::Fetch {
                branch: "feature/x".into(),
                msg: "boom".into()
            }
            .to_string(),
            "failed to fetch 'feature/x' from origin: boom"
        );
        assert_eq!(
            WtError::WorktreeAdd("boom".into()).to_string(),
            "failed to create worktree: boom"
        );
    }
}
