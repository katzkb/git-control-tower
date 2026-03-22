mod app;
mod data;
mod event;
mod git;
mod ui;

use std::process;
use std::time::Duration;

use crossterm::event::KeyEventKind;

use crate::app::App;
use crate::event::{Event, EventHandler};
use crate::git::command::{run_gh, run_git};
use crate::git::parser::{parse_branches, parse_log, parse_worktrees};
use crate::git::types::{PrDetail, PullRequest};
use crate::ui::notification::Notification;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Startup checks before initializing TUI
    check_prerequisites().await;

    let mut terminal = ratatui::init();
    let result = run(&mut terminal).await;
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

async fn run(terminal: &mut ratatui::DefaultTerminal) -> anyhow::Result<()> {
    let mut app = App::new();
    let mut events = EventHandler::new(Duration::from_millis(250));

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

    // Load GitHub user
    if let Ok(user) = run_gh(&["api", "user", "--jq", ".login"]).await {
        app.gh_user = user.trim().to_string();
    }

    // Load PR list
    if let Ok(output) = run_gh(&[
        "pr",
        "list",
        "--json",
        "number,title,author,state,headRefName,updatedAt,reviewRequests",
        "--limit",
        "50",
    ])
    .await
        && let Ok(prs) = serde_json::from_str::<Vec<PullRequest>>(&output)
    {
        app.pull_requests = prs;
    }
    app.prs_loaded = true;

    // Load worktrees
    if let Ok(output) = run_git(&["worktree", "list", "--porcelain"]).await {
        app.worktrees = parse_worktrees(&output);
    }
    app.wt_loaded = true;

    // Load branches
    load_branches(&mut app).await;
    app.branches_loaded = true;

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

        // Delete worktree if requested
        if let Some(path) = app.wt_delete_requested.take() {
            let _ = run_git(&["worktree", "remove", &path]).await;
            // Refresh worktree list
            if let Ok(output) = run_git(&["worktree", "list", "--porcelain"]).await {
                app.worktrees = parse_worktrees(&output);
                if app.wt_scroll >= app.worktrees.len() && app.wt_scroll > 0 {
                    app.wt_scroll -= 1;
                }
            }
        }

        // Create worktree from PR if requested
        if let Some((head_ref, pr_number)) = app.wt_create_requested.take() {
            let remote_ref = format!("origin/{head_ref}");
            let wt_path = format!("../gct-review-{pr_number}");

            // Fetch the branch first
            match run_git(&["fetch", "origin", &head_ref]).await {
                Ok(_) => {
                    // Create the worktree
                    match run_git(&["worktree", "add", &wt_path, &remote_ref]).await {
                        Ok(_) => {
                            app.notification = Some(Notification::success(format!(
                                "Worktree created: {wt_path}"
                            )));
                            // Refresh worktree list
                            if let Ok(output) = run_git(&["worktree", "list", "--porcelain"]).await
                            {
                                app.worktrees = parse_worktrees(&output);
                            }
                        }
                        Err(e) => {
                            app.notification = Some(Notification::error(format!(
                                "Failed to create worktree: {e}"
                            )));
                        }
                    }
                }
                Err(e) => {
                    app.notification = Some(Notification::error(format!("Failed to fetch: {e}")));
                }
            }
        }

        // Delete selected branches if requested
        if app.branch_delete_requested {
            app.branch_delete_requested = false;
            let selected: Vec<String> = app.branch_selected.drain().collect();
            for name in &selected {
                let _ = run_git(&["branch", "-d", name]).await;
            }
            // Refresh branch list
            load_branches(&mut app).await;
            if app.branch_scroll >= app.branches.len() && app.branch_scroll > 0 {
                app.branch_scroll = app.branches.len().saturating_sub(1);
            }
        }

        // Load PR detail if requested
        if let Some(number) = app.pr_detail_requested.take() {
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
                app.pr_detail = Some(detail);
            }
        }

        if app.should_quit {
            break;
        }
    }

    Ok(())
}

async fn load_branches(app: &mut App) {
    let branch_output = run_git(&["branch", "-vv"]).await.unwrap_or_default();
    let merged_output = run_git(&["branch", "--merged"]).await.unwrap_or_default();
    app.branches = parse_branches(&branch_output, &merged_output);
}
