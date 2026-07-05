//! `gct mcp` — a Model Context Protocol server over stdio.
//!
//! Exposes gct's convention-aware worktree operations as MCP tools so AI
//! agents create worktrees through gct (config-driven path layout +
//! post-create hooks) instead of running `git worktree add` directly.
//!
//! stdout is the JSON-RPC channel: nothing in this module (or in `ops`,
//! which it delegates to) may print to stdout. Diagnostics go to stderr or
//! the GCT_DEBUG file log.

use rmcp::{
    ErrorData as McpError, ServerHandler, ServiceExt,
    handler::server::{
        router::tool::ToolRouter,
        wrapper::{Json, Parameters},
    },
    tool, tool_handler, tool_router,
    transport::stdio,
};

use crate::ops;

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct CreateWorktreeParams {
    /// Branch name to create or reuse a worktree for (e.g. "feature/login").
    /// If the branch does not exist locally it is fetched from origin first.
    pub branch: String,
}

#[derive(serde::Serialize, schemars::JsonSchema)]
pub struct CreateWorktreeResult {
    /// The branch the worktree is checked out on.
    pub branch: String,
    /// Path of the worktree (existing or newly created).
    pub path: String,
    /// False when an existing worktree was reused (idempotent hit).
    pub created: bool,
    /// The branch was fetched from origin because it did not exist locally.
    pub fetched_from_origin: bool,
    /// Non-fatal post-create hook failures. The worktree itself was created.
    pub hook_errors: Vec<String>,
}

#[derive(serde::Serialize, schemars::JsonSchema)]
pub struct WorktreeInfo {
    /// Absolute path of the worktree.
    pub path: String,
    /// Commit hash of the worktree's HEAD.
    pub head: String,
    /// Checked-out branch; null when detached or bare.
    pub branch: Option<String>,
    /// True for the bare repository entry.
    pub is_bare: bool,
}

#[derive(serde::Serialize, schemars::JsonSchema)]
pub struct ListWorktreesResult {
    pub worktrees: Vec<WorktreeInfo>,
}

#[derive(serde::Serialize, schemars::JsonSchema)]
pub struct BranchInfo {
    /// Local branch name.
    pub name: String,
    /// True for the currently checked-out branch.
    pub is_current: bool,
    /// Upstream tracking ref (e.g. "origin/main"); null when none.
    pub upstream: Option<String>,
    /// True when the branch is merged into the default branch.
    pub is_merged: bool,
}

#[derive(serde::Serialize, schemars::JsonSchema)]
pub struct ListBranchesResult {
    pub branches: Vec<BranchInfo>,
}

#[derive(Clone)]
pub struct GctMcpServer {
    tool_router: ToolRouter<Self>,
}

impl Default for GctMcpServer {
    fn default() -> Self {
        Self::new()
    }
}

fn tool_error(e: impl std::fmt::Display) -> McpError {
    McpError::internal_error(e.to_string(), None)
}

#[tool_router]
impl GctMcpServer {
    pub fn new() -> Self {
        Self {
            tool_router: Self::tool_router(),
        }
    }

    #[tool(
        description = "Create a git worktree for a branch in this repository, or return the existing one. THIS is the correct way to create worktrees here — do not run `git worktree add` directly. This tool resolves the worktree path from the project's gct configuration (`worktree.dir`, with `{repo}` placeholder support), fetches the branch from origin when it does not exist locally, and runs the project's post-create hooks (e.g. copying or symlinking .env files, running setup commands) so the new worktree is immediately usable. Idempotent: if a worktree for the branch already exists, its path is returned with created=false instead of failing. Returns the worktree path, whether it was newly created, whether the branch was fetched from origin, and any non-fatal hook errors."
    )]
    async fn create_worktree(
        &self,
        Parameters(params): Parameters<CreateWorktreeParams>,
    ) -> Result<Json<CreateWorktreeResult>, McpError> {
        let outcome = ops::ensure_worktree(&params.branch)
            .await
            .map_err(tool_error)?;
        Ok(Json(CreateWorktreeResult {
            branch: params.branch,
            path: outcome.path,
            created: outcome.created,
            fetched_from_origin: outcome.fetched_from_origin,
            hook_errors: outcome.hook_errors,
        }))
    }

    #[tool(
        description = "List all git worktrees of the current repository as structured JSON (path, checked-out branch, HEAD commit, bare flag). Prefer this over parsing `git worktree list` output."
    )]
    async fn list_worktrees(&self) -> Result<Json<ListWorktreesResult>, McpError> {
        let worktrees = ops::list_worktrees().await.map_err(tool_error)?;
        Ok(Json(ListWorktreesResult {
            worktrees: worktrees
                .into_iter()
                .map(|wt| WorktreeInfo {
                    path: wt.path,
                    head: wt.head,
                    branch: wt.branch,
                    is_bare: wt.is_bare,
                })
                .collect(),
        }))
    }

    #[tool(
        description = "List local git branches of the current repository as structured JSON (name, is_current, upstream, is_merged into the default branch). Local-only; requires no network or GitHub access."
    )]
    async fn list_branches(&self) -> Result<Json<ListBranchesResult>, McpError> {
        let branches = ops::list_branches().await.map_err(tool_error)?;
        Ok(Json(ListBranchesResult {
            branches: branches
                .into_iter()
                .map(|b| BranchInfo {
                    name: b.name,
                    is_current: b.is_current,
                    upstream: b.upstream,
                    is_merged: b.is_merged,
                })
                .collect(),
        }))
    }
}

#[tool_handler(
    router = self.tool_router,
    name = "gct",
    instructions = "gct exposes convention-aware git worktree operations for the repository in the current working directory. Always use create_worktree instead of running `git worktree add` yourself: it applies the project's configured worktree path layout and runs its post-create setup hooks (such as copying .env files), so the branch↔directory mapping stays consistent and the worktree is immediately usable."
)]
impl ServerHandler for GctMcpServer {}

/// Entry point called from main's subcommand dispatch. Returns a process
/// exit code; all failure detail goes to stderr, never stdout.
pub async fn run_mcp_server() -> i32 {
    match serve().await {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("Error: MCP server failed: {e}");
            1
        }
    }
}

async fn serve() -> anyhow::Result<()> {
    let service = GctMcpServer::new().serve(stdio()).await?;
    service.waiting().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_worktree_result_serializes_expected_fields() {
        let v = serde_json::to_value(CreateWorktreeResult {
            branch: "feature/x".into(),
            path: "/repos/wt/feature/x".into(),
            created: true,
            fetched_from_origin: false,
            hook_errors: vec![],
        })
        .unwrap();
        assert_eq!(v["branch"], "feature/x");
        assert_eq!(v["path"], "/repos/wt/feature/x");
        assert_eq!(v["created"], true);
        assert_eq!(v["fetched_from_origin"], false);
        assert!(v["hook_errors"].as_array().unwrap().is_empty());
    }

    #[test]
    fn list_results_wrap_arrays_in_objects() {
        // MCP structuredContent must be a JSON object at the top level.
        let wt = serde_json::to_value(ListWorktreesResult {
            worktrees: vec![WorktreeInfo {
                path: "/repo".into(),
                head: "abc123".into(),
                branch: Some("main".into()),
                is_bare: false,
            }],
        })
        .unwrap();
        assert!(wt.is_object());
        assert_eq!(wt["worktrees"][0]["branch"], "main");

        let br = serde_json::to_value(ListBranchesResult {
            branches: vec![BranchInfo {
                name: "main".into(),
                is_current: true,
                upstream: Some("origin/main".into()),
                is_merged: true,
            }],
        })
        .unwrap();
        assert!(br.is_object());
        assert_eq!(br["branches"][0]["is_current"], true);
    }
}
