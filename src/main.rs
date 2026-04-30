mod app;
mod config;
mod data;
mod event;
mod git;
mod ui;

use anyhow::Context;
use std::collections::HashSet;
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
use crate::app::{OpProgress, OpStep};
use crate::data::merge_entries;
use crate::event::{Event, EventHandler};
use crate::git::command::{run_gh, run_git};
use crate::git::parser::{parse_branches, parse_log, parse_worktrees};
use crate::git::types::{GitStatus, PrDetail, PullRequest, ReviewStatus};
use crate::ui::notification::Notification;

/// Args used for fetching commits in the Log view (startup + `r` refresh).
const LOG_ARGS: &[&str] = &[
    "log",
    "--format=%h%x00%s%x00%an%x00%ad",
    "--date=short",
    "-n",
    "200",
];

struct RepoInfo {
    owner: String,
    repo: String,
    hostname: Option<String>, // None for github.com
}

enum AsyncResult {
    PrDetail(PrDetail),
    PrDetailError(u64, String),
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

    // Initialize debug logging (GCT_DEBUG=1 or --verbose enables it)
    let verbose = std::env::args().any(|a| a == "--verbose");
    crate::git::command::init_debug_log(verbose);

    // Startup checks and config loading before TUI init (eprintln is safe here)
    check_prerequisites().await;
    let config = config::load_config();

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

    let (result, cd_path) = run(&mut terminal, config, verbose).await;

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

async fn run(
    terminal: &mut Terminal<CrosstermBackend<std::fs::File>>,
    config: config::Config,
    verbose: bool,
) -> (anyhow::Result<()>, Option<String>) {
    let mut app = App::new(config);
    app.verbose = verbose;
    let mut events = EventHandler::new(Duration::from_millis(80));
    let (tx, mut rx) = mpsc::unbounded_channel::<AsyncResult>();
    let mut pr_inflight: HashSet<u64> = HashSet::new();
    let mut status_inflight: HashSet<String> = HashSet::new();

    // Phase 1: Fast local loads (blocking, ~170ms)
    if let Ok(output) = run_git(LOG_ARGS).await {
        app.commits = parse_log(&output);
    }
    if let Ok(output) = run_git(&["worktree", "list", "--porcelain"]).await {
        app.worktrees = parse_worktrees(&output);
    }
    load_branches(&mut app).await;
    let repo_info = run_git(&["remote", "get-url", "origin"])
        .await
        .ok()
        .and_then(|url| extract_repo_info(url.trim()));
    app.entries = merge_entries(&app.branches, &app.worktrees, &[]);
    app.entries_loaded = true;
    app.request_details_for_selection();

    // Phase 2: Slow network loads (background, non-blocking)
    let gh_hostname = repo_info.as_ref().and_then(|r| r.hostname.clone());

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
        let repo = info.repo.clone();
        let hostname = info.hostname.clone();
        tokio::spawn(async move {
            let (prs, errors) =
                data::fetch_local_prs(&branch_names, &owner, &repo, hostname.as_deref()).await;
            let _ = tx_local.send(AsyncResult::LocalPrList(prs, errors));
        });
    } else {
        // No repo info available (no origin remote or unsupported URL format)
        app.local_prs_loaded = true;
    }

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

        // Spawn PR detail load in background (non-blocking, deduplicated)
        if let Some(number) = app.pr_detail_requested.take()
            && !pr_inflight.contains(&number)
        {
            pr_inflight.insert(number);
            let tx = tx.clone();
            tokio::spawn(async move {
                let num_str = number.to_string();
                let result = run_gh(&[
                    "pr",
                    "view",
                    &num_str,
                    "--json",
                    "number,title,author,state,body,additions,deletions,headRefName",
                ])
                .await;
                match result {
                    Ok(output) => match serde_json::from_str::<PrDetail>(&output) {
                        Ok(detail) => {
                            let _ = tx.send(AsyncResult::PrDetail(detail));
                        }
                        Err(e) => {
                            let _ = tx.send(AsyncResult::PrDetailError(
                                number,
                                format!("PR #{number} parse error: {e}"),
                            ));
                        }
                    },
                    Err(e) => {
                        let _ = tx.send(AsyncResult::PrDetailError(
                            number,
                            format!("PR #{number} fetch failed: {e}"),
                        ));
                    }
                }
            });
        }

        // Reload branches/worktrees on `r` refresh from Main view
        if app.branches_reload_requested {
            app.branches_reload_requested = false;
            refresh_entries(&mut app).await;
        }

        // Reload commits on `r` refresh from Log view
        if app.commits_reload_requested {
            app.commits_reload_requested = false;
            if let Ok(output) = run_git(LOG_ARGS).await {
                app.commits = parse_log(&output);
            }
        }

        // Spawn git status load in background (non-blocking, deduplicated)
        if let Some(wt_path) = app.git_status_requested.take()
            && !status_inflight.contains(&wt_path)
        {
            status_inflight.insert(wt_path.clone());
            let tx = tx.clone();
            tokio::spawn(async move {
                if let Some(status) = data::load_git_status(&wt_path).await {
                    let _ = tx.send(AsyncResult::GitStatus { wt_path, status });
                } else {
                    let _ = tx.send(AsyncResult::GitStatusError(wt_path));
                }
            });
        }

        // Spawn PR fetch for view switch (non-blocking)
        if let Some(filter) = app.pr_fetch_requested.take() {
            let tx = tx.clone();
            match filter {
                MainFilter::Local => {
                    if let Some(ref info) = repo_info {
                        let branch_names: Vec<String> =
                            app.branches.iter().map(|b| b.name.clone()).collect();
                        let owner = info.owner.clone();
                        let repo = info.repo.clone();
                        let hostname = info.hostname.clone();
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
                    tokio::spawn(async move {
                        let (prs, errors) = data::fetch_my_prs(show_merged).await;
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
                        app.pr_fetch_requested = Some(MainFilter::ReviewRequested);
                    } else {
                        let show_merged = app.show_merged;
                        let include_team = app.include_team_reviews;
                        let gh_user = app.gh_user.clone();
                        tokio::spawn(async move {
                            let (prs, errors) =
                                data::fetch_review_prs(show_merged, include_team, &gh_user).await;
                            let _ = tx.send(AsyncResult::ReviewPrList(prs, errors));
                        });
                    }
                }
            }
        }

        // Receive completed background results (non-blocking)
        while let Ok(result) = rx.try_recv() {
            match result {
                AsyncResult::PrDetail(detail) => {
                    pr_inflight.remove(&detail.number);
                    app.pr_detail_cache.insert(detail.number, detail);
                }
                AsyncResult::PrDetailError(number, error_msg) => {
                    pr_inflight.remove(&number);
                    app.notification =
                        Some(Notification::error(format!("Failed to load PR #{number}")));
                    if app.verbose && !app.verbose_errors.contains(&error_msg) {
                        app.verbose_errors.push(error_msg);
                    }
                }
                AsyncResult::GitStatus { wt_path, status } => {
                    status_inflight.remove(&wt_path);
                    if let Some(entry) = app
                        .entries
                        .iter_mut()
                        .find(|e| e.worktree_path() == Some(wt_path.as_str()))
                    {
                        entry.git_status = Some(status);
                    }
                }
                AsyncResult::GitStatusError(wt_path) => {
                    status_inflight.remove(&wt_path);
                    if app.verbose {
                        let msg = format!("git status failed for {wt_path}");
                        if !app.verbose_errors.contains(&msg) {
                            app.verbose_errors.push(msg);
                        }
                    }
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
                    if app.verbose && !app.verbose_errors.contains(&error_msg) {
                        app.verbose_errors.push(error_msg);
                    }
                }
                AsyncResult::LocalPrList(prs, errors) => {
                    app.local_prs = prs;
                    app.local_prs_loaded = true;
                    if app.verbose {
                        for e in errors {
                            if !app.verbose_errors.contains(&e) {
                                app.verbose_errors.push(e);
                            }
                        }
                    }
                    if app.main_filter == MainFilter::Local {
                        app.entries =
                            merge_entries(&app.branches, &app.worktrees, app.current_prs());
                        let filtered_len = app.filtered_entries().len();
                        if app.sidebar_scroll >= filtered_len && filtered_len > 0 {
                            app.sidebar_scroll = filtered_len - 1;
                        }
                        app.request_details_for_selection();
                    }
                }
                AsyncResult::MyPrList(prs, errors) => {
                    app.my_prs = prs;
                    app.my_prs_loaded = true;
                    if app.verbose {
                        for e in errors {
                            if !app.verbose_errors.contains(&e) {
                                app.verbose_errors.push(e);
                            }
                        }
                    }
                    if app.main_filter == MainFilter::MyPr {
                        app.entries =
                            merge_entries(&app.branches, &app.worktrees, app.current_prs());
                        let filtered_len = app.filtered_entries().len();
                        if app.sidebar_scroll >= filtered_len && filtered_len > 0 {
                            app.sidebar_scroll = filtered_len - 1;
                        }
                        app.request_details_for_selection();
                    }
                }
                AsyncResult::ReviewPrList(mut prs, errors) => {
                    for pr in &mut prs {
                        pr.review_status = Some(compute_review_status(pr, &app.gh_user));
                    }
                    app.review_prs = prs;
                    app.review_prs_loaded = true;
                    if app.verbose {
                        for e in errors {
                            if !app.verbose_errors.contains(&e) {
                                app.verbose_errors.push(e);
                            }
                        }
                    }
                    if app.main_filter == MainFilter::ReviewRequested {
                        app.entries =
                            merge_entries(&app.branches, &app.worktrees, app.current_prs());
                        let filtered_len = app.filtered_entries().len();
                        if app.sidebar_scroll >= filtered_len && filtered_len > 0 {
                            app.sidebar_scroll = filtered_len - 1;
                        }
                        app.request_details_for_selection();
                    }
                }
                AsyncResult::WtCreated {
                    wt_path,
                    copy_errors,
                } => {
                    app.wt_inflight.remove(&wt_path);
                    if copy_errors.is_empty() {
                        app.notification = Some(Notification::success(format!(
                            "Worktree created: {wt_path}"
                        )));
                    } else {
                        app.notification = Some(Notification::success(format!(
                            "Worktree created: {wt_path} (copy errors: {})",
                            copy_errors.len()
                        )));
                        if app.verbose {
                            app.verbose_errors.extend(copy_errors);
                        }
                    }
                    refresh_entries(&mut app).await;
                    if app.confirm_dialog.is_none() {
                        app.confirm_dialog = Some(crate::ui::confirm_dialog::ConfirmDialog::new(
                            "Move to Worktree",
                            format!(
                                "Move into the new worktree?\n(Requires shell integration — see README.)\n{wt_path}"
                            ),
                        ));
                        app.wt_cd_pending_path = Some(wt_path);
                    }
                }
                AsyncResult::WtCreateError { wt_path, message } => {
                    app.wt_inflight.remove(&wt_path);
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
                        app.wt_inflight.remove(path);
                    }
                    app.progress.sweep_unfinished();
                    let success_parts =
                        bulk_success_parts(branches_deleted.len(), worktrees_removed.len());
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
                        if app.verbose {
                            for err in &failures {
                                if !app.verbose_errors.contains(err) {
                                    app.verbose_errors.push(err.clone());
                                }
                            }
                        }
                    }
                    app.progress.clear();
                    app.quit_pressed_during_progress = false;
                    refresh_entries(&mut app).await;
                }
                AsyncResult::WtForceDecisionRequested { path, reason } => {
                    // Plain remove failed; ask the user whether to force.
                    app.confirm_dialog = Some(crate::ui::confirm_dialog::ConfirmDialog::new(
                        "Force Delete Worktree",
                        format!("{reason}\nForce remove {path}?"),
                    ));
                    app.wt_force_delete_pending_path = Some(path);
                }
            }
        }

        // Delete worktree if requested (single-item, no auto-force fallback).
        if let Some(path) = app.wt_delete_requested.take() {
            app.wt_inflight.insert(path.clone());
            let op_id = app.progress.allocate_ids(1).start;
            let label = wt_label_for(&path);
            let claimed = vec![path.clone()];
            let tx_c = tx.clone();
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

        // Force delete worktree if confirmed (single-item, force only).
        if let Some(path) = app.wt_force_delete_requested.take() {
            app.wt_inflight.insert(path.clone());
            let op_id = app.progress.allocate_ids(1).start;
            let label = wt_label_for(&path);
            let claimed = vec![path.clone()];
            let tx_c = tx.clone();
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

        // Create worktree if requested (async, non-blocking)
        if let Some(branch_name) = app.wt_create_requested.take() {
            let wt_path = app.config.worktree_path(&branch_name);
            app.wt_inflight.insert(wt_path.clone());
            if let Some(parent) = std::path::Path::new(&wt_path).parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let has_local = app.branches.iter().any(|b| b.name == branch_name);
            let post_create = app.config.worktree.post_create.clone();
            let tx = tx.clone();
            tokio::spawn(async move {
                let result = if has_local {
                    run_git(&["worktree", "add", &wt_path, &branch_name]).await
                } else {
                    match run_git(&["fetch", "origin", &branch_name]).await {
                        Ok(_) => run_git(&["worktree", "add", &wt_path, &branch_name]).await,
                        Err(e) => Err(e),
                    }
                };
                match result {
                    Ok(_) => {
                        let repo_root = std::env::current_dir().unwrap_or_default();
                        let copy_errors = config::run_post_create(
                            &post_create,
                            &repo_root,
                            std::path::Path::new(&wt_path),
                        );
                        let _ = tx.send(AsyncResult::WtCreated {
                            wt_path,
                            copy_errors,
                        });
                    }
                    Err(e) => {
                        let _ = tx.send(AsyncResult::WtCreateError {
                            wt_path,
                            message: format!("Failed to create worktree: {e}"),
                        });
                    }
                }
            });
        }

        // Delete selected entries (branches + optional worktrees) in parallel.
        if app.branch_delete_requested {
            app.branch_delete_requested = false;
            let selected: Vec<String> = app.branch_selected.drain().collect();

            struct Work {
                name: String,
                wt_path: Option<String>,
                has_local_branch: bool,
            }
            let mut work: Vec<Work> = Vec::with_capacity(selected.len());
            let mut wt_paths_claimed: Vec<String> = Vec::new();
            for name in selected {
                let Some(entry) = app.entries.iter().find(|e| e.name == name) else {
                    continue;
                };
                if entry.is_current() || app.is_protected_branch(&entry.name) {
                    continue;
                }
                let wt_path = entry.worktree_path().map(str::to_string);
                if let Some(ref p) = wt_path
                    && app.wt_inflight.contains(p)
                {
                    continue;
                }
                if let Some(ref p) = wt_path {
                    app.wt_inflight.insert(p.clone());
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
                    let tx_c = tx.clone();
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

                let tx_done = tx.clone();
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

        // Create a new branch from the selected branch if requested
        if let Some((source, name)) = app.branch_create_requested.take() {
            match run_git(&["branch", "--", &name, &source]).await {
                Ok(_) => {
                    app.notification = Some(Notification::success(format!(
                        "Created branch '{name}' from '{source}'"
                    )));
                    refresh_entries(&mut app).await;
                }
                Err(e) => {
                    let err_str = e.to_string();
                    let short = err_str.lines().next().unwrap_or(&err_str).to_string();
                    app.notification = Some(Notification::error(short));
                    if app.verbose && !app.verbose_errors.contains(&err_str) {
                        app.verbose_errors.push(err_str);
                    }
                }
            }
        }

        // Open PR in browser if requested
        if let Some(pr_number) = app.open_pr_requested.take() {
            let _ = run_gh(&["pr", "view", &pr_number.to_string(), "--web"]).await;
        }

        // Copy branch name to clipboard if requested
        if let Some(name) = app.copy_branch_requested.take() {
            copy_to_clipboard(&name);
        }

        if app.should_quit {
            break;
        }
    }

    let cd_path = app.cd_path.clone();
    (Ok(()), cd_path)
}

async fn refresh_entries(app: &mut App) {
    load_branches(app).await;
    match run_git(&["worktree", "list", "--porcelain"]).await {
        Ok(output) => app.worktrees = parse_worktrees(&output),
        Err(e) => {
            if app.verbose {
                let msg = format!("git worktree list failed: {e}");
                if !app.verbose_errors.contains(&msg) {
                    app.verbose_errors.push(msg);
                }
            }
        }
    }
    app.entries = merge_entries(&app.branches, &app.worktrees, app.current_prs());
    let filtered_len = app.filtered_entries().len();
    if app.sidebar_scroll >= filtered_len && filtered_len > 0 {
        app.sidebar_scroll = filtered_len - 1;
    }
}

async fn load_branches(app: &mut App) {
    let branch_output = match run_git(&["branch", "-vv"]).await {
        Ok(output) => output,
        Err(e) => {
            if app.verbose {
                let msg = format!("git branch -vv failed: {e}");
                if !app.verbose_errors.contains(&msg) {
                    app.verbose_errors.push(msg);
                }
            }
            return;
        }
    };
    let default_branch = detect_default_branch().await;
    let merged_output = match run_git(&["branch", "--merged", &default_branch]).await {
        Ok(output) => output,
        Err(e) => {
            if app.verbose {
                let msg = format!("git branch --merged failed: {e}");
                if !app.verbose_errors.contains(&msg) {
                    app.verbose_errors.push(msg);
                }
            }
            String::new()
        }
    };
    let base_hash = match run_git(&["rev-parse", &default_branch]).await {
        Ok(output) => output,
        Err(e) => {
            if app.verbose {
                let msg = format!("git rev-parse {default_branch} failed: {e}");
                if !app.verbose_errors.contains(&msg) {
                    app.verbose_errors.push(msg);
                }
            }
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
    gct shell-init <SHELL>

OPTIONS:
    -h, --help        Print this help message and exit
    -v, --version     Print version and exit
        --verbose     Surface silenced errors for troubleshooting

SUBCOMMANDS:
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

fn compute_review_status(pr: &PullRequest, gh_user: &str) -> ReviewStatus {
    if gh_user.is_empty() {
        return ReviewStatus::NeedsReview;
    }
    for review in &pr.latest_reviews {
        if review.author == gh_user {
            return match review.state.as_str() {
                "APPROVED" => ReviewStatus::Approved,
                "CHANGES_REQUESTED" => ReviewStatus::ChangesRequested,
                "COMMENTED" => ReviewStatus::Commented,
                _ => ReviewStatus::Commented,
            };
        }
    }
    ReviewStatus::NeedsReview
}

/// Extract owner, repo, and hostname from a git remote URL.
fn extract_repo_info(remote_url: &str) -> Option<RepoInfo> {
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

    Some(RepoInfo {
        owner: parts[0].to_string(),
        repo: parts[1].to_string(),
        hostname: gh_hostname,
    })
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
        assert_eq!(info.repo, "repo");
        assert!(info.hostname.is_none()); // github.com → None
    }

    #[test]
    fn test_extract_repo_info_ghe() {
        let info = extract_repo_info("git@ghe.company.com:org/repo.git").unwrap();
        assert_eq!(info.owner, "org");
        assert_eq!(info.repo, "repo");
        assert_eq!(info.hostname.as_deref(), Some("ghe.company.com"));
    }

    #[test]
    fn test_extract_repo_info_https() {
        let info = extract_repo_info("https://github.com/katzkb/repo.git").unwrap();
        assert_eq!(info.owner, "katzkb");
        assert_eq!(info.repo, "repo");
        assert!(info.hostname.is_none());
    }

    #[test]
    fn test_extract_repo_info_ssh_url() {
        let info = extract_repo_info("ssh://git@ghe.company.com/org/repo.git").unwrap();
        assert_eq!(info.owner, "org");
        assert_eq!(info.repo, "repo");
        assert_eq!(info.hostname.as_deref(), Some("ghe.company.com"));
    }

    #[test]
    fn test_extract_repo_info_ssh_url_with_port() {
        let info = extract_repo_info("ssh://git@ghe.company.com:2222/org/repo.git").unwrap();
        assert_eq!(info.owner, "org");
        assert_eq!(info.repo, "repo");
        assert_eq!(info.hostname.as_deref(), Some("ghe.company.com"));
    }

    #[test]
    fn test_extract_repo_info_unknown() {
        assert!(extract_repo_info("file:///path/to/repo").is_none());
    }

    #[test]
    fn test_compute_review_status_no_user() {
        let pr = PullRequest {
            number: 1,
            title: String::new(),
            author: String::new(),
            state: "OPEN".to_string(),
            head_ref: String::new(),
            updated_at: String::new(),
            review_requests: vec![],
            is_draft: false,
            latest_reviews: vec![],
            review_status: None,
        };
        assert_eq!(compute_review_status(&pr, ""), ReviewStatus::NeedsReview);
    }

    #[test]
    fn test_compute_review_status_no_matching_review() {
        use crate::git::types::LatestReview;
        let pr = PullRequest {
            number: 1,
            title: String::new(),
            author: String::new(),
            state: "OPEN".to_string(),
            head_ref: String::new(),
            updated_at: String::new(),
            review_requests: vec![],
            is_draft: false,
            latest_reviews: vec![LatestReview {
                author: "other-user".to_string(),
                state: "APPROVED".to_string(),
            }],
            review_status: None,
        };
        assert_eq!(
            compute_review_status(&pr, "katzkb"),
            ReviewStatus::NeedsReview
        );
    }

    #[test]
    fn test_compute_review_status_approved() {
        use crate::git::types::LatestReview;
        let pr = PullRequest {
            number: 1,
            title: String::new(),
            author: String::new(),
            state: "OPEN".to_string(),
            head_ref: String::new(),
            updated_at: String::new(),
            review_requests: vec![],
            is_draft: false,
            latest_reviews: vec![LatestReview {
                author: "katzkb".to_string(),
                state: "APPROVED".to_string(),
            }],
            review_status: None,
        };
        assert_eq!(compute_review_status(&pr, "katzkb"), ReviewStatus::Approved);
    }

    #[test]
    fn test_compute_review_status_changes_requested() {
        use crate::git::types::LatestReview;
        let pr = PullRequest {
            number: 1,
            title: String::new(),
            author: String::new(),
            state: "OPEN".to_string(),
            head_ref: String::new(),
            updated_at: String::new(),
            review_requests: vec![],
            is_draft: false,
            latest_reviews: vec![LatestReview {
                author: "katzkb".to_string(),
                state: "CHANGES_REQUESTED".to_string(),
            }],
            review_status: None,
        };
        assert_eq!(
            compute_review_status(&pr, "katzkb"),
            ReviewStatus::ChangesRequested
        );
    }
}
