//! Black-box integration tests for the `gct mcp` subcommand.
//!
//! These drive the compiled `gct` binary as a real MCP server subprocess:
//! newline-delimited JSON-RPC requests go to its stdin and responses are
//! read back from stdout, against a temp git repo created with the real
//! `git` CLI. Any stray print to stdout would corrupt the JSON-RPC stream
//! and fail these tests, so stdout purity is asserted implicitly.

use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use serde_json::{Value, json};

fn gct_bin() -> &'static str {
    env!("CARGO_BIN_EXE_gct")
}

fn run_git(dir: &Path, args: &[&str]) {
    let status = Command::new("git")
        .args(args)
        .current_dir(dir)
        .status()
        .expect("git should be installed and on PATH");
    assert!(status.success(), "git {args:?} failed in {dir:?}");
}

/// Initializes a temp repo with one commit on `main` and a repo-local
/// `.gct.toml` setting `worktree.dir = "wt"`, mirroring the setup in
/// `tests/cli_integration.rs` (see the rationale there).
fn init_repo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("failed to create temp dir");
    run_git(dir.path(), &["init", "-q", "-b", "main"]);
    run_git(
        dir.path(),
        &["config", "user.email", "gct-test@example.com"],
    );
    run_git(dir.path(), &["config", "user.name", "gct-test"]);
    std::fs::write(dir.path().join(".gct.toml"), "[worktree]\ndir = \"wt\"\n")
        .expect("write .gct.toml");
    std::fs::write(dir.path().join("README.md"), "hello\n").expect("write README");
    run_git(dir.path(), &["add", "."]);
    run_git(dir.path(), &["commit", "-q", "-m", "init"]);
    dir
}

/// A `gct mcp` subprocess plus the client side of its JSON-RPC stdio channel.
struct McpServer {
    child: Child,
    stdin: Option<ChildStdin>,
    reader: BufReader<ChildStdout>,
    next_id: u64,
}

impl McpServer {
    /// Spawns `gct mcp` in `dir` and performs the initialize handshake,
    /// returning the connection and the `initialize` result.
    fn start(dir: &Path) -> (Self, Value) {
        let mut child = Command::new(gct_bin())
            .arg("mcp")
            .current_dir(dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            // Never inherit an override from the outer test-runner environment.
            .env_remove("GCT_GIT_BIN")
            .env_remove("GCT_GH_BIN")
            .spawn()
            .expect("failed to spawn gct mcp");
        let stdin = child.stdin.take().expect("child stdin");
        let reader = BufReader::new(child.stdout.take().expect("child stdout"));
        let mut server = Self {
            child,
            stdin: Some(stdin),
            reader,
            next_id: 0,
        };

        let init = server.request(
            "initialize",
            json!({
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": {"name": "gct-test", "version": "0"}
            }),
        );
        let init = init.expect("initialize should succeed");
        server.notify("notifications/initialized");
        (server, init)
    }

    /// Sends a request and reads responses until the matching id arrives.
    /// Returns `Ok(result)` or `Err(error)` from the JSON-RPC response.
    fn request(&mut self, method: &str, params: Value) -> Result<Value, Value> {
        self.next_id += 1;
        let id = self.next_id;
        let msg = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        let stdin = self.stdin.as_mut().expect("stdin still open");
        writeln!(stdin, "{msg}").expect("write request");
        stdin.flush().expect("flush request");

        loop {
            let mut line = String::new();
            let n = self.reader.read_line(&mut line).expect("read response");
            assert!(n > 0, "server closed stdout before answering {method}");
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let value: Value = serde_json::from_str(line)
                .unwrap_or_else(|e| panic!("stdout is not clean JSON-RPC ({e}): {line:?}"));
            // Skip anything that isn't the response to this request
            // (e.g. server-initiated notifications).
            if value.get("id") == Some(&json!(id)) {
                if let Some(err) = value.get("error") {
                    return Err(err.clone());
                }
                return Ok(value["result"].clone());
            }
        }
    }

    fn notify(&mut self, method: &str) {
        let msg = json!({"jsonrpc": "2.0", "method": method});
        let stdin = self.stdin.as_mut().expect("stdin still open");
        writeln!(stdin, "{msg}").expect("write notification");
        stdin.flush().expect("flush notification");
    }

    /// Calls a tool and returns its `structuredContent`, panicking on any
    /// JSON-RPC error or `isError` tool result.
    fn call_tool(&mut self, name: &str, arguments: Value) -> Value {
        let result = self
            .request("tools/call", json!({"name": name, "arguments": arguments}))
            .unwrap_or_else(|e| panic!("tools/call {name} failed: {e}"));
        assert_ne!(
            result.get("isError"),
            Some(&json!(true)),
            "tool {name} returned isError: {result}"
        );
        result
            .get("structuredContent")
            .unwrap_or_else(|| panic!("tool {name} returned no structuredContent: {result}"))
            .clone()
    }

    /// Closes stdin and asserts the server shuts down cleanly.
    fn shutdown(mut self) {
        drop(self.stdin.take());
        let status = self.child.wait().expect("wait for gct mcp");
        assert!(
            status.success(),
            "gct mcp should exit 0 on stdin close, got {status:?}"
        );
    }
}

impl Drop for McpServer {
    fn drop(&mut self) {
        // Don't leak the subprocess if a test assertion fails mid-flight.
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn initialize_and_tools_list() {
    let repo = init_repo();
    let (mut server, init) = McpServer::start(repo.path());

    assert_eq!(init["serverInfo"]["name"], "gct");
    let instructions = init["instructions"].as_str().unwrap_or_default();
    assert!(
        instructions.contains("create_worktree"),
        "instructions should steer agents to create_worktree: {instructions:?}"
    );

    let result = server.request("tools/list", json!({})).unwrap();
    let tools = result["tools"].as_array().expect("tools array");
    let mut names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
    names.sort_unstable();
    assert_eq!(
        names,
        ["create_worktree", "list_branches", "list_worktrees"]
    );
    for tool in tools {
        assert!(
            tool["inputSchema"].is_object(),
            "tool {} lacks inputSchema",
            tool["name"]
        );
        assert!(
            tool["outputSchema"].is_object(),
            "tool {} lacks outputSchema",
            tool["name"]
        );
    }

    server.shutdown();
}

#[test]
fn list_worktrees_returns_structured_json() {
    let repo = init_repo();
    let (mut server, _) = McpServer::start(repo.path());

    let content = server.call_tool("list_worktrees", json!({}));
    let worktrees = content["worktrees"].as_array().expect("worktrees array");
    assert_eq!(worktrees.len(), 1, "fresh repo has one worktree: {content}");
    assert_eq!(worktrees[0]["branch"], "main");
    assert_eq!(worktrees[0]["is_bare"], false);
    assert!(worktrees[0]["path"].as_str().unwrap().contains("/"));

    server.shutdown();
}

#[test]
fn list_branches_marks_current() {
    let repo = init_repo();
    run_git(repo.path(), &["branch", "feature/y"]);
    let (mut server, _) = McpServer::start(repo.path());

    let content = server.call_tool("list_branches", json!({}));
    let branches = content["branches"].as_array().expect("branches array");
    let find = |name: &str| {
        branches
            .iter()
            .find(|b| b["name"] == name)
            .unwrap_or_else(|| panic!("branch {name} missing from {content}"))
    };
    assert_eq!(find("main")["is_current"], true);
    assert_eq!(find("feature/y")["is_current"], false);

    server.shutdown();
}

#[test]
fn create_worktree_creates_runs_hooks_and_is_idempotent() {
    let repo = init_repo();
    // Add a post-create copy hook for an untracked file, the flagship use
    // case: .env doesn't travel with `git worktree add`, gct copies it.
    std::fs::write(repo.path().join(".env"), "SECRET=1\n").expect("write .env");
    std::fs::write(
        repo.path().join(".gct.toml"),
        "[worktree]\ndir = \"wt\"\n\n[[worktree.post_create]]\ntype = \"copy\"\nfrom = \".env\"\nto = \".env\"\n",
    )
    .expect("write .gct.toml");
    run_git(repo.path(), &["branch", "feature/x"]);
    let (mut server, _) = McpServer::start(repo.path());

    let content = server.call_tool("create_worktree", json!({"branch": "feature/x"}));
    assert_eq!(content["branch"], "feature/x");
    assert_eq!(content["created"], true);
    assert_eq!(content["fetched_from_origin"], false);
    assert_eq!(content["hook_errors"], json!([]));
    let path = content["path"].as_str().expect("path string");
    assert!(
        path.ends_with("wt/feature/x"),
        "path should follow the configured `wt/` layout: {path:?}"
    );
    assert!(Path::new(path).is_dir(), "worktree dir should exist");
    assert_eq!(
        std::fs::read_to_string(Path::new(path).join(".env")).expect(".env copied by hook"),
        "SECRET=1\n"
    );

    // Second call is idempotent: same worktree, created=false. Compare
    // canonicalized paths (git may report a normalized form).
    let content2 = server.call_tool("create_worktree", json!({"branch": "feature/x"}));
    assert_eq!(content2["created"], false);
    assert_eq!(
        std::fs::canonicalize(path).unwrap(),
        std::fs::canonicalize(content2["path"].as_str().unwrap()).unwrap()
    );

    server.shutdown();
}

#[test]
fn create_worktree_errors_outside_git_repo() {
    let dir = tempfile::tempdir().expect("failed to create temp dir");
    let (mut server, _) = McpServer::start(dir.path());

    let err = server
        .request(
            "tools/call",
            json!({"name": "create_worktree", "arguments": {"branch": "feature/x"}}),
        )
        .expect_err("create_worktree should fail outside a git repository");
    assert!(
        err["message"]
            .as_str()
            .unwrap_or_default()
            .contains("not a git repository"),
        "unexpected error: {err}"
    );

    server.shutdown();
}
