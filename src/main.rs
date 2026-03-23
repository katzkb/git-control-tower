mod app;
mod config;
mod data;
mod event;
mod git;
mod ui;

use std::process;
use std::time::Duration;

use crossterm::event::KeyEventKind;
use tokio::sync::mpsc;

use crate::app::App;
use crate::data::merge_entries;
use crate::event::{Event, EventHandler};
use crate::git::command::{run_gh, run_git};
use crate::git::parser::{parse_branches, parse_log, parse_worktrees};
use crate::git::types::{GitStatus, PrDetail, PullRequest};
use crate::ui::notification::Notification;

enum AsyncResult {
    PrDetail(PrDetail),
    GitStatus { wt_path: String, status: GitStatus },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Startup checks and config loading before TUI init (eprintln is safe here)
    check_prerequisites().await;
    let config = config::load_config();

    let mut terminal = ratatui::init();
    let result = run(&mut terminal, &config).await;
    ratatui::restore();
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
    terminal: &mut ratatui::DefaultTerminal,
    config: &config::Config,
) -> anyhow::Result<()> {
    let mut app = App::new();
    let mut events = EventHandler::new(Duration::from_millis(250));
    let (tx, mut rx) = mpsc::unbounded_channel::<AsyncResult>();

    // Load commit history
    if let Ok(output) = run_git(&[
        "log",
        "--format=%h%x00%s%x00%an%x00%ad",
        "--date=short",
        "-n",
        "200",
    ])
    .await
    {
        app.commits = parse_log(&output);
    }

    // Load GitHub user (uses graphql viewer query which respects GHE host)
    if let Ok(user) = run_gh(&[
        "api",
        "graphql",
        "-f",
        "query={viewer{login}}",
        "--jq",
        ".data.viewer.login",
    ])
    .await
    {
        app.gh_user = user.trim().to_string();
    }

    // Load PR list (open + merged)
    let pr_fields = "number,title,author,state,headRefName,updatedAt,reviewRequests";
    if let Ok(output) = run_gh(&["pr", "list", "--json", pr_fields, "--limit", "50"]).await
        && let Ok(prs) = serde_json::from_str::<Vec<PullRequest>>(&output)
    {
        app.pull_requests = prs;
    }
    if let Ok(output) = run_gh(&[
        "pr", "list", "--state", "merged", "--json", pr_fields, "--limit", "50",
    ])
    .await
        && let Ok(prs) = serde_json::from_str::<Vec<PullRequest>>(&output)
    {
        app.pull_requests.extend(prs);
    }

    // Load worktrees
    if let Ok(output) = run_git(&["worktree", "list", "--porcelain"]).await {
        app.worktrees = parse_worktrees(&output);
    }

    // Load branches
    load_branches(&mut app).await;

    // Build merged entries
    app.entries = merge_entries(&app.branches, &app.worktrees, &app.pull_requests);
    app.entries_loaded = true;

    // Request details for the initial selection (lazy-loads git status and PR detail)
    app.request_details_for_selection();

    loop {
        terminal.draw(|frame| ui::draw(frame, &app))?;

        match events.next().await {
            Some(Event::Key(key)) if key.kind == KeyEventKind::Press => {
                app.handle_key(key);
            }
            Some(Event::Resize(_, _)) => {}
            Some(Event::Tick) => {}
            Some(Event::Key(_)) => {}
            None => break,
        }

        // Spawn PR detail load in background (non-blocking)
        if let Some(number) = app.pr_detail_requested.take() {
            let tx = tx.clone();
            tokio::spawn(async move {
                let num_str = number.to_string();
                if let Ok(output) = run_gh(&[
                    "pr",
                    "view",
                    &num_str,
                    "--json",
                    "number,title,author,state,body,additions,deletions,headRefName",
                ])
                .await
                    && let Ok(detail) = serde_json::from_str::<PrDetail>(&output)
                {
                    let _ = tx.send(AsyncResult::PrDetail(detail));
                }
            });
        }

        // Spawn git status load in background (non-blocking)
        if let Some(wt_path) = app.git_status_requested.take() {
            let tx = tx.clone();
            tokio::spawn(async move {
                if let Some(status) = data::load_git_status(&wt_path).await {
                    let _ = tx.send(AsyncResult::GitStatus { wt_path, status });
                }
            });
        }

        // Receive completed background results (non-blocking)
        while let Ok(result) = rx.try_recv() {
            match result {
                AsyncResult::PrDetail(detail) => {
                    app.pr_detail_cache.insert(detail.number, detail);
                }
                AsyncResult::GitStatus { wt_path, status } => {
                    if let Some(entry) = app
                        .entries
                        .iter_mut()
                        .find(|e| e.worktree_path() == Some(wt_path.as_str()))
                    {
                        entry.git_status = Some(status);
                    }
                }
            }
        }

        // Delete worktree if requested
        if let Some(path) = app.wt_delete_requested.take() {
            match run_git(&["worktree", "remove", &path]).await {
                Ok(_) => {
                    app.notification =
                        Some(Notification::success(format!("Worktree removed: {path}")));
                }
                Err(e) => {
                    app.notification = Some(Notification::error(format!(
                        "Failed to remove worktree: {e}"
                    )));
                }
            }
            refresh_entries(&mut app).await;
        }

        // Create worktree from PR if requested
        if let Some((head_ref, _pr_number)) = app.wt_create_requested.take() {
            let wt_path = config.worktree_path(&head_ref);
            if let Some(parent) = std::path::Path::new(&wt_path).parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            match run_git(&["fetch", "origin", &head_ref]).await {
                Ok(_) => match run_git(&["worktree", "add", &wt_path, &head_ref]).await {
                    Ok(_) => {
                        app.notification = Some(Notification::success(format!(
                            "Worktree created: {wt_path}"
                        )));
                    }
                    Err(e) => {
                        app.notification = Some(Notification::error(format!(
                            "Failed to create worktree: {e}"
                        )));
                    }
                },
                Err(e) => {
                    app.notification = Some(Notification::error(format!("Failed to fetch: {e}")));
                }
            }
            refresh_entries(&mut app).await;
        }

        // Delete selected branches if requested
        if app.branch_delete_requested {
            app.branch_delete_requested = false;
            let selected: Vec<String> = app.branch_selected.drain().collect();
            for name in &selected {
                let _ = run_git(&["branch", "-d", name]).await;
            }
            refresh_entries(&mut app).await;
        }

        if app.should_quit {
            break;
        }
    }

    Ok(())
}

async fn refresh_entries(app: &mut App) {
    load_branches(app).await;
    if let Ok(output) = run_git(&["worktree", "list", "--porcelain"]).await {
        app.worktrees = parse_worktrees(&output);
    }
    app.entries = merge_entries(&app.branches, &app.worktrees, &app.pull_requests);
    let filtered_len = app.filtered_entries().len();
    if app.sidebar_scroll >= filtered_len && filtered_len > 0 {
        app.sidebar_scroll = filtered_len - 1;
    }
}

async fn load_branches(app: &mut App) {
    let branch_output = run_git(&["branch", "-vv"]).await.unwrap_or_default();
    let merged_output = run_git(&["branch", "--merged"]).await.unwrap_or_default();
    app.branches = parse_branches(&branch_output, &merged_output);
}
