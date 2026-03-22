mod app;
mod data;
mod event;
mod git;
mod ui;

use std::process;
use std::time::Duration;

use crossterm::event::KeyEventKind;

use crate::app::App;
use crate::data::merge_entries;
use crate::event::{Event, EventHandler};
use crate::git::command::{run_gh, run_git};
use crate::git::parser::{parse_branches, parse_log, parse_worktrees};
use crate::git::types::{PrDetail, PullRequest};

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
                app.pr_detail_cache.insert(detail.number, detail);
            }
        }

        // Load git status if requested for a specific worktree
        if let Some(wt_path) = app.git_status_requested.take()
            && let Some(status) = data::load_git_status(&wt_path).await
        {
            // Find the entry with this worktree path and update its git_status
            if let Some(entry) = app
                .entries
                .iter_mut()
                .find(|e| e.worktree_path() == Some(wt_path.as_str()))
            {
                entry.git_status = Some(status);
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
