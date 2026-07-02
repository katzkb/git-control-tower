//! End-to-end smoke tests for the gct TUI.
//!
//! The TUI renders to `/dev/tty` (not stdout), so these tests launch the
//! compiled `gct` binary under a real pseudo-terminal via `portable-pty`
//! (whose `spawn_command` makes the PTY slave the child's controlling
//! terminal, so `/dev/tty` inside gct resolves to it). The raw escape-code
//! stream is fed into a `vt100` parser, and assertions run against the
//! rendered screen grid — ratatui redraws with cursor jumps, so grepping
//! the byte stream directly would be unreliable.
//!
//! Determinism: each test gets its own fixture git repo (frozen author +
//! dates), an isolated `$HOME`, and a stub `gh` injected via the existing
//! `GCT_GH_BIN` seam (`src/git/command.rs`), so no network or auth is ever
//! touched. Synchronization is exclusively poll-until-visible with a
//! timeout — no fixed sleeps.

#![cfg(unix)]

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use portable_pty::{CommandBuilder, PtySize, native_pty_system};

const COLS: u16 = 120;
const ROWS: u16 = 40;
const POLL_INTERVAL: Duration = Duration::from_millis(25);
const SCREEN_TIMEOUT: Duration = Duration::from_secs(10);
const EXIT_TIMEOUT: Duration = Duration::from_secs(5);

/// Frozen commit date so the Log view (`--date=short`) always shows the
/// same string regardless of when the test runs.
const FROZEN_DATE: &str = "2026-01-02T03:04:05 +0000";

fn gct_bin() -> &'static str {
    env!("CARGO_BIN_EXE_gct")
}

fn run_git(dir: &Path, args: &[&str]) {
    let status = Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_AUTHOR_DATE", FROZEN_DATE)
        .env("GIT_COMMITTER_DATE", FROZEN_DATE)
        .status()
        .expect("git should be installed and on PATH");
    assert!(status.success(), "git {args:?} failed in {dir:?}");
}

/// Everything one TUI test needs on disk: the fixture repo, an isolated
/// `$HOME`, and the stub `gh`. TempDirs clean themselves up on drop.
struct Fixture {
    repo: tempfile::TempDir,
    home: tempfile::TempDir,
    _stub_dir: tempfile::TempDir,
    gh_stub: PathBuf,
}

/// Builds a deterministic repo: one commit on `main` plus two feature
/// branches that are each one commit ahead (so neither renders `[merged]`),
/// with `main` left checked out (so it renders the current-branch `*`).
/// An `origin` remote must exist for gct to issue the local-PRs gh query.
fn setup_fixture() -> Fixture {
    let repo = tempfile::tempdir().expect("create repo temp dir");
    let dir = repo.path();
    run_git(dir, &["init", "-q", "-b", "main"]);
    run_git(dir, &["config", "user.email", "gct-test@example.com"]);
    run_git(dir, &["config", "user.name", "gct-test"]);
    std::fs::write(dir.join(".gct.toml"), "[worktree]\ndir = \"wt\"\n").expect("write .gct.toml");
    std::fs::write(dir.join("README.md"), "hello\n").expect("write README");
    run_git(dir, &["add", "."]);
    run_git(dir, &["commit", "-q", "-m", "init: add readme"]);

    for (branch, file, msg) in [
        ("feature/alpha", "alpha.txt", "alpha: add alpha feature"),
        ("feature/beta", "beta.txt", "beta: add beta feature"),
    ] {
        run_git(dir, &["checkout", "-q", "-b", branch, "main"]);
        std::fs::write(dir.join(file), "content\n").expect("write branch file");
        run_git(dir, &["add", "."]);
        run_git(dir, &["commit", "-q", "-m", msg]);
    }
    run_git(dir, &["checkout", "-q", "main"]);
    run_git(
        dir,
        &[
            "remote",
            "add",
            "origin",
            "https://github.com/demo-user/demo-project.git",
        ],
    );

    let home = tempfile::tempdir().expect("create home temp dir");
    let stub_dir = tempfile::tempdir().expect("create stub temp dir");
    let gh_stub = write_gh_stub(stub_dir.path());
    Fixture {
        repo,
        home,
        _stub_dir: stub_dir,
        gh_stub,
    }
}

/// Writes a minimal `gh` stub covering every call gct makes in these tests:
/// the prerequisite `--version` check, the viewer-login query (gct passes
/// `--jq .data.viewer.login`, so it prints the bare login), the local-PRs
/// `repository(` query (one canned PR matched to `feature/alpha` by
/// `headRefName` — alias names are irrelevant to the parser), and the
/// My PR / Review `search(query:` fan-out (always empty). `pr view` is
/// answered defensively so a selection change can never surface a
/// "Failed to load PR" notification mid-assertion. Anything else fails
/// fast on stderr rather than hanging.
fn write_gh_stub(dir: &Path) -> PathBuf {
    let path = dir.join("gh-test-stub");
    std::fs::write(
        &path,
        r#"#!/bin/sh
case "${1:-}" in
  --version)
    echo "gh version 2.76.0 (gct-test-stub)"
    ;;
  api)
    query=""
    for arg in "$@"; do
      case "$arg" in
        query=*) query="${arg#query=}" ;;
      esac
    done
    case "$query" in
      *"viewer{login}"*)
        echo "test-user"
        ;;
      *"search(query:"*)
        echo '{"data":{"search":{"nodes":[]}}}'
        ;;
      *"repository("*)
        cat <<'JSON'
{"data":{"repository":{"b0":{"nodes":[{"number":42,"title":"Add alpha feature","state":"OPEN","headRefName":"feature/alpha","updatedAt":"2026-01-01T00:00:00Z","isDraft":false,"author":{"login":"test-user"},"reviewRequests":{"nodes":[]}}]}}}}
JSON
        ;;
      *)
        echo "gh-test-stub: unsupported graphql query: $query" >&2
        exit 1
        ;;
    esac
    ;;
  pr)
    if [ "${2:-}" = "view" ]; then
      printf '{"number":%s,"title":"Add alpha feature","author":{"login":"test-user"},"state":"OPEN","body":"stub","additions":1,"deletions":0,"headRefName":"feature/alpha"}\n' "${3:-42}"
    else
      echo "[]"
    fi
    ;;
  *)
    echo "gh-test-stub: unsupported command: $*" >&2
    exit 1
    ;;
esac
"#,
    )
    .expect("write gh stub");
    let mut perms = std::fs::metadata(&path).unwrap().permissions();
    std::os::unix::fs::PermissionsExt::set_mode(&mut perms, 0o755);
    std::fs::set_permissions(&path, perms).expect("chmod gh stub");
    path
}

/// A running gct under a PTY, with a live vt100 screen fed by a reader
/// thread. The reader stays alive until PTY EOF so the child's shutdown
/// escape sequences (LeaveAlternateScreen) never block on a full buffer.
struct TuiSession {
    child: Box<dyn portable_pty::Child + Send + Sync>,
    writer: Box<dyn Write + Send>,
    parser: Arc<Mutex<vt100::Parser>>,
    // Keeps the PTY master open for the whole session; dropping it early
    // would close the terminal underneath the child.
    _master: Box<dyn portable_pty::MasterPty + Send>,
    exited: bool,
}

fn spawn_gct(fx: &Fixture) -> TuiSession {
    let pty_system = native_pty_system();
    let pair = pty_system
        .openpty(PtySize {
            rows: ROWS,
            cols: COLS,
            pixel_width: 0,
            pixel_height: 0,
        })
        .expect("openpty");

    let mut cmd = CommandBuilder::new(gct_bin());
    cmd.cwd(fx.repo.path());
    // Start from a clean environment so no gct/git/gh setting can leak in
    // from the outer test runner; real `git` is reached via the inherited
    // PATH value, `gh` is always the stub.
    cmd.env_clear();
    cmd.env("PATH", std::env::var_os("PATH").unwrap_or_default());
    cmd.env("HOME", fx.home.path());
    cmd.env("TERM", "xterm-256color");
    cmd.env("GIT_CONFIG_NOSYSTEM", "1");
    cmd.env("GCT_GH_BIN", &fx.gh_stub);

    let child = pair.slave.spawn_command(cmd).expect("spawn gct");
    drop(pair.slave);

    let mut reader = pair.master.try_clone_reader().expect("clone pty reader");
    let writer = pair.master.take_writer().expect("take pty writer");
    let parser = Arc::new(Mutex::new(vt100::Parser::new(ROWS, COLS, 0)));
    let parser_for_reader = Arc::clone(&parser);
    thread::spawn(move || {
        let mut buf = [0u8; 8192];
        loop {
            match reader.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => parser_for_reader.lock().unwrap().process(&buf[..n]),
            }
        }
    });

    TuiSession {
        child,
        writer,
        parser,
        _master: pair.master,
        exited: false,
    }
}

impl TuiSession {
    /// Current rendered screen as plain text (one line per terminal row).
    fn screen(&self) -> String {
        self.parser.lock().unwrap().screen().contents()
    }

    fn send_keys(&mut self, keys: &str) {
        self.writer.write_all(keys.as_bytes()).expect("pty write");
        self.writer.flush().expect("pty flush");
    }

    /// Polls the screen until `pred` matches, panicking with a full screen
    /// dump (and child liveness) on timeout so CI failures are diagnosable
    /// from the log alone.
    fn wait_for(&mut self, desc: &str, pred: impl Fn(&str) -> bool) {
        let deadline = Instant::now() + SCREEN_TIMEOUT;
        loop {
            let screen = self.screen();
            if pred(&screen) {
                return;
            }
            if Instant::now() >= deadline {
                let liveness = match self.child.try_wait() {
                    Ok(None) => "still running".to_string(),
                    Ok(Some(status)) => format!("exited ({status:?})"),
                    Err(e) => format!("try_wait failed: {e}"),
                };
                panic!(
                    "timed out after {}s waiting for: {desc}\nchild: {liveness}\n--- screen ---\n{screen}\n--- end screen ---",
                    SCREEN_TIMEOUT.as_secs()
                );
            }
            thread::sleep(POLL_INTERVAL);
        }
    }

    fn wait_for_contains(&mut self, needle: &str) {
        self.wait_for(&format!("screen to contain {needle:?}"), |s| {
            s.contains(needle)
        });
    }

    /// Sends `q` and asserts the process exits successfully. Normal quit
    /// prints nothing to stdout and returns exit code 0.
    fn quit_and_assert_clean_exit(mut self) {
        self.send_keys("q");
        let deadline = Instant::now() + EXIT_TIMEOUT;
        loop {
            match self.child.try_wait().expect("try_wait failed") {
                Some(status) => {
                    self.exited = true;
                    assert!(
                        status.success(),
                        "gct exited with failure status {status:?}\n--- screen ---\n{}\n--- end screen ---",
                        self.screen()
                    );
                    return;
                }
                None => {
                    if Instant::now() >= deadline {
                        panic!(
                            "gct did not exit within {}s after 'q'\n--- screen ---\n{}\n--- end screen ---",
                            EXIT_TIMEOUT.as_secs(),
                            self.screen()
                        );
                    }
                    thread::sleep(POLL_INTERVAL);
                }
            }
        }
    }

    /// Waits until the Main view has finished its initial render: sidebar
    /// title, all fixture branches, current-branch marker, and status bar.
    fn wait_for_startup(&mut self) {
        self.wait_for("initial Main view render", |s| {
            s.contains(" Branches ")
                && s.contains(" main *")
                && s.contains("feature/alpha")
                && s.contains("feature/beta")
                && s.contains("[Local]")
                && s.contains("q:Quit")
        });
    }
}

impl Drop for TuiSession {
    fn drop(&mut self) {
        // A panicking test must not leave gct alive holding the PTY open,
        // or the suite would hang instead of reporting the failure.
        if !self.exited {
            let _ = self.child.kill();
        }
    }
}

#[test]
fn startup_shows_branches_and_quits_cleanly() {
    let fx = setup_fixture();
    let mut session = spawn_gct(&fx);
    session.wait_for_startup();
    session.quit_and_assert_clean_exit();
}

#[test]
fn log_view_shows_fixture_commits() {
    let fx = setup_fixture();
    let mut session = spawn_gct(&fx);
    session.wait_for_startup();

    session.send_keys("l");
    session.wait_for("Log view with the fixture commit", |s| {
        s.contains(" Log ")
            && s.contains("[Log]")
            // Assert on message/author/date; the short hash is not stable.
            && s.contains("init: add readme  (gct-test, 2026-01-02)")
    });

    // Esc returns to the Main view.
    session.send_keys("\x1b");
    session.wait_for_contains("[Local]");
    session.quit_and_assert_clean_exit();
}

#[test]
fn local_view_renders_stubbed_pr_number() {
    let fx = setup_fixture();
    let mut session = spawn_gct(&fx);
    session.wait_for_startup();

    // Proves the async pipeline end to end: fetch_local_prs -> gh stub ->
    // LocalPrList merge -> sidebar render. The PR number is attached to
    // feature/alpha by headRefName; feature/beta must stay PR-less.
    session.wait_for_contains("feature/alpha #42");
    let screen = session.screen();
    assert!(
        !screen.contains("feature/beta #"),
        "feature/beta should have no PR number\n--- screen ---\n{screen}\n--- end screen ---"
    );
    session.quit_and_assert_clean_exit();
}

#[test]
fn filter_switching_shows_empty_states() {
    let fx = setup_fixture();
    let mut session = spawn_gct(&fx);
    session.wait_for_startup();

    session.send_keys("2");
    session.wait_for("My PR view empty state", |s| {
        s.contains("[My PR]") && s.contains("No branches with your PRs")
    });

    session.send_keys("3");
    session.wait_for("Review view empty state", |s| {
        s.contains("[Review]") && s.contains("No branches awaiting review")
    });

    session.send_keys("1");
    session.wait_for("back on the Local view", |s| {
        s.contains("[Local]") && s.contains(" main *")
    });
    session.quit_and_assert_clean_exit();
}
