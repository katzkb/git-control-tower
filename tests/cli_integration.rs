//! Black-box integration tests for gct's non-TUI CLI subcommands.
//!
//! These drive the compiled `gct` binary as a real subprocess against a
//! temp git repo created with the real `git` CLI (via `tempfile`), so they
//! exercise the actual command/exec/parse round-trip that unit tests can't
//! reach. `gct` is a binary-only crate (no `[lib]` target), so this is the
//! only way to test it as a black box rather than reaching into internals.
//!
//! A couple of tests also exercise the `GCT_GIT_BIN` injection seam
//! (`src/git/command.rs`), proving that overriding it actually substitutes
//! a different executable rather than being silently ignored.

use std::path::Path;
use std::process::{Command, Output};

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

/// Initializes a temp repo with one commit on `main`, so it looks like a
/// normal small local repository from gct's point of view.
///
/// Sets `worktree.dir = "wt"` via a repo-local `.gct.toml` so any worktree
/// `gct wt` creates lands inside this same temp dir (and is cleaned up when
/// it drops), rather than the default `../<branch>` — which, since every
/// `tempfile::tempdir()` repo is a sibling under `/tmp`, would make separate
/// test repos collide on the same `/tmp/<branch>` path.
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

fn run_gct(dir: &Path, args: &[&str]) -> Output {
    Command::new(gct_bin())
        .args(args)
        .current_dir(dir)
        // Never inherit an override from the outer test-runner environment.
        .env_remove("GCT_GIT_BIN")
        .env_remove("GCT_GH_BIN")
        .output()
        .expect("failed to run gct binary")
}

fn run_gct_with_git_bin(dir: &Path, args: &[&str], git_bin: &str) -> Output {
    Command::new(gct_bin())
        .args(args)
        .current_dir(dir)
        .env("GCT_GIT_BIN", git_bin)
        .env_remove("GCT_GH_BIN")
        .output()
        .expect("failed to run gct binary")
}

#[test]
fn ls_branches_lists_created_branches() {
    let repo = init_repo();
    run_git(repo.path(), &["branch", "feature/foo"]);

    let output = run_gct(repo.path(), &["ls", "branches"]);
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let names: Vec<&str> = stdout.lines().collect();
    assert!(names.contains(&"main"), "expected 'main' in {names:?}");
    assert!(
        names.contains(&"feature/foo"),
        "expected 'feature/foo' in {names:?}"
    );
}

#[test]
fn ls_worktrees_lists_main_worktree() {
    let repo = init_repo();

    let output = run_gct(repo.path(), &["ls", "worktrees"]);
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(
        stdout.lines().count(),
        1,
        "expected exactly one worktree: {stdout:?}"
    );
    assert!(
        stdout.starts_with("main\t"),
        "expected main branch worktree: {stdout:?}"
    );
}

#[test]
fn version_flag_prints_version() {
    let output = Command::new(gct_bin())
        .arg("--version")
        .output()
        .expect("failed to run gct binary");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.trim().starts_with("gct v"),
        "unexpected version output: {stdout:?}"
    );
}

#[test]
fn cd_reports_error_for_unknown_branch() {
    let repo = init_repo();

    let output = run_gct(repo.path(), &["cd", "does-not-exist"]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("no worktree for branch 'does-not-exist'"),
        "unexpected stderr: {stderr:?}"
    );
}

#[test]
fn wt_creates_worktree_and_prints_path() {
    let repo = init_repo();
    run_git(repo.path(), &["branch", "feature/bar"]);

    let output = run_gct(repo.path(), &["wt", "feature/bar"]);
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let printed_path = String::from_utf8_lossy(&output.stdout).trim().to_string();
    assert!(
        !printed_path.is_empty(),
        "expected a worktree path on stdout"
    );
    assert!(
        Path::new(&printed_path).is_dir(),
        "worktree path {printed_path:?} should exist after `gct wt` (real `git worktree add` should have run)"
    );

    // Idempotent: re-running for the same branch reuses the existing worktree
    // instead of erroring, and points at the same directory. The two calls
    // aren't guaranteed to print byte-identical strings (the create path is
    // built from the config template, e.g. with a literal `..`, while the
    // reused path comes back from `git worktree list --porcelain`, which
    // reports git's own normalized form) — compare canonicalized paths.
    let output2 = run_gct(repo.path(), &["wt", "feature/bar"]);
    assert!(output2.status.success());
    let printed_path2 = String::from_utf8_lossy(&output2.stdout).trim().to_string();
    assert_eq!(
        std::fs::canonicalize(&printed_path).unwrap(),
        std::fs::canonicalize(&printed_path2).unwrap(),
    );
}

/// Proves the `GCT_GIT_BIN` injection seam actually changes which binary gets
/// spawned, rather than being silently ignored: pointing it at a path that
/// doesn't exist must break a command that would otherwise succeed against
/// the perfectly valid repo `init_repo()` set up.
#[test]
fn git_bin_override_nonexistent_path_causes_failure() {
    let repo = init_repo();

    // Sanity check: without the override, this succeeds.
    let baseline = run_gct(repo.path(), &["ls", "branches"]);
    assert!(baseline.status.success());

    let overridden = run_gct_with_git_bin(
        repo.path(),
        &["ls", "branches"],
        "/nonexistent/gct-test-fake-git",
    );
    assert!(
        !overridden.status.success(),
        "expected failure once GCT_GIT_BIN points at a nonexistent binary"
    );
}

/// Proves `GCT_GIT_BIN` can substitute a *working* stub binary: a fake `git`
/// script that always reports one canned worktree, regardless of the real
/// repo state, mirroring the approach `scripts/demo/gh-stub` uses for `gh`.
#[test]
#[cfg(unix)]
fn git_bin_override_stub_script_is_used() {
    let repo = init_repo();
    let stub_dir = tempfile::tempdir().expect("failed to create stub dir");
    let stub_path = stub_dir.path().join("fake-git");
    std::fs::write(
        &stub_path,
        r#"#!/bin/sh
if [ "$1" = "rev-parse" ] && [ "$2" = "--git-dir" ]; then
  echo ".git"
  exit 0
fi
if [ "$1" = "worktree" ] && [ "$2" = "list" ]; then
  printf 'worktree /fake/stub/path\nHEAD 1111111111111111111111111111111111111111\nbranch refs/heads/stub-branch\n\n'
  exit 0
fi
echo "fake-git: unsupported args: $*" >&2
exit 1
"#,
    )
    .expect("write stub script");
    let mut perms = std::fs::metadata(&stub_path).unwrap().permissions();
    std::os::unix::fs::PermissionsExt::set_mode(&mut perms, 0o755);
    std::fs::set_permissions(&stub_path, perms).expect("chmod stub script");

    let output = run_gct_with_git_bin(
        repo.path(),
        &["ls", "worktrees"],
        stub_path.to_str().unwrap(),
    );
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(
        stdout.trim(),
        "stub-branch\t/fake/stub/path",
        "expected the stub's canned worktree, not the real repo's: {stdout:?}"
    );
}
