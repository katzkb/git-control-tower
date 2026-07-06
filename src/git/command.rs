use std::collections::VecDeque;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use tokio::process::Command;

/// Upper bound on a single `git`/`gh` invocation. Guards against a stalled
/// network call, proxy hang, or an interactive prompt waiting on stdin —
/// none of which should ever freeze the app indefinitely.
const CMD_TIMEOUT: Duration = Duration::from_secs(30);

/// Injection seam: resolves the actual binary spawned for `git`/`gh` calls,
/// letting tests substitute a stub executable (a fake `git`/`gh` script) via
/// `GCT_GIT_BIN` / `GCT_GH_BIN` without touching production call sites, which
/// always pass the logical name (`"git"` or `"gh"`).
fn resolve_executable(executable: &'static str) -> String {
    let env_var = match executable {
        "git" => "GCT_GIT_BIN",
        "gh" => "GCT_GH_BIN",
        _ => return executable.to_string(),
    };
    std::env::var(env_var).unwrap_or_else(|_| executable.to_string())
}

/// Runs `cmd` to completion, enforcing [`CMD_TIMEOUT`] and closing stdin so a
/// stray interactive prompt (credential/auth/pager/editor) can't block on
/// input that will never arrive. Shared by every code path that spawns
/// `git`/`gh` so no invocation can hang forever.
async fn run_with_timeout(cmd: &mut Command) -> std::io::Result<std::process::Output> {
    run_with_timeout_for(cmd, CMD_TIMEOUT).await
}

/// Parameterized over the timeout duration so tests can exercise the
/// timeout path without waiting out the real [`CMD_TIMEOUT`].
async fn run_with_timeout_for(
    cmd: &mut Command,
    timeout_duration: Duration,
) -> std::io::Result<std::process::Output> {
    cmd.stdin(Stdio::null())
        // Defense in depth: even with stdin closed, make sure git/gh don't
        // themselves attempt an interactive prompt.
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GH_PROMPT_DISABLED", "1")
        // Pin the locale so human-facing output we parse (e.g. the tracking
        // brackets in `git branch -vv`) is never translated. LC_ALL overrides
        // LANG and every LC_* category.
        .env("LC_ALL", "C");
    match tokio::time::timeout(timeout_duration, cmd.output()).await {
        Ok(result) => result,
        Err(_) => Err(std::io::Error::new(
            std::io::ErrorKind::TimedOut,
            format!("command timed out after {timeout_duration:?}"),
        )),
    }
}

static DEBUG_LOG: OnceLock<Option<Mutex<File>>> = OnceLock::new();

pub fn init_debug_log(verbose: bool) {
    DEBUG_LOG.get_or_init(|| {
        let env_enabled = std::env::var("GCT_DEBUG")
            .map(|v| matches!(v.as_str(), "1" | "true"))
            .unwrap_or(false);
        if !env_enabled && !verbose {
            return None;
        }
        let path = debug_log_path();
        if let Some(parent) = path.parent()
            && let Err(e) = std::fs::create_dir_all(parent)
        {
            eprintln!(
                "Warning: cannot create debug log directory {}: {e}",
                parent.display()
            );
            return None;
        }
        match OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&path)
        {
            Ok(f) => {
                eprintln!("Debug log: {}", path.display());
                Some(Mutex::new(f))
            }
            Err(e) => {
                eprintln!("Warning: cannot open debug log {}: {e}", path.display());
                None
            }
        }
    });
}

fn debug_log_path() -> PathBuf {
    crate::config::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".config/gct/debug.log")
}

pub fn debug_log(msg: &str) {
    if let Some(Some(file)) = DEBUG_LOG.get()
        && let Ok(mut f) = file.lock()
    {
        let _ = writeln!(f, "{msg}");
        let _ = f.flush();
    }
}

// ---- Command history (session-only, in-memory ring buffer) ----

const MAX_HISTORY: usize = 1000;

static COMMAND_HISTORY: OnceLock<Mutex<VecDeque<CommandRecord>>> = OnceLock::new();

/// Monotonic start time for the current session — lets the History view
/// format each record as an offset (e.g. `+1.24s`) instead of wall-clock
/// time, avoiding the need for a date/time dependency.
static SESSION_START: OnceLock<Instant> = OnceLock::new();

fn session_start() -> Instant {
    *SESSION_START.get_or_init(Instant::now)
}

pub fn session_elapsed_at(timestamp: Instant) -> Duration {
    timestamp.saturating_duration_since(session_start())
}

/// Structured record of a single `git`/`gh` invocation.
/// Lives only in the in-memory ring buffer used by the History view.
#[derive(Debug, Clone)]
pub struct CommandRecord {
    pub started_at: Instant, // monotonic; used for elapsed-since-session-start display
    pub executable: &'static str, // "git" or "gh"
    pub args: Vec<String>,
    pub success: bool,
    pub duration: Duration,
    /// stdout bytes on success, stderr bytes on failure.
    pub output_bytes: usize,
    pub error: Option<String>,
}

fn history() -> &'static Mutex<VecDeque<CommandRecord>> {
    COMMAND_HISTORY.get_or_init(|| Mutex::new(VecDeque::with_capacity(MAX_HISTORY)))
}

fn push_record(record: CommandRecord) {
    if let Ok(mut buf) = history().lock() {
        if buf.len() == MAX_HISTORY {
            buf.pop_front();
        }
        buf.push_back(record);
    }
}

/// Returns a clone of the current command history, newest first.
pub fn command_history_snapshot() -> Vec<CommandRecord> {
    history()
        .lock()
        .map(|buf| buf.iter().rev().cloned().collect())
        .unwrap_or_default()
}

/// Cheap count of recorded commands — avoids cloning the whole buffer
/// when only the length is needed (e.g. for scroll-bound checks).
pub fn command_history_len() -> usize {
    history().lock().map(|b| b.len()).unwrap_or(0)
}

// ---- Command execution ----

pub async fn run_git(args: &[&str]) -> Result<String> {
    run_cmd_tagged("git", args).await
}

pub async fn run_gh(args: &[&str]) -> Result<String> {
    run_cmd_tagged("gh", args).await
}

/// Run `git -C <cwd> <args>` for repo-specific operations.
pub async fn run_git_in(cwd: &Path, args: &[&str]) -> Result<String> {
    let cwd_str = cwd.to_string_lossy().to_string();
    let mut full = Vec::with_capacity(args.len() + 2);
    full.push("-C");
    full.push(cwd_str.as_str());
    full.extend_from_slice(args);
    run_cmd_tagged("git", &full).await
}

/// Shared execution path for `run_git`/`run_gh`:
/// - logs to the debug file if enabled
/// - records a `CommandRecord` in the in-memory history buffer
async fn run_cmd_tagged(executable: &'static str, args: &[&str]) -> Result<String> {
    // Initialize session start lazily; cheap and idempotent.
    let _ = session_start();
    debug_log(&format!("$ {executable} {}", args.join(" ")));
    let started_at = Instant::now();
    let owned_args: Vec<String> = args.iter().map(|s| s.to_string()).collect();

    let mut cmd = Command::new(resolve_executable(executable));
    cmd.args(args);
    let output = match run_with_timeout(&mut cmd).await {
        Ok(o) => o,
        Err(e) => {
            push_record(CommandRecord {
                started_at,
                executable,
                args: owned_args,
                success: false,
                duration: started_at.elapsed(),
                output_bytes: 0,
                error: Some(e.to_string()),
            });
            return Err(anyhow::Error::new(e))
                .with_context(|| format!("failed to execute {executable}"));
        }
    };

    let duration = started_at.elapsed();

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        debug_log(&format!("  → FAIL: {stderr}"));
        push_record(CommandRecord {
            started_at,
            executable,
            args: owned_args.clone(),
            success: false,
            duration,
            output_bytes: output.stderr.len(),
            error: Some(stderr.clone()),
        });
        bail!("{executable} {} failed: {stderr}", owned_args.join(" "));
    }

    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    debug_log(&format!("  → OK ({} bytes)", stdout.len()));
    push_record(CommandRecord {
        started_at,
        executable,
        args: owned_args,
        success: true,
        duration,
        output_bytes: stdout.len(),
        error: None,
    });
    Ok(stdout)
}

#[cfg(test)]
mod plumbing_tests {
    use super::*;

    #[tokio::test]
    async fn run_git_in_uses_cwd() {
        let cwd = std::env::current_dir().unwrap();
        let result = run_git_in(&cwd, &["rev-parse", "--show-toplevel"]).await;
        let toplevel = result.expect("git -C should succeed");
        let expected = std::fs::canonicalize(&cwd).unwrap();
        let actual = std::fs::canonicalize(toplevel.trim()).unwrap();
        assert_eq!(actual, expected);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn run_with_timeout_for_times_out_on_slow_command() {
        let mut cmd = Command::new("sh");
        cmd.args(["-c", "sleep 2"]);
        let err = run_with_timeout_for(&mut cmd, Duration::from_millis(50))
            .await
            .expect_err("slow command should time out");
        assert_eq!(err.kind(), std::io::ErrorKind::TimedOut);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn run_with_timeout_closes_stdin() {
        // `cat` with no args reads from stdin until EOF. With stdin closed
        // it should see EOF immediately and exit, instead of hanging
        // forever waiting on input that will never arrive.
        let mut cmd = Command::new("cat");
        let output = tokio::time::timeout(Duration::from_secs(5), run_with_timeout(&mut cmd))
            .await
            .expect("cat should not hang waiting on stdin")
            .expect("cat should exit successfully on closed stdin");
        assert!(output.stdout.is_empty());
    }
}
