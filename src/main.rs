mod app;
mod event;
mod git;
mod ui;

use std::time::Duration;

use crossterm::event::KeyEventKind;

use crate::app::App;
use crate::event::{Event, EventHandler};
use crate::git::command::{run_gh, run_git};
use crate::git::parser::{parse_log, parse_worktrees};
use crate::git::types::{PrDetail, PullRequest};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut terminal = ratatui::init();
    let result = run(&mut terminal).await;
    ratatui::restore();
    result
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
