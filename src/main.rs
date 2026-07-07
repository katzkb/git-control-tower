mod app;
mod config;
mod data;
mod event;
mod git;
mod mcp;
mod ops;
mod ui;

use anyhow::Context;
use std::fs::OpenOptions;
use std::process;
use std::time::Duration;

use crossterm::event::KeyEventKind;
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use tokio::sync::mpsc;
use tokio::task::JoinSet;

use crate::app::App;
use crate::app::MainFilter;
use crate::app::{Command, OpProgress, OpStep};
use crate::event::{Event, EventHandler};
use crate::git::command::{run_gh, run_git, run_git_in};
use crate::git::parser::{parse_branches, parse_log, parse_worktrees};
use crate::git::types::{GitStatus, PrDetail, PullRequest, RepoId, ReviewState, ReviewStatus};
use crate::ui::notification::Notification;

/// Args used for fetching commits in the Log view (startup + `r` refresh).
const LOG_ARGS: &[&str] = &[
    "log",
    "--format=%h%x00%s%x00%an%x00%ad",
    "--date=short",
    "-n",
    "200",
];

enum AsyncResult {
    PrDetail {
        repo_id: crate::git::types::RepoId,
        detail: PrDetail,
    },
    PrDetailError {
        repo_id: crate::git::types::RepoId,
        number: u64,
        error: String,
    },
    GitStatus {
        wt_path: String,
        status: GitStatus,
    },
    GitStatusError(String),
    UserLogin(String),
    UserLoginError(String),
    LocalPrList(Vec<PullRequest>, Vec<String>),
    MyPrList(Vec<PullRequest>, Vec<String>),
    ReviewPrList(Vec<PullRequest>, Vec<String>),
    WtCreated {
        wt_path: String,
        copy_errors: Vec<String>,
        target_repo: Option<crate::git::types::RepoId>,
    },
    WtCreateError {
        wt_path: String,
        message: String,
    },
    WtForceDecisionRequested {
        path: String,
        reason: String,
    },
    OpStarted {
        op_id: u64,
        label: String,
    },
    OpStepBegin {
        op_id: u64,
        step: OpStep,
        command: String,
    },
    OpFinished {
        op_id: u64,
        success: bool,
        error: Option<String>,
    },
    OpAllDone {
        branches_deleted: Vec<String>,
        worktrees_removed: Vec<String>,
        failures: Vec<String>,
        wt_paths_claimed: Vec<String>,
    },
    WtListLoaded {
        repo_id: crate::git::types::RepoId,
        list: Vec<crate::git::types::Worktree>,
    },
    BranchesReloaded(BranchesReloadData),
    CommitsReloaded(Option<Vec<crate::git::types::Commit>>),
    BranchCreated {
        name: String,
        source: String,
    },
    BranchCreateError(String),
}

/// Result of a background branches/worktrees refresh. `None` for a field
/// means that particular git call failed, so the caller should keep the
/// previously-loaded value rather than overwrite it with an empty one.
struct BranchesReloadData {
    branches: Option<Vec<crate::git::types::Branch>>,
    worktrees: Option<Vec<crate::git::types::Worktree>>,
    errors: Vec<String>,
}

/// Fetches branches + worktrees without touching `App`, so it can run inside
/// `tokio::spawn` without blocking the event loop. Reuses `ops::list_branches`,
/// which already performs the same branch/default-branch/merged/rev-parse
/// sequence for the `gct ls` command.
async fn fetch_branches_and_worktrees() -> BranchesReloadData {
    let mut errors = Vec::new();

    let branches = match ops::list_branches().await {
        Ok(b) => Some(b),
        Err(e) => {
            errors.push(format!("git branch -vv failed: {e}"));
            None
        }
    };

    let worktrees = match run_git(&["worktree", "list", "--porcelain"]).await {
        Ok(output) => Some(parse_worktrees(&output)),
        Err(e) => {
            errors.push(format!("git worktree list failed: {e}"));
            None
        }
    };

    BranchesReloadData {
        branches,
        worktrees,
        errors,
    }
}

/// Display-friendly label for a worktree path: takes the last path segment.
fn wt_label_for(path: &str) -> String {
    std::path::Path::new(path)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(path)
        .to_string()
}

fn bulk_success_parts(branches: usize, worktrees: usize) -> Vec<String> {
    let mut parts = Vec::with_capacity(2);
    if branches > 0 {
        let label = if branches == 1 { "branch" } else { "branches" };
        parts.push(format!("{branches} {label}"));
    }
    if worktrees > 0 {
        let label = if worktrees == 1 {
            "worktree"
        } else {
            "worktrees"
        };
        parts.push(format!("{worktrees} {label}"));
    }
    parts
}

#[derive(Debug, Clone, Copy)]
enum DeleteMode {
    /// Try plain `worktree remove`; on failure, retry with `--force` (used for bulk).
    TryThenForce,
    /// Run `worktree remove` only; do NOT auto-fallback (used for single, first attempt).
    PlainOnly,
    /// Run `worktree remove --force` directly (used for single, after user confirms).
    ForceOnly,
}

struct DeleteOpResult {
    branch_name: Option<String>,
    wt_path: Option<String>,
    branch_deleted: bool,
    worktree_removed: bool,
    failure: Option<String>,
}

impl DeleteOpResult {
    fn collect_into(
        self,
        branches: &mut Vec<String>,
        wts: &mut Vec<String>,
        failures: &mut Vec<String>,
    ) {
        if self.branch_deleted
            && let Some(n) = self.branch_name
        {
            branches.push(n);
        }
        if self.worktree_removed
            && let Some(p) = self.wt_path
        {
            wts.push(p);
        }
        if let Some(f) = self.failure {
            failures.push(f);
        }
    }
}

async fn run_delete_op(
    op_id: u64,
    label: String,
    wt_path: Option<String>,
    branch_name: Option<String>,
    has_local_branch: bool,
    mode: DeleteMode,
    tx: mpsc::UnboundedSender<AsyncResult>,
) -> DeleteOpResult {
    let _ = tx.send(AsyncResult::OpStarted {
        op_id,
        label: label.clone(),
    });

    let mut worktree_removed = false;
    if let Some(path) = wt_path.as_ref() {
        // Plain attempt (skipped for ForceOnly)
        if matches!(mode, DeleteMode::TryThenForce | DeleteMode::PlainOnly) {
            let cmd = format!("git worktree remove {path}");
            let _ = tx.send(AsyncResult::OpStepBegin {
                op_id,
                step: OpStep::RunningWtRemove,
                command: cmd,
            });
            match run_git(&["worktree", "remove", path]).await {
                Ok(_) => worktree_removed = true,
                Err(e) => {
                    if matches!(mode, DeleteMode::PlainOnly) {
                        // Caller decides whether to escalate to force; report failure.
                        let short = e
                            .to_string()
                            .lines()
                            .next()
                            .unwrap_or("unknown")
                            .to_string();
                        let _ = tx.send(AsyncResult::OpFinished {
                            op_id,
                            success: false,
                            error: Some(short.clone()),
                        });
                        return DeleteOpResult {
                            branch_name,
                            wt_path,
                            branch_deleted: false,
                            worktree_removed: false,
                            failure: Some(format!("{label}: {short}")),
                        };
                    }
                    // TryThenForce: fall through to force attempt.
                }
            }
        }

        // Force attempt (TryThenForce after plain failure, or ForceOnly always)
        if !worktree_removed {
            let cmd = format!("git worktree remove --force {path}");
            let _ = tx.send(AsyncResult::OpStepBegin {
                op_id,
                step: OpStep::RunningWtForceRemove,
                command: cmd,
            });
            match run_git(&["worktree", "remove", "--force", path]).await {
                Ok(_) => worktree_removed = true,
                Err(e) => {
                    let short = e
                        .to_string()
                        .lines()
                        .next()
                        .unwrap_or("unknown")
                        .to_string();
                    let _ = tx.send(AsyncResult::OpFinished {
                        op_id,
                        success: false,
                        error: Some(short.clone()),
                    });
                    return DeleteOpResult {
                        branch_name,
                        wt_path,
                        branch_deleted: false,
                        worktree_removed: false,
                        failure: Some(format!("{label}: worktree remove failed: {short}")),
                    };
                }
            }
        }
    }

    let mut branch_deleted = false;
    if has_local_branch && let Some(name) = branch_name.as_ref() {
        let cmd = format!("git branch -D {name}");
        let _ = tx.send(AsyncResult::OpStepBegin {
            op_id,
            step: OpStep::RunningBranchDelete,
            command: cmd,
        });
        match run_git(&["branch", "-D", name]).await {
            Ok(_) => branch_deleted = true,
            Err(e) => {
                let short = e
                    .to_string()
                    .lines()
                    .next()
                    .unwrap_or("unknown")
                    .to_string();
                let _ = tx.send(AsyncResult::OpFinished {
                    op_id,
                    success: false,
                    error: Some(short.clone()),
                });
                return DeleteOpResult {
                    branch_name,
                    wt_path,
                    branch_deleted: false,
                    worktree_removed,
                    failure: Some(format!("{label}: {short}")),
                };
            }
        }
    }

    let _ = tx.send(AsyncResult::OpFinished {
        op_id,
        success: true,
        error: None,
    });

    DeleteOpResult {
        branch_name,
        wt_path,
        branch_deleted,
        worktree_removed,
        failure: None,
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Handle --version / -v before anything else
    if std::env::args().any(|a| a == "--version" || a == "-v") {
        println!("gct v{}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }

    // Handle --help / -h before any prerequisite checks so it works outside a git repo
    if std::env::args().any(|a| a == "--help" || a == "-h") {
        print_help();
        return Ok(());
    }

    // Handle shell-init subcommand
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(|s| s.as_str()) == Some("shell-init") {
        let shell = args.get(2).map(|s| s.as_str()).unwrap_or("zsh");
        print_shell_init(shell);
        return Ok(());
    }

    // Handle non-TUI subcommands. These run without launching the TUI and don't
    // need `gh`, so they're dispatched before the TUI-oriented prerequisite check.
    // `cd`/`wt` print a lone worktree path so the shell wrapper cd's into it; the
    // others print informational output that is never a bare directory path.
    match args.get(1).map(|s| s.as_str()) {
        Some("cd") => {
            let code = run_cd(args.get(2).map(|s| s.as_str())).await;
            process::exit(code);
        }
        Some("wt") => {
            let code = run_wt(args.get(2).map(|s| s.as_str())).await;
            process::exit(code);
        }
        Some("ls") => {
            let code = run_ls(args.get(2).map(|s| s.as_str())).await;
            process::exit(code);
        }
        Some("prune") => {
            let code = run_prune(&args[2..]).await;
            process::exit(code);
        }
        Some("completions") => {
            print_completions(args.get(2).map(|s| s.as_str()).unwrap_or("zsh"));
            return Ok(());
        }
        Some("mcp") => {
            // stdout is the JSON-RPC channel here; debug logging (GCT_DEBUG=1)
            // goes to a file, and notices/errors go to stderr only.
            crate::git::command::init_debug_log(false);
            let code = mcp::run_mcp_server().await;
            process::exit(code);
        }
        _ => {}
    }

    // Initialize debug logging (GCT_DEBUG=1 or --verbose enables it)
    let verbose = std::env::args().any(|a| a == "--verbose");
    crate::git::command::init_debug_log(verbose);

    // Startup checks and config loading before TUI init (eprintln is safe here)
    check_prerequisites().await;
    let (config, config_warnings) = config::load_config_with_warnings();
    for w in &config_warnings {
        eprintln!("Warning: {w}");
    }

    // Render TUI to /dev/tty so shell wrapper stdout capture doesn't interfere
    let mut tty = OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/tty")
        .context("failed to open /dev/tty — is a controlling terminal available?")?;
    enable_raw_mode()?;
    execute!(tty, EnterAlternateScreen)?;

    // Install panic hook to restore terminal on panic
    let original_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = disable_raw_mode();
        if let Ok(mut panic_tty) = OpenOptions::new().write(true).open("/dev/tty") {
            let _ = crossterm::execute!(panic_tty, LeaveAlternateScreen);
        }
        original_hook(info);
    }));

    let backend = CrosstermBackend::new(tty);
    let mut terminal = Terminal::new(backend)?;

    let (result, cd_path) = run(&mut terminal, config, config_warnings, verbose).await;

    // Restore terminal — always disable raw mode even if LeaveAlternateScreen fails
    let _ = execute!(terminal.backend_mut(), LeaveAlternateScreen);
    disable_raw_mode()?;

    if let Some(path) = cd_path {
        use std::io::Write;
        println!("{path}");
        let _ = std::io::stdout().flush();
    }
    result
}

async fn check_prerequisites() {
    if run_git(&["--version"]).await.is_err() {
        eprintln!("Error: git is not installed or not in PATH.");
        eprintln!("Please install git: https://git-scm.com/");
        process::exit(1);
    }

    if run_gh(&["--version"]).await.is_err() {
        eprintln!("Error: gh (GitHub CLI) is not installed or not in PATH.");
        eprintln!("Please install gh: https://cli.github.com/");
        process::exit(1);
    }

    if run_git(&["rev-parse", "--git-dir"]).await.is_err() {
        eprintln!("Error: not a git repository.");
        eprintln!("Please run gct from inside a git repository.");
        process::exit(1);
    }
}

/// Implements `gct cd <branch>`: print the branch's worktree path to stdout (the
/// shell wrapper cd's into it) and return an exit code. On any failure nothing is
/// written to stdout and a non-zero code is returned so the wrapper does not cd.
async fn run_cd(branch: Option<&str>) -> i32 {
    let branch = match branch {
        Some(b) if !b.is_empty() => b,
        _ => {
            eprintln!("Usage: gct cd <branch>");
            return 1;
        }
    };

    if run_git(&["rev-parse", "--git-dir"]).await.is_err() {
        eprintln!("Error: not a git repository.");
        return 1;
    }

    let output = match run_git(&["worktree", "list", "--porcelain"]).await {
        Ok(output) => output,
        Err(e) => {
            eprintln!("Error: failed to list worktrees: {e}");
            return 1;
        }
    };

    match ops::worktree_path_for_branch(&parse_worktrees(&output), branch) {
        Some(path) => {
            println!("{path}");
            0
        }
        None => {
            eprintln!(
                "Error: no worktree for branch '{branch}'.\nCreate one with `gct wt {branch}` or in the gct TUI (action menu → Worktree → create)."
            );
            1
        }
    }
}

/// Implements `gct wt <branch>`: reuse the branch's existing worktree or create one,
/// then print its path (the shell wrapper cd's into it). The create-side complement
/// to `cd`. On any failure nothing is written to stdout and a non-zero code is
/// returned so the wrapper does not cd. Thin printer over `ops::ensure_worktree`,
/// which is shared with the MCP server.
async fn run_wt(branch: Option<&str>) -> i32 {
    let branch = match branch {
        Some(b) if !b.is_empty() => b,
        _ => {
            eprintln!("Usage: gct wt <branch>");
            return 1;
        }
    };

    match ops::ensure_worktree(branch).await {
        Ok(outcome) => {
            // Post-create hook failures are non-fatal, matching TUI semantics.
            for err in &outcome.hook_errors {
                eprintln!("Warning: post-create hook failed: {err}");
            }
            println!("{}", outcome.path);
            0
        }
        Err(e) => {
            eprintln!("Error: {e}");
            1
        }
    }
}

/// Implements `gct ls [worktrees|branches]`: print a plain-text, pipe-friendly
/// listing. Worktrees are printed tab-separated (`branch<TAB>path`) so a single
/// result is never a lone directory path the shell wrapper would cd into.
async fn run_ls(subject: Option<&str>) -> i32 {
    if run_git(&["rev-parse", "--git-dir"]).await.is_err() {
        eprintln!("Error: not a git repository.");
        return 1;
    }

    match subject.unwrap_or("worktrees") {
        "worktrees" | "wt" => {
            let output = match run_git(&["worktree", "list", "--porcelain"]).await {
                Ok(o) => o,
                Err(e) => {
                    eprintln!("Error: failed to list worktrees: {e}");
                    return 1;
                }
            };
            for line in format_worktree_list(&parse_worktrees(&output)) {
                println!("{line}");
            }
            0
        }
        "branches" | "branch" => {
            let branches = match ops::list_branches().await {
                Ok(b) => b,
                Err(e) => {
                    eprintln!("Error: failed to list branches: {e}");
                    return 1;
                }
            };
            for b in branches {
                println!("{}", b.name);
            }
            0
        }
        other => {
            eprintln!("Error: unknown subject '{other}'. Use: worktrees | branches");
            1
        }
    }
}

/// Format a parsed worktree list as `branch<TAB>path` lines (detached → `(detached)`).
/// Pure helper so it can be unit-tested without git.
fn format_worktree_list(worktrees: &[crate::git::types::Worktree]) -> Vec<String> {
    worktrees
        .iter()
        .map(|wt| {
            let label = wt.branch.as_deref().unwrap_or("(detached)");
            format!("{label}\t{}", wt.path)
        })
        .collect()
}

/// A branch eligible for pruning, paired with its worktree path (if any).
struct PruneCandidate {
    branch: String,
    wt_path: Option<String>,
}

/// Select prunable branches: merged, not current, not protected. Pure helper.
fn prune_candidates(
    branches: &[crate::git::types::Branch],
    worktrees: &[crate::git::types::Worktree],
    protected: &[String],
) -> Vec<PruneCandidate> {
    branches
        .iter()
        .filter(|b| b.is_merged && !b.is_current && !protected.iter().any(|p| p == &b.name))
        .map(|b| PruneCandidate {
            branch: b.name.clone(),
            wt_path: ops::worktree_path_for_branch(worktrees, &b.name),
        })
        .collect()
}

/// Implements `gct prune [--dry-run] [--yes] [--force]`: delete merged branches (and
/// their worktrees), mirroring the TUI `a`+`d` cleanup. Safe by default — without
/// `--yes` it only lists what would be deleted. Never prints a lone directory path.
async fn run_prune(flags: &[String]) -> i32 {
    let yes = flags.iter().any(|f| f == "--yes" || f == "-y");
    let force = flags.iter().any(|f| f == "--force" || f == "-f");
    let dry_run = flags.iter().any(|f| f == "--dry-run") || !yes;
    if let Some(bad) = flags
        .iter()
        .find(|f| !matches!(f.as_str(), "--yes" | "-y" | "--force" | "-f" | "--dry-run"))
    {
        eprintln!("Error: unknown flag '{bad}'. Use: --dry-run | --yes | --force");
        return 1;
    }

    if run_git(&["rev-parse", "--git-dir"]).await.is_err() {
        eprintln!("Error: not a git repository.");
        return 1;
    }

    let branches = match ops::list_branches().await {
        Ok(b) => b,
        Err(e) => {
            eprintln!("Error: failed to list branches: {e}");
            return 1;
        }
    };
    let worktrees = run_git(&["worktree", "list", "--porcelain"])
        .await
        .map(|o| parse_worktrees(&o))
        .unwrap_or_default();
    let cfg = config::load_config();
    let candidates = prune_candidates(&branches, &worktrees, &cfg.protected_branches);

    if candidates.is_empty() {
        println!("No merged branches to prune.");
        return 0;
    }

    if dry_run {
        println!("Would delete {} merged branch(es):", candidates.len());
        for c in &candidates {
            match &c.wt_path {
                Some(p) => println!("  {} (worktree: {p})", c.branch),
                None => println!("  {}", c.branch),
            }
        }
        println!("Re-run with --yes to delete.");
        return 0;
    }

    let mut deleted_branches = 0usize;
    let mut removed_worktrees = 0usize;
    let mut failures: Vec<String> = Vec::new();

    for c in &candidates {
        // Remove the worktree first (a branch checked out in a worktree can't be deleted).
        if let Some(path) = &c.wt_path {
            let args: Vec<&str> = if force {
                vec!["worktree", "remove", "--force", path]
            } else {
                vec!["worktree", "remove", path]
            };
            match run_git(&args).await {
                Ok(_) => removed_worktrees += 1,
                Err(e) => {
                    failures.push(format!("{}: {}", c.branch, first_line(&e.to_string())));
                    continue;
                }
            }
        }
        let del_flag = if force { "-D" } else { "-d" };
        match run_git(&["branch", del_flag, &c.branch]).await {
            Ok(_) => deleted_branches += 1,
            Err(e) => failures.push(format!("{}: {}", c.branch, first_line(&e.to_string()))),
        }
    }

    let parts = bulk_success_parts(deleted_branches, removed_worktrees);
    if failures.is_empty() {
        println!("Deleted {}", parts.join(", "));
        0
    } else {
        if !parts.is_empty() {
            println!("Deleted {}", parts.join(", "));
        }
        eprintln!("Failed: {}", failures.join("; "));
        1
    }
}

/// First line of a (possibly multi-line) error string, for compact messages.
fn first_line(s: &str) -> String {
    s.lines().next().unwrap_or(s).to_string()
}

async fn run(
    terminal: &mut Terminal<CrosstermBackend<std::fs::File>>,
    config: config::Config,
    config_warnings: Vec<String>,
    verbose: bool,
) -> (anyhow::Result<()>, Option<String>) {
    let mut app = App::new(config);
    app.verbose = verbose;
    let mut events = EventHandler::new(Duration::from_millis(80));
    // Unbounded by design: the receiver drains fully every loop iteration,
    // and senders are spawned tasks that must never block on the UI. The
    // producer count — not the channel — is the real resource bound, and it
    // is capped by `App::inflight` dedup plus the selection-fetch debounce.
    let (tx, mut rx) = mpsc::unbounded_channel::<AsyncResult>();
    let repo_info = startup(&mut app, config_warnings, &tx).await;
    let mut tasks = RunState { tx, repo_info };

    loop {
        if let Err(e) = terminal.draw(|frame| ui::draw(frame, &mut app)) {
            return (Err(e.into()), None);
        }

        match events.next().await {
            Some(Event::Key(key)) if key.kind == KeyEventKind::Press => {
                app.handle_key(key);
            }
            Some(Event::Resize(_, _)) => {}
            Some(Event::Tick) => {
                app.tick();
            }
            Some(Event::Key(_)) => {}
            None => break,
        }

        // Receive completed background results (non-blocking)
        while let Ok(result) = rx.try_recv() {
            handle_result(&mut app, result);
        }

        // Dispatch commands staged by key handling above and by the
        // async-result handling just before this point. Snapshot drain:
        // commands pushed while dispatching (e.g. reload re-arms) land on
        // the fresh queue and run next iteration — no busy re-loop.
        let mut queued = app.take_commands();
        while let Some(cmd) = queued.pop_front() {
            dispatch_command(&mut app, cmd, &mut tasks);
        }

        if app.should_quit {
            break;
        }
    }

    let cd_path = app.cd_path.clone();
    (Ok(()), cd_path)
}

/// Loop-local state of `run()`: the task-result channel sender and the
/// active repo's identity. (Inflight-dedup bookkeeping lives on
/// `App::inflight` so key handling and dispatch share one tracker.)
struct RunState {
    tx: mpsc::UnboundedSender<AsyncResult>,
    repo_info: Option<crate::git::types::RepoId>,
}

/// One-time startup: surface config warnings, detect the active repo, run
/// the fast local loads (blocking), and spawn the slow network loads.
/// Returns the active repo's identity, if one could be detected.
async fn startup(
    app: &mut App,
    config_warnings: Vec<String>,
    tx: &mpsc::UnboundedSender<AsyncResult>,
) -> Option<crate::git::types::RepoId> {
    // Config problems were already printed to stderr, but that's invisible
    // once the TUI takes over — surface them in-app too.
    if let Some(first) = config_warnings.first() {
        let more = config_warnings.len() - 1;
        let suffix = if more > 0 {
            format!(" (+{more} more)")
        } else {
            String::new()
        };
        app.notification = Some(Notification::error(format!(
            "Config warning: {first}{suffix}"
        )));
        app.verbose_errors
            .extend(config_warnings.iter().map(|w| format!("config: {w}")));
    }
    // Global (home-dir) config layers, used to resolve a per-target-repo
    // effective config for cross-repo worktree operations.
    app.cross_repo.global_layers = config::load_global_layers();

    let active_root = run_git(&["rev-parse", "--show-toplevel"])
        .await
        .ok()
        .map(|s| std::path::PathBuf::from(s.trim()));
    let active_id = run_git(&["remote", "get-url", "origin"])
        .await
        .ok()
        .and_then(|url| extract_repo_info(url.trim()));
    app.cross_repo.active_repo = active_id.clone();
    app.cross_repo.clone_root = match (&active_root, &active_id) {
        (Some(p), Some(id)) => infer_clone_root(p, id),
        _ => None,
    }
    .or_else(|| app.config.workspace.clone_root_expanded());

    // Seed repos with the active repo immediately (local_path and resolved flag known now).
    if let Some(id) = app.cross_repo.active_repo.clone() {
        app.cross_repo
            .repos
            .entry(id.clone())
            .or_insert_with(|| crate::git::types::RepoMeta {
                local_path: active_root.clone(),
                local_path_resolved: true,
            });
    }

    // Phase 1: Fast local loads (blocking, ~170ms)
    if let Ok(output) = run_git(LOG_ARGS).await {
        app.commits = parse_log(&output);
    }
    if let Ok(output) = run_git(&["worktree", "list", "--porcelain"]).await {
        app.worktrees = parse_worktrees(&output);
    }
    load_branches(app).await;
    let repo_info = active_id;
    app.rebuild_entries();
    app.request_details_for_selection();

    // Phase 2: Slow network loads (background, non-blocking)
    let gh_hostname = repo_info.as_ref().and_then(|r| r.host.clone());

    let tx_user = tx.clone();
    let hostname_for_user = gh_hostname.clone();
    tokio::spawn(async move {
        let mut args = vec![
            "api",
            "graphql",
            "-f",
            "query={viewer{login}}",
            "--jq",
            ".data.viewer.login",
        ];
        if let Some(ref h) = hostname_for_user {
            args.push("--hostname");
            args.push(h);
        }
        match run_gh(&args).await {
            Ok(user) => {
                let _ = tx_user.send(AsyncResult::UserLogin(user.trim().to_string()));
            }
            Err(e) => {
                let _ = tx_user.send(AsyncResult::UserLoginError(format!(
                    "Failed to detect GitHub user: {e}"
                )));
            }
        }
    });

    // Fetch Local PRs via GraphQL (startup default view)
    if let Some(ref info) = repo_info {
        let tx_local = tx.clone();
        let branch_names: Vec<String> = app.branches.iter().map(|b| b.name.clone()).collect();
        let owner = info.owner.clone();
        let repo = info.name.clone();
        let hostname = info.host.clone();
        tokio::spawn(async move {
            let (prs, errors) =
                data::fetch_local_prs(&branch_names, &owner, &repo, hostname.as_deref()).await;
            let _ = tx_local.send(AsyncResult::LocalPrList(prs, errors));
        });
    } else {
        // No repo info available (no origin remote or unsupported URL format)
        app.local_prs_loaded = true;
    }

    repo_info
}

/// Store a fetched PR list for `filter` and, when that filter is the one
/// currently displayed, rebuild the entry list around it.
fn apply_pr_list(
    app: &mut App,
    filter: MainFilter,
    mut prs: Vec<PullRequest>,
    errors: Vec<String>,
) {
    if filter == MainFilter::ReviewRequested {
        for pr in &mut prs {
            pr.review_status = Some(compute_review_status(pr, &app.gh_user));
        }
    }
    report_fetch_errors(app, "PR fetch failed", errors);
    seed_repos_from_prs(&mut app.cross_repo.repos, &prs);
    match filter {
        MainFilter::Local => {
            app.local_prs = prs;
            app.local_prs_loaded = true;
        }
        MainFilter::MyPr => {
            app.my_prs = prs;
            app.my_prs_loaded = true;
        }
        MainFilter::ReviewRequested => {
            app.review_prs = prs;
            app.review_prs_loaded = true;
        }
    }
    if app.main_filter == filter {
        app.rebuild_entries_and_clamp();
        app.request_details_for_selection();
    }
}

/// Handle one completed background-task result.
fn handle_result(app: &mut App, result: AsyncResult) {
    match result {
        AsyncResult::PrDetail { repo_id, detail } => {
            let key = (repo_id.clone(), detail.number);
            app.inflight.pr_detail.remove(&key);
            app.pr_detail_cache.insert(key, detail);
        }
        AsyncResult::PrDetailError {
            repo_id,
            number,
            error,
        } => {
            app.inflight.pr_detail.remove(&(repo_id, number));
            app.notification = Some(Notification::error(format!("Failed to load PR #{number}")));
            app.record_error(error);
        }
        AsyncResult::GitStatus { wt_path, status } => {
            app.inflight.git_status.remove(&wt_path);
            if let Some(entry) = app
                .entries
                .iter_mut()
                .find(|e| e.worktree_path() == Some(wt_path.as_str()))
            {
                entry.git_status = Some(status);
            }
        }
        AsyncResult::GitStatusError(wt_path) => {
            app.inflight.git_status.remove(&wt_path);
            let msg = format!("git status failed for {wt_path}");
            app.notification = Some(Notification::error(msg.clone()));
            app.record_error(msg);
        }
        AsyncResult::UserLogin(user) => {
            app.gh_user = user;
            // Recompute review status now that we know the user
            for pr in &mut app.review_prs {
                pr.review_status = Some(compute_review_status(pr, &app.gh_user));
            }
            if app.main_filter == MainFilter::ReviewRequested {
                app.rebuild_entries();
            }
        }
        AsyncResult::UserLoginError(error_msg) => {
            app.gh_user_load_failed = true;
            app.notification = Some(Notification::error(
                "Failed to load GitHub user — is `gh` authenticated?".to_string(),
            ));
            app.record_error(error_msg);
        }
        AsyncResult::LocalPrList(prs, errors) => {
            apply_pr_list(app, MainFilter::Local, prs, errors);
        }
        AsyncResult::MyPrList(prs, errors) => {
            apply_pr_list(app, MainFilter::MyPr, prs, errors);
        }
        AsyncResult::ReviewPrList(prs, errors) => {
            apply_pr_list(app, MainFilter::ReviewRequested, prs, errors);
        }
        AsyncResult::WtCreated {
            wt_path,
            copy_errors,
            target_repo,
        } => {
            app.inflight.worktrees.remove(&wt_path);
            if copy_errors.is_empty() {
                app.notification = Some(Notification::success(format!(
                    "Worktree created: {wt_path}"
                )));
            } else {
                app.notification = Some(Notification::success(format!(
                    "Worktree created: {wt_path} (copy errors: {})",
                    copy_errors.len()
                )));
                for e in copy_errors {
                    app.record_error(e);
                }
            }
            app.push_command(Command::ReloadBranches);
            // Cross-repo: invalidate worktree-list cache for target so next selection re-fetches.
            if let Some(repo_id) = target_repo {
                app.cross_repo.wt_lists_per_repo.remove(&repo_id);
            }
            if app.confirm_dialog.is_none() {
                app.confirm_dialog = Some(crate::app::PendingConfirm {
                    dialog: crate::ui::confirm_dialog::ConfirmDialog::new(
                        "Move to Worktree",
                        format!(
                            "Move into the new worktree?\n(Requires shell integration — see README.)\n{wt_path}"
                        ),
                    ),
                    on_confirm: Command::CdAndQuit(wt_path),
                });
            }
        }
        AsyncResult::WtCreateError { wt_path, message } => {
            app.inflight.worktrees.remove(&wt_path);
            app.notification = Some(Notification::error(message));
        }
        AsyncResult::OpStarted { op_id, label } => {
            app.progress.insert(op_id, OpProgress::new(label));
        }
        AsyncResult::OpStepBegin {
            op_id,
            step,
            command,
        } => {
            app.progress.update_step(op_id, step, command);
        }
        AsyncResult::OpFinished {
            op_id,
            success,
            error,
        } => {
            app.progress.finish(op_id, success, error);
        }
        AsyncResult::OpAllDone {
            branches_deleted,
            worktrees_removed,
            failures,
            wt_paths_claimed,
        } => {
            for path in &wt_paths_claimed {
                app.inflight.worktrees.remove(path);
            }
            app.progress.sweep_unfinished();
            let success_parts = bulk_success_parts(branches_deleted.len(), worktrees_removed.len());
            if failures.is_empty() {
                let msg = if success_parts.is_empty() {
                    "Nothing to delete".to_string()
                } else {
                    format!("Deleted {}", success_parts.join(", "))
                };
                app.notification = Some(Notification::success(msg));
            } else {
                let short: Vec<String> = failures
                    .iter()
                    .map(|e| e.lines().next().unwrap_or(e).to_string())
                    .collect();
                let summary = if success_parts.is_empty() {
                    format!("Bulk delete failed: {}", short.join("; "))
                } else {
                    format!(
                        "Bulk delete: {}; failed: {}",
                        success_parts.join(", "),
                        short.join("; ")
                    )
                };
                app.notification = Some(Notification::error(summary));
                for err in failures {
                    app.record_error(err);
                }
            }
            app.progress.clear();
            app.quit_pressed_during_progress = false;
            app.push_command(Command::ReloadBranches);
        }
        AsyncResult::WtForceDecisionRequested { path, reason } => {
            // Plain remove failed; ask the user whether to force.
            app.confirm_dialog = Some(crate::app::PendingConfirm {
                dialog: crate::ui::confirm_dialog::ConfirmDialog::new(
                    "Force Delete Worktree",
                    format!("{reason}\nForce remove {path}?"),
                ),
                on_confirm: Command::ForceDeleteWorktree(path),
            });
        }
        AsyncResult::WtListLoaded { repo_id, list } => {
            app.inflight.wt_lists.remove(&repo_id);
            app.cross_repo.wt_lists_per_repo.insert(repo_id, list);
            app.rebuild_entries();
            app.snap_scroll_to_entry();
        }
        AsyncResult::BranchesReloaded(data) => {
            app.inflight.branches_reload = false;
            if let Some(branches) = data.branches {
                app.branches = branches;
            }
            if let Some(worktrees) = data.worktrees {
                app.worktrees = worktrees;
            }
            report_fetch_errors(app, "Branch reload failed", data.errors);
            app.rebuild_entries_and_clamp();
            app.snap_scroll_to_entry();
        }
        AsyncResult::CommitsReloaded(commits) => {
            app.inflight.commits_reload = false;
            if let Some(commits) = commits {
                app.commits = commits;
            }
        }
        AsyncResult::BranchCreated { name, source } => {
            app.notification = Some(Notification::success(format!(
                "Created branch '{name}' from '{source}'"
            )));
            app.push_command(Command::ReloadBranches);
        }
        AsyncResult::BranchCreateError(err_str) => {
            let short = err_str.lines().next().unwrap_or(&err_str).to_string();
            app.notification = Some(Notification::error(short));
            app.record_error(err_str);
        }
    }
}

/// Dispatch one queued `Command`: spawn the async task (or perform the
/// synchronous action) it describes.
fn dispatch_command(app: &mut App, cmd: Command, tasks: &mut RunState) {
    match cmd {
        // Delete worktree (single-item, no auto-force fallback).
        Command::DeleteWorktree(path) => {
            app.inflight.worktrees.insert(path.clone());
            let op_id = app.progress.allocate_ids(1).start;
            let label = wt_label_for(&path);
            let claimed = vec![path.clone()];
            let tx_c = tasks.tx.clone();
            tokio::spawn(async move {
                let result = run_delete_op(
                    op_id,
                    label.clone(),
                    Some(path.clone()),
                    None,
                    false,
                    DeleteMode::PlainOnly,
                    tx_c.clone(),
                )
                .await;

                if let Some(failure) = result.failure {
                    // Plain failed — drain the progress panel without raising a
                    // failure notification (it would flash before the force-confirm
                    // dialog appears). The actual outcome is communicated by the
                    // subsequent force attempt or by the user dismissing the dialog.
                    let _ = tx_c.send(AsyncResult::OpAllDone {
                        branches_deleted: vec![],
                        worktrees_removed: vec![],
                        failures: vec![],
                        wt_paths_claimed: claimed,
                    });
                    let reason = failure
                        .lines()
                        .next()
                        .unwrap_or("unknown error")
                        .to_string();
                    let _ = tx_c.send(AsyncResult::WtForceDecisionRequested { path, reason });
                } else {
                    let mut wts = vec![];
                    if result.worktree_removed
                        && let Some(p) = result.wt_path
                    {
                        wts.push(p);
                    }
                    let _ = tx_c.send(AsyncResult::OpAllDone {
                        branches_deleted: vec![],
                        worktrees_removed: wts,
                        failures: vec![],
                        wt_paths_claimed: claimed,
                    });
                }
            });
        }

        // Force delete worktree after confirmation (single-item, force only).
        Command::ForceDeleteWorktree(path) => {
            app.inflight.worktrees.insert(path.clone());
            let op_id = app.progress.allocate_ids(1).start;
            let label = wt_label_for(&path);
            let claimed = vec![path.clone()];
            let tx_c = tasks.tx.clone();
            tokio::spawn(async move {
                let result = run_delete_op(
                    op_id,
                    label,
                    Some(path),
                    None,
                    false,
                    DeleteMode::ForceOnly,
                    tx_c.clone(),
                )
                .await;
                let mut wts = vec![];
                let mut failures = vec![];
                if let Some(f) = result.failure {
                    failures.push(f);
                } else if let Some(p) = result.wt_path
                    && result.worktree_removed
                {
                    wts.push(p);
                }
                let _ = tx_c.send(AsyncResult::OpAllDone {
                    branches_deleted: vec![],
                    worktrees_removed: wts,
                    failures,
                    wt_paths_claimed: claimed,
                });
            });
        }

        // Create worktree (async, non-blocking)
        Command::CreateWorktree(req_repo_id, branch_name) => {
            // Resolve target repo (active or cross-repo) and its root path.
            let entry = app
                .entries
                .iter()
                .find(|e| e.repo_id == req_repo_id && e.name == branch_name)
                .cloned();
            let Some(entry) = entry else {
                return;
            };
            let target_repo = entry.repo_id.clone();
            let active_repo = app.cross_repo.active_repo.clone();
            let is_active = active_repo.as_ref() == Some(&target_repo);

            // The active repo's root is already cached in `app.cross_repo.repos` (seeded
            // at startup), so this never needs a blocking `rev-parse` call —
            // cross-repo targets rely on the same cached `local_path` too.
            let target_root: std::path::PathBuf = match app
                .cross_repo
                .repos
                .get(&target_repo)
                .and_then(|m| m.local_path.clone())
            {
                Some(p) => p,
                None if is_active => std::env::current_dir().unwrap_or_default(),
                None => {
                    app.notification =
                        Some(Notification::error(format!("{target_repo} not cloned")));
                    return;
                }
            };

            // Active repo uses the launch config; cross-repo resolves the
            // target repo's own config (global layers + <target_root>/.gct.toml).
            let cfg = if is_active {
                app.config.clone()
            } else {
                config::resolve_config(&app.cross_repo.global_layers, Some(&target_root))
            };

            let wt_path = cfg.worktree_path_for(&target_root, &entry.repo_id.name, &branch_name);
            app.inflight.worktrees.insert(wt_path.clone());
            if let Some(parent) = std::path::Path::new(&wt_path).parent() {
                let _ = std::fs::create_dir_all(parent);
            }

            let is_active_with_local =
                is_active && app.branches.iter().any(|b| b.name == branch_name);
            let post_create = cfg.worktree.post_create.clone();
            let target_repo_for_send = if is_active {
                None
            } else {
                Some(target_repo.clone())
            };
            let tx = tasks.tx.clone();
            let wt_path_arg = wt_path.clone();
            let branch_arg = branch_name.clone();
            let target_root_arg = target_root.clone();

            tokio::spawn(async move {
                let result = if is_active_with_local {
                    run_git_in(
                        &target_root_arg,
                        &["worktree", "add", &wt_path_arg, &branch_arg],
                    )
                    .await
                } else {
                    match run_git_in(&target_root_arg, &["fetch", "origin", &branch_arg]).await {
                        Ok(_) => {
                            run_git_in(
                                &target_root_arg,
                                &["worktree", "add", &wt_path_arg, &branch_arg],
                            )
                            .await
                        }
                        Err(e) => Err(e),
                    }
                };
                match result {
                    Ok(_) => {
                        let copy_errors = config::run_post_create(
                            &post_create,
                            &target_root_arg,
                            std::path::Path::new(&wt_path_arg),
                        );
                        let _ = tx.send(AsyncResult::WtCreated {
                            wt_path: wt_path_arg,
                            copy_errors,
                            target_repo: target_repo_for_send,
                        });
                    }
                    Err(e) => {
                        let _ = tx.send(AsyncResult::WtCreateError {
                            wt_path: wt_path_arg,
                            message: format!("Failed to create worktree: {e}"),
                        });
                    }
                }
            });
        }

        // Delete selected entries (branches + optional worktrees) in parallel.
        Command::DeleteBranches(selected) => {
            // The op is starting — clear the sidebar checkboxes now
            // (a declined dialog keeps the selection intact).
            app.branch_selected.clear();
            struct Work {
                name: String,
                wt_path: Option<String>,
                has_local_branch: bool,
            }
            let mut work: Vec<Work> = Vec::with_capacity(selected.len());
            let mut wt_paths_claimed: Vec<String> = Vec::new();
            let active_repo = app.cross_repo.active_repo.clone();
            for name in selected {
                let Some(entry) = app
                    .entries
                    .iter()
                    .find(|e| e.name == name && active_repo.as_ref() == Some(&e.repo_id))
                else {
                    continue;
                };
                if entry.is_current() || app.is_protected_branch(&entry.name) {
                    continue;
                }
                let wt_path = entry.worktree_path().map(str::to_string);
                if let Some(ref p) = wt_path
                    && app.inflight.worktrees.contains(p)
                {
                    continue;
                }
                if let Some(ref p) = wt_path {
                    app.inflight.worktrees.insert(p.clone());
                    wt_paths_claimed.push(p.clone());
                }
                work.push(Work {
                    name: entry.name.clone(),
                    wt_path,
                    has_local_branch: entry.local_branch.is_some(),
                });
            }

            if work.is_empty() {
                app.notification = Some(Notification::error("Nothing to delete".to_string()));
            } else {
                let ids: Vec<u64> = app.progress.allocate_ids(work.len()).collect();
                let mut set: JoinSet<DeleteOpResult> = JoinSet::new();
                for (op_id, w) in ids.into_iter().zip(work) {
                    let tx_c = tasks.tx.clone();
                    set.spawn(run_delete_op(
                        op_id,
                        w.name.clone(),
                        w.wt_path,
                        Some(w.name),
                        w.has_local_branch,
                        DeleteMode::TryThenForce,
                        tx_c,
                    ));
                }

                let tx_done = tasks.tx.clone();
                let claimed = wt_paths_claimed;
                tokio::spawn(async move {
                    let mut branches: Vec<String> = Vec::new();
                    let mut wts: Vec<String> = Vec::new();
                    let mut failures: Vec<String> = Vec::new();
                    while let Some(res) = set.join_next().await {
                        match res {
                            Ok(r) => r.collect_into(&mut branches, &mut wts, &mut failures),
                            Err(e) => failures.push(format!("task panic: {e}")),
                        }
                    }
                    let _ = tx_done.send(AsyncResult::OpAllDone {
                        branches_deleted: branches,
                        worktrees_removed: wts,
                        failures,
                        wt_paths_claimed: claimed,
                    });
                });
            }
        }

        // Create a new branch from the selected branch (non-blocking)
        Command::CreateBranch { source, name } => {
            let tx = tasks.tx.clone();
            tokio::spawn(async move {
                match run_git(&["branch", "--", &name, &source]).await {
                    Ok(_) => {
                        let _ = tx.send(AsyncResult::BranchCreated { name, source });
                    }
                    Err(e) => {
                        let _ = tx.send(AsyncResult::BranchCreateError(e.to_string()));
                    }
                }
            });
        }

        // Open PR in browser (non-blocking; result is ignored either way)
        Command::OpenPrInBrowser(repo_id, pr_number) => {
            tokio::spawn(async move {
                // `gh pr view --web` doesn't accept --hostname; embed host into --repo.
                let repo_arg = repo_id.repo_arg();
                let num = pr_number.to_string();
                let args = vec!["pr", "view", &num, "--web", "--repo", repo_arg.as_str()];
                let _ = run_gh(&args).await;
            });
        }

        // Copy branch name to clipboard (synchronous)
        Command::CopyBranchName(name) => {
            copy_to_clipboard(&name);
        }

        // Quit and hand the path to the shell-integration `cd`
        Command::CdAndQuit(path) => {
            app.cd_path = Some(path);
            app.should_quit = true;
        }

        // Spawn PR detail load in background (deduplicated)
        Command::FetchPrDetail(repo_id, number) => {
            if app.inflight.pr_detail.contains(&(repo_id.clone(), number)) {
                return;
            }
            app.inflight.pr_detail.insert((repo_id.clone(), number));
            let tx = tasks.tx.clone();
            // `gh pr view` doesn't accept --hostname; the host (if any) is
            // embedded into --repo as `[HOST/]OWNER/REPO`.
            let repo_arg = repo_id.repo_arg();
            tokio::spawn(async move {
                let num_str = number.to_string();
                let args = vec![
                    "pr",
                    "view",
                    &num_str,
                    "--repo",
                    repo_arg.as_str(),
                    "--json",
                    "number,body,additions,deletions",
                ];
                let result = run_gh(&args).await;
                match result {
                    Ok(output) => match serde_json::from_str::<PrDetail>(&output) {
                        Ok(detail) => {
                            let _ = tx.send(AsyncResult::PrDetail { repo_id, detail });
                        }
                        Err(e) => {
                            let _ = tx.send(AsyncResult::PrDetailError {
                                repo_id,
                                number,
                                error: format!("PR #{number} parse error: {e}"),
                            });
                        }
                    },
                    Err(e) => {
                        let _ = tx.send(AsyncResult::PrDetailError {
                            repo_id,
                            number,
                            error: format!("PR #{number} fetch failed: {e}"),
                        });
                    }
                }
            });
        }

        // Spawn cross-repo worktree list load in background (deduplicated)
        Command::LoadWorktreeList(repo_id) => {
            if app.inflight.wt_lists.contains(&repo_id) {
                return;
            }
            let Some(path) = app
                .cross_repo
                .repos
                .get(&repo_id)
                .and_then(|m| m.local_path.clone())
            else {
                return;
            };
            app.inflight.wt_lists.insert(repo_id.clone());
            let tx_c = tasks.tx.clone();
            let repo_id_c = repo_id.clone();
            tokio::spawn(async move {
                match run_git_in(&path, &["worktree", "list", "--porcelain"]).await {
                    Ok(out) => {
                        let list = crate::git::parser::parse_worktrees(&out);
                        let _ = tx_c.send(AsyncResult::WtListLoaded {
                            repo_id: repo_id_c,
                            list,
                        });
                    }
                    Err(_) => {
                        let _ = tx_c.send(AsyncResult::WtListLoaded {
                            repo_id: repo_id_c,
                            list: Vec::new(),
                        });
                    }
                }
            });
        }

        // Reload branches/worktrees in background (deduplicated).
        // Triggered by `r` in Main view, and also re-armed after
        // worktree/branch mutations that invalidate the cached entries.
        Command::ReloadBranches => {
            if app.inflight.branches_reload {
                // A reload is already running — retry next iteration so a
                // request issued mid-reload still yields a fresh fetch.
                app.push_command(Command::ReloadBranches);
                return;
            }
            app.inflight.branches_reload = true;
            let tx = tasks.tx.clone();
            tokio::spawn(async move {
                let data = fetch_branches_and_worktrees().await;
                let _ = tx.send(AsyncResult::BranchesReloaded(data));
            });
        }

        // Reload commits on `r` refresh from Log view (deduplicated)
        Command::ReloadCommits => {
            if app.inflight.commits_reload {
                app.push_command(Command::ReloadCommits);
                return;
            }
            app.inflight.commits_reload = true;
            let tx = tasks.tx.clone();
            tokio::spawn(async move {
                // Always send, even on failure, so `Inflight.commits_reload`
                // is reliably cleared instead of getting stuck forever.
                let commits = run_git(LOG_ARGS)
                    .await
                    .ok()
                    .map(|output| parse_log(&output));
                let _ = tx.send(AsyncResult::CommitsReloaded(commits));
            });
        }

        // Spawn git status load in background (deduplicated)
        Command::LoadGitStatus(wt_path) => {
            if app.inflight.git_status.contains(&wt_path) {
                return;
            }
            app.inflight.git_status.insert(wt_path.clone());
            let tx = tasks.tx.clone();
            tokio::spawn(async move {
                if let Some(status) = data::load_git_status(&wt_path).await {
                    let _ = tx.send(AsyncResult::GitStatus { wt_path, status });
                } else {
                    let _ = tx.send(AsyncResult::GitStatusError(wt_path));
                }
            });
        }

        // Spawn PR fetch for view switch (non-blocking)
        Command::FetchPrs(filter) => {
            let tx = tasks.tx.clone();
            match filter {
                MainFilter::Local => {
                    if let Some(ref info) = tasks.repo_info {
                        let branch_names: Vec<String> =
                            app.branches.iter().map(|b| b.name.clone()).collect();
                        let owner = info.owner.clone();
                        let repo = info.name.clone();
                        let hostname = info.host.clone();
                        tokio::spawn(async move {
                            let (prs, errors) = data::fetch_local_prs(
                                &branch_names,
                                &owner,
                                &repo,
                                hostname.as_deref(),
                            )
                            .await;
                            let _ = tx.send(AsyncResult::LocalPrList(prs, errors));
                        });
                    }
                }
                MainFilter::MyPr => {
                    let show_merged = app.show_merged;
                    let hosts = app.known_hosts();
                    tokio::spawn(async move {
                        let (prs, errors) = data::fetch_my_prs(show_merged, &hosts).await;
                        let _ = tx.send(AsyncResult::MyPrList(prs, errors));
                    });
                }
                MainFilter::ReviewRequested => {
                    // Defer until gh_user is known — GitHub's `review-requested:@me`
                    // search expands to team memberships, and the post-fetch filter
                    // in fetch_review_prs only runs when gh_user is non-empty. Without
                    // this guard, switching to Review right after startup can show
                    // team PRs even in me-only mode. If the user-login fetch failed,
                    // proceed anyway — no point in spinning forever.
                    if app.gh_user.is_empty()
                        && !app.include_team_reviews
                        && !app.gh_user_load_failed
                    {
                        app.push_command(Command::FetchPrs(MainFilter::ReviewRequested));
                    } else {
                        let show_merged = app.show_merged;
                        let include_team = app.include_team_reviews;
                        let gh_user = app.gh_user.clone();
                        let hosts = app.known_hosts();
                        tokio::spawn(async move {
                            let (prs, errors) =
                                data::fetch_review_prs(show_merged, include_team, &gh_user, &hosts)
                                    .await;
                            let _ = tx.send(AsyncResult::ReviewPrList(prs, errors));
                        });
                    }
                }
            }
        }
    }
}

/// Surface background fetch failures: a short error toast (first line of the
/// first failure, plus a count of the rest) and the full messages recorded
/// for the `--verbose` error list.
fn report_fetch_errors(app: &mut App, context: &str, errors: Vec<String>) {
    if errors.is_empty() {
        return;
    }
    let first = errors
        .first()
        .and_then(|e| e.lines().next())
        .unwrap_or("unknown error");
    let suffix = if errors.len() > 1 {
        format!(" (+{} more)", errors.len() - 1)
    } else {
        String::new()
    };
    let toast = format!("{context}: {first}{suffix}");
    app.notification = Some(Notification::error(toast));
    for e in errors {
        app.record_error(e);
    }
}

async fn load_branches(app: &mut App) {
    let branch_output = match run_git(&["branch", "-vv"]).await {
        Ok(output) => output,
        Err(e) => {
            report_fetch_errors(
                app,
                "Branch list failed",
                vec![format!("git branch -vv failed: {e}")],
            );
            return;
        }
    };
    let default_branch = detect_default_branch().await;
    let merged_output = match run_git(&["branch", "--merged", &default_branch]).await {
        Ok(output) => output,
        Err(e) => {
            // Non-fatal: only the merged-marker annotations are lost.
            app.record_error(format!("git branch --merged failed: {e}"));
            String::new()
        }
    };
    let base_hash = match run_git(&["rev-parse", &default_branch]).await {
        Ok(output) => output,
        Err(e) => {
            // Non-fatal: only the merged-marker annotations are lost.
            app.record_error(format!("git rev-parse {default_branch} failed: {e}"));
            String::new()
        }
    };
    app.branches = parse_branches(&branch_output, &merged_output, base_hash.trim());
}

async fn detect_default_branch() -> String {
    // Try remote HEAD symbolic ref (most reliable)
    if let Ok(output) = run_git(&["symbolic-ref", "refs/remotes/origin/HEAD"]).await
        && let Some(name) = output.trim().strip_prefix("refs/remotes/origin/")
    {
        return name.to_string();
    }
    // Fallback: try main, then master, then HEAD
    if run_git(&["rev-parse", "--verify", "main"]).await.is_ok() {
        return "main".to_string();
    }
    if run_git(&["rev-parse", "--verify", "master"]).await.is_ok() {
        return "master".to_string();
    }
    "HEAD".to_string()
}

fn print_help() {
    print!(
        "gct — Git Control Tower
A terminal TUI for Git/GitHub branch, PR, and worktree management.

USAGE:
    gct [OPTIONS]
    gct cd <BRANCH>
    gct wt <BRANCH>
    gct ls [worktrees|branches]
    gct prune [--dry-run] [--yes] [--force]
    gct mcp
    gct completions <SHELL>
    gct shell-init <SHELL>

OPTIONS:
    -h, --help        Print this help message and exit
    -v, --version     Print version and exit
        --verbose     Show collected error details in the detail pane

SUBCOMMANDS:
    cd <BRANCH>
        Print the worktree path for BRANCH and cd into it (requires shell
        integration — see `shell-init`). Exits non-zero without printing a
        path if no worktree exists for BRANCH.
        Example: gct cd feature/login

    wt <BRANCH>
        Reuse BRANCH's worktree or create one, then cd into it (requires shell
        integration). The create-side complement to `cd`. Applies worktree
        post-create hooks from config.
        Example: gct wt feature/login

    ls [worktrees|branches]
        Print a plain-text listing (default: worktrees as `branch<TAB>path`)
        for scripting, e.g. `gct cd \"$(gct ls | fzf | cut -f1)\"`.

    prune [--dry-run] [--yes] [--force]
        Delete merged branches and their worktrees (protected/current branches
        are skipped). Lists candidates only unless --yes is given. --force uses
        `worktree remove --force` and `branch -D`.

    mcp
        Run a Model Context Protocol server over stdio, exposing worktree
        tools (create_worktree, list_worktrees, list_branches) to MCP
        clients such as Claude Code. For client configuration, not
        interactive use. Does not require gh.

    completions <SHELL>
        Print a shell completion script. SHELL is one of: zsh, bash, fish.
        Example: eval \"$(gct completions zsh)\"

    shell-init <SHELL>
        Print shell wrapper code for `cd into worktree` support.
        SHELL is one of: zsh, bash, fish (default: zsh).
        Example: eval \"$(gct shell-init zsh)\"

Run `gct` with no arguments inside a git repository to launch the TUI.

CONFIG:
    .gct.toml (repo root) → ~/.config/gct/config.toml → ~/.gct.toml
"
    );
}

fn print_shell_init(shell: &str) {
    match shell {
        "zsh" | "bash" => {
            print!(
                r#"gct() {{
    local output
    output=$(command gct "$@")
    local exit_code=$?
    if [[ $exit_code -eq 0 && -n "$output" && -d "$output" ]]; then
        cd "$output" || return $?
    elif [[ -n "$output" ]]; then
        printf '%s\n' "$output"
    fi
    return $exit_code
}}
"#
            );
        }
        "fish" => {
            print!(
                r#"function gct
    set -l output (command gct $argv | string collect)
    set -l status_code $pipestatus[1]
    if test $status_code -eq 0 -a -n "$output" -a -d "$output"
        cd "$output"; or return $status_code
    else if test -n "$output"
        printf '%s\n' "$output"
    end
    return $status_code
end
"#
            );
        }
        _ => {
            eprintln!("Unsupported shell: {shell}. Supported: zsh, bash, fish");
            std::process::exit(1);
        }
    }
}

/// Print a shell completion script for `gct`. Completes subcommands, and for
/// `cd`/`wt` completes branch names via `gct ls branches`.
fn print_completions(shell: &str) {
    match shell {
        "bash" => {
            print!(
                r#"_gct() {{
    local cur prev
    cur="${{COMP_WORDS[COMP_CWORD]}}"
    prev="${{COMP_WORDS[COMP_CWORD-1]}}"
    if [ "$COMP_CWORD" -eq 1 ]; then
        COMPREPLY=($(compgen -W "cd wt ls prune mcp completions shell-init --help --version" -- "$cur"))
        return
    fi
    case "$prev" in
        cd|wt) COMPREPLY=($(compgen -W "$(command gct ls branches 2>/dev/null)" -- "$cur")) ;;
        ls) COMPREPLY=($(compgen -W "worktrees branches" -- "$cur")) ;;
        completions|shell-init) COMPREPLY=($(compgen -W "zsh bash fish" -- "$cur")) ;;
    esac
}}
complete -F _gct gct
"#
            );
        }
        "zsh" => {
            print!(
                r#"_gct() {{
    if (( CURRENT == 2 )); then
        compadd -- cd wt ls prune mcp completions shell-init --help --version
        return
    fi
    case "${{words[2]}}" in
        cd|wt) compadd -- ${{(f)"$(command gct ls branches 2>/dev/null)"}} ;;
        ls) compadd -- worktrees branches ;;
        completions|shell-init) compadd -- zsh bash fish ;;
    esac
}}
compdef _gct gct
"#
            );
        }
        "fish" => {
            print!(
                r#"complete -c gct -f
complete -c gct -n '__fish_use_subcommand' -a 'cd wt ls prune mcp completions shell-init'
complete -c gct -n '__fish_seen_subcommand_from cd wt' -a '(command gct ls branches 2>/dev/null)'
complete -c gct -n '__fish_seen_subcommand_from ls' -a 'worktrees branches'
complete -c gct -n '__fish_seen_subcommand_from completions shell-init' -a 'zsh bash fish'
"#
            );
        }
        _ => {
            eprintln!("Unsupported shell: {shell}. Supported: zsh, bash, fish");
            std::process::exit(1);
        }
    }
}

fn copy_to_clipboard(text: &str) {
    // Try platform-native clipboard commands first, fall back to OSC 52
    if copy_via_native_command(text).is_ok() {
        return;
    }
    copy_via_osc52(text);
}

fn copy_via_native_command(text: &str) -> std::io::Result<()> {
    use std::io::Write;

    #[cfg(target_os = "macos")]
    let mut child = std::process::Command::new("pbcopy")
        .stdin(std::process::Stdio::piped())
        .spawn()?;

    #[cfg(target_os = "linux")]
    let mut child = {
        std::process::Command::new("xclip")
            .args(["-selection", "clipboard"])
            .stdin(std::process::Stdio::piped())
            .spawn()
            .or_else(|_| {
                std::process::Command::new("xsel")
                    .args(["--clipboard", "--input"])
                    .stdin(std::process::Stdio::piped())
                    .spawn()
            })?
    };

    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        let _ = text;
        return Err(std::io::Error::other("no native clipboard command"));
    }

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    {
        if let Some(mut stdin) = child.stdin.take() {
            stdin.write_all(text.as_bytes())?;
        }
        let status = child.wait()?;
        if status.success() {
            Ok(())
        } else {
            Err(std::io::Error::other("clipboard command failed"))
        }
    }
}

fn copy_via_osc52(text: &str) {
    use base64::Engine;
    use std::io::Write;
    let encoded = base64::engine::general_purpose::STANDARD.encode(text);
    let osc52 = format!("\x1b]52;c;{encoded}\x07");
    // Write to /dev/tty to bypass shell function stdout capture
    if let Ok(mut tty) = std::fs::OpenOptions::new().write(true).open("/dev/tty") {
        let _ = tty.write_all(osc52.as_bytes());
        let _ = tty.flush();
    }
}

/// Seed `repos` map with stub entries for every repo referenced by `prs`.
/// Called after each PR list arrives from the network so that cross-repo
/// entries have a `RepoMeta` record even before a clone is resolved.
fn seed_repos_from_prs(
    repos: &mut std::collections::HashMap<crate::git::types::RepoId, crate::git::types::RepoMeta>,
    prs: &[PullRequest],
) {
    for pr in prs {
        repos
            .entry(pr.repo_id.clone())
            .or_insert_with(|| crate::git::types::RepoMeta {
                local_path: None,
                local_path_resolved: false,
            });
    }
}

fn compute_review_status(pr: &PullRequest, gh_user: &str) -> ReviewStatus {
    if gh_user.is_empty() {
        return ReviewStatus::NeedsReview;
    }
    for review in &pr.latest_reviews {
        if review.author == gh_user {
            return match review.state {
                ReviewState::Approved => ReviewStatus::Approved,
                ReviewState::ChangesRequested => ReviewStatus::ChangesRequested,
                ReviewState::Commented | ReviewState::Dismissed | ReviewState::Pending => {
                    ReviewStatus::Commented
                }
            };
        }
    }
    ReviewStatus::NeedsReview
}

/// Parse a git remote URL into a `RepoId` (owner, name, host).
fn extract_repo_info(remote_url: &str) -> Option<RepoId> {
    let hostname = extract_gh_hostname(remote_url)?;
    let gh_hostname = if hostname == "github.com" {
        None
    } else {
        Some(hostname)
    };

    // Extract org/repo path part
    let path_part = if let Some(rest) = remote_url.strip_prefix("git@")
        && !rest.starts_with("//")
    {
        // SCP-style: git@hostname:org/repo.git
        rest.split(':').nth(1).map(|s| s.to_string())
    } else if let Some(rest) = remote_url.strip_prefix("ssh://") {
        // ssh://git@hostname/org/repo.git or ssh://git@hostname:port/org/repo.git
        let after_user = rest.split('@').next_back()?;
        // Skip hostname (and optional :port), then take the rest as path
        let (_, path) = after_user.split_once('/')?;
        Some(path.to_string())
    } else if let Some(rest) = remote_url
        .strip_prefix("https://")
        .or_else(|| remote_url.strip_prefix("http://"))
    {
        // https://hostname/org/repo.git
        let parts: Vec<&str> = rest.splitn(2, '/').collect();
        parts.get(1).map(|s| s.to_string())
    } else {
        None
    }?;

    // Clean .git suffix and split into owner/repo
    let cleaned = path_part.trim_end_matches(".git");
    let parts: Vec<&str> = cleaned.splitn(2, '/').collect();
    if parts.len() != 2 || parts[0].is_empty() || parts[1].is_empty() {
        return None;
    }

    Some(RepoId {
        owner: parts[0].to_string(),
        name: parts[1].to_string(),
        host: gh_hostname,
    })
}

/// If `local_path` ends with `<…>/<host>/<owner>/<name>`, strip that suffix
/// and return the prefix. `RepoId.host == None` is treated as `github.com`
/// for the purpose of suffix matching.
fn infer_clone_root(
    local_path: &std::path::Path,
    repo_id: &crate::git::types::RepoId,
) -> Option<std::path::PathBuf> {
    let host = repo_id.host.as_deref().unwrap_or("github.com");
    let mut comps: Vec<&std::ffi::OsStr> = local_path.iter().collect();
    if comps.len() < 3 {
        return None;
    }
    let last = comps.pop()?;
    let owner = comps.pop()?;
    let host_seg = comps.pop()?;
    if last != std::ffi::OsStr::new(repo_id.name.as_str()) {
        return None;
    }
    if owner != std::ffi::OsStr::new(repo_id.owner.as_str()) {
        return None;
    }
    if host_seg != std::ffi::OsStr::new(host) {
        return None;
    }
    Some(comps.iter().collect::<std::path::PathBuf>())
}

/// Extract the hostname from a git remote URL.
/// Returns None for unrecognized formats.
fn extract_gh_hostname(remote_url: &str) -> Option<String> {
    // SCP-style SSH: git@hostname:org/repo.git
    if let Some(rest) = remote_url.strip_prefix("git@")
        && !rest.starts_with("//")
    {
        return rest.split(':').next().map(|s| s.to_string());
    }
    // SSH URL: ssh://git@hostname/org/repo.git or ssh://git@hostname:port/org/repo.git
    if let Some(rest) = remote_url.strip_prefix("ssh://") {
        let after_user = rest.split('@').next_back()?;
        return after_user
            .split('/')
            .next()
            .map(|s| s.split(':').next().unwrap_or(s).to_string());
    }
    // HTTP(S): https://hostname/org/repo.git or https://user@hostname:port/org/repo.git
    if let Some(rest) = remote_url
        .strip_prefix("https://")
        .or_else(|| remote_url.strip_prefix("http://"))
    {
        let authority = rest.split('/').next()?;
        let after_user = authority.split('@').next_back().unwrap_or(authority);
        return Some(
            after_user
                .split(':')
                .next()
                .unwrap_or(after_user)
                .to_string(),
        );
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wt(path: &str, branch: Option<&str>, is_bare: bool) -> crate::git::types::Worktree {
        crate::git::types::Worktree {
            path: path.to_string(),
            head: "abc123".to_string(),
            branch: branch.map(|s| s.to_string()),
            is_bare,
        }
    }

    fn branch(name: &str, is_current: bool, is_merged: bool) -> crate::git::types::Branch {
        crate::git::types::Branch {
            name: name.to_string(),
            is_current,
            upstream: None,
            is_merged,
        }
    }

    #[test]
    fn format_worktree_list_labels_and_detached() {
        let wts = vec![
            wt("/repo", Some("main"), false),
            wt("/repo-detached", None, false),
        ];
        assert_eq!(
            format_worktree_list(&wts),
            vec![
                "main\t/repo".to_string(),
                "(detached)\t/repo-detached".to_string()
            ]
        );
    }

    #[test]
    fn prune_candidates_selects_only_merged_unprotected_noncurrent() {
        let branches = vec![
            branch("main", true, true),          // current → skip
            branch("develop", false, true),      // protected → skip
            branch("feature/done", false, true), // merged → keep
            branch("feature/wip", false, false), // not merged → skip
        ];
        let wts = vec![wt("/wt/done", Some("feature/done"), false)];
        let protected = vec!["main".to_string(), "develop".to_string()];
        let picked = prune_candidates(&branches, &wts, &protected);
        assert_eq!(picked.len(), 1);
        assert_eq!(picked[0].branch, "feature/done");
        assert_eq!(picked[0].wt_path.as_deref(), Some("/wt/done"));
    }

    #[test]
    fn infer_clone_root_ghq_layout() {
        use std::path::PathBuf;
        let local = PathBuf::from("/Users/me/workspace/github.com/owner/repo");
        let id = RepoId {
            host: None,
            owner: "owner".into(),
            name: "repo".into(),
        };
        let root = infer_clone_root(&local, &id).expect("should strip suffix");
        assert_eq!(root, PathBuf::from("/Users/me/workspace"));
    }

    #[test]
    fn infer_clone_root_ghe_layout() {
        use std::path::PathBuf;
        let local = PathBuf::from("/work/ghe.company.com/team/svc");
        let id = RepoId {
            host: Some("ghe.company.com".into()),
            owner: "team".into(),
            name: "svc".into(),
        };
        let root = infer_clone_root(&local, &id).expect("should strip suffix");
        assert_eq!(root, PathBuf::from("/work"));
    }

    #[test]
    fn infer_clone_root_mismatch_returns_none() {
        use std::path::PathBuf;
        let local = PathBuf::from("/Users/me/somewhere/repo-x");
        let id = RepoId {
            host: None,
            owner: "owner".into(),
            name: "repo-x".into(),
        };
        assert!(infer_clone_root(&local, &id).is_none());
    }

    #[test]
    fn test_extract_hostname_ssh() {
        assert_eq!(
            extract_gh_hostname("git@github.com:katzkb/repo.git"),
            Some("github.com".to_string())
        );
        assert_eq!(
            extract_gh_hostname("git@ghe.company.com:org/repo.git"),
            Some("ghe.company.com".to_string())
        );
    }

    #[test]
    fn test_extract_hostname_https() {
        assert_eq!(
            extract_gh_hostname("https://github.com/katzkb/repo.git"),
            Some("github.com".to_string())
        );
        assert_eq!(
            extract_gh_hostname("https://ghe.company.com/org/repo.git"),
            Some("ghe.company.com".to_string())
        );
    }

    #[test]
    fn test_extract_hostname_ssh_url() {
        assert_eq!(
            extract_gh_hostname("ssh://git@ghe.company.com/org/repo.git"),
            Some("ghe.company.com".to_string())
        );
        assert_eq!(
            extract_gh_hostname("ssh://git@ghe.company.com:2222/org/repo.git"),
            Some("ghe.company.com".to_string())
        );
    }

    #[test]
    fn test_extract_hostname_https_with_credentials_and_port() {
        assert_eq!(
            extract_gh_hostname("https://token@ghe.company.com:8443/org/repo.git"),
            Some("ghe.company.com".to_string())
        );
        assert_eq!(
            extract_gh_hostname("https://user@github.com/org/repo.git"),
            Some("github.com".to_string())
        );
    }

    #[test]
    fn test_extract_hostname_unknown() {
        assert_eq!(extract_gh_hostname("file:///path/to/repo"), None);
    }

    #[test]
    fn test_extract_repo_info_ssh() {
        let info = extract_repo_info("git@github.com:katzkb/repo.git").unwrap();
        assert_eq!(info.owner, "katzkb");
        assert_eq!(info.name, "repo");
        assert!(info.host.is_none()); // github.com → None
    }

    #[test]
    fn test_extract_repo_info_ghe() {
        let info = extract_repo_info("git@ghe.company.com:org/repo.git").unwrap();
        assert_eq!(info.owner, "org");
        assert_eq!(info.name, "repo");
        assert_eq!(info.host.as_deref(), Some("ghe.company.com"));
    }

    #[test]
    fn test_extract_repo_info_https() {
        let info = extract_repo_info("https://github.com/katzkb/repo.git").unwrap();
        assert_eq!(info.owner, "katzkb");
        assert_eq!(info.name, "repo");
        assert!(info.host.is_none());
    }

    #[test]
    fn test_extract_repo_info_ssh_url() {
        let info = extract_repo_info("ssh://git@ghe.company.com/org/repo.git").unwrap();
        assert_eq!(info.owner, "org");
        assert_eq!(info.name, "repo");
        assert_eq!(info.host.as_deref(), Some("ghe.company.com"));
    }

    #[test]
    fn test_extract_repo_info_ssh_url_with_port() {
        let info = extract_repo_info("ssh://git@ghe.company.com:2222/org/repo.git").unwrap();
        assert_eq!(info.owner, "org");
        assert_eq!(info.name, "repo");
        assert_eq!(info.host.as_deref(), Some("ghe.company.com"));
    }

    #[test]
    fn test_extract_repo_info_unknown() {
        assert!(extract_repo_info("file:///path/to/repo").is_none());
    }

    #[test]
    fn test_compute_review_status_no_user() {
        use crate::git::types::{PrState, RepoId};
        let pr = PullRequest {
            number: 1,
            title: String::new(),
            author: String::new(),
            state: PrState::Open,
            head_ref: String::new(),
            updated_at: String::new(),
            review_requests: vec![],
            is_draft: false,
            latest_reviews: vec![],
            review_status: None,
            repo_id: RepoId::default(),
        };
        assert_eq!(compute_review_status(&pr, ""), ReviewStatus::NeedsReview);
    }

    #[test]
    fn test_compute_review_status_no_matching_review() {
        use crate::git::types::{LatestReview, PrState, RepoId};
        let pr = PullRequest {
            number: 1,
            title: String::new(),
            author: String::new(),
            state: PrState::Open,
            head_ref: String::new(),
            updated_at: String::new(),
            review_requests: vec![],
            is_draft: false,
            latest_reviews: vec![LatestReview {
                author: "other-user".to_string(),
                state: ReviewState::Approved,
            }],
            review_status: None,
            repo_id: RepoId::default(),
        };
        assert_eq!(
            compute_review_status(&pr, "katzkb"),
            ReviewStatus::NeedsReview
        );
    }

    #[test]
    fn test_compute_review_status_approved() {
        use crate::git::types::{LatestReview, PrState, RepoId};
        let pr = PullRequest {
            number: 1,
            title: String::new(),
            author: String::new(),
            state: PrState::Open,
            head_ref: String::new(),
            updated_at: String::new(),
            review_requests: vec![],
            is_draft: false,
            latest_reviews: vec![LatestReview {
                author: "katzkb".to_string(),
                state: ReviewState::Approved,
            }],
            review_status: None,
            repo_id: RepoId::default(),
        };
        assert_eq!(compute_review_status(&pr, "katzkb"), ReviewStatus::Approved);
    }

    #[test]
    fn test_compute_review_status_changes_requested() {
        use crate::git::types::{LatestReview, PrState, RepoId};
        let pr = PullRequest {
            number: 1,
            title: String::new(),
            author: String::new(),
            state: PrState::Open,
            head_ref: String::new(),
            updated_at: String::new(),
            review_requests: vec![],
            is_draft: false,
            latest_reviews: vec![LatestReview {
                author: "katzkb".to_string(),
                state: ReviewState::ChangesRequested,
            }],
            review_status: None,
            repo_id: RepoId::default(),
        };
        assert_eq!(
            compute_review_status(&pr, "katzkb"),
            ReviewStatus::ChangesRequested
        );
    }
}
