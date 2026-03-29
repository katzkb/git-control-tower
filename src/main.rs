mod app;
mod config;
mod data;
mod event;
mod git;
mod ui;

use std::collections::HashSet;
use std::process;
use std::time::Duration;

use crossterm::event::KeyEventKind;
use tokio::sync::mpsc;

use crate::app::App;
use crate::app::MainFilter;
use crate::data::merge_entries;
use crate::event::{Event, EventHandler};
use crate::git::command::{run_gh, run_git};
use crate::git::parser::{parse_branches, parse_log, parse_worktrees};
use crate::git::types::{GitStatus, PrDetail, PullRequest, ReviewStatus};
use crate::ui::notification::Notification;

struct RepoInfo {
    owner: String,
    repo: String,
    hostname: Option<String>, // None for github.com
}

enum AsyncResult {
    PrDetail(PrDetail),
    PrDetailError(u64, String),
    GitStatus { wt_path: String, status: GitStatus },
    GitStatusError(String),
    UserLogin(String),
    UserLoginError(String),
    LocalPrList(Vec<PullRequest>, Vec<String>),
    MyPrList(Vec<PullRequest>, Vec<String>),
    ReviewPrList(Vec<PullRequest>, Vec<String>),
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Handle --version / -v before anything else
    if std::env::args().any(|a| a == "--version" || a == "-v") {
        println!("gct v{}", env!("CARGO_PKG_VERSION"));
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

    let mut terminal = ratatui::init();
    let (result, cd_path) = run(&mut terminal, &config, verbose).await;
    ratatui::restore();
    if let Some(path) = cd_path {
        println!("{path}");
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
    terminal: &mut ratatui::DefaultTerminal,
    config: &config::Config,
    verbose: bool,
) -> (anyhow::Result<()>, Option<String>) {
    let mut app = App::new();
    app.verbose = verbose;
    let mut events = EventHandler::new(Duration::from_millis(80));
    let (tx, mut rx) = mpsc::unbounded_channel::<AsyncResult>();
    let mut pr_inflight: HashSet<u64> = HashSet::new();
    let mut status_inflight: HashSet<String> = HashSet::new();

    // Phase 1: Fast local loads (blocking, ~170ms)
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
                    let show_merged = app.show_merged;
                    let include_team = app.include_team_reviews;
                    tokio::spawn(async move {
                        let (prs, errors) = data::fetch_review_prs(show_merged, include_team).await;
                        let _ = tx.send(AsyncResult::ReviewPrList(prs, errors));
                    });
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

        // Create worktree if requested
        if let Some(branch_name) = app.wt_create_requested.take() {
            let wt_path = config.worktree_path(&branch_name);
            if let Some(parent) = std::path::Path::new(&wt_path).parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let has_local = app.branches.iter().any(|b| b.name == branch_name);
            let result = if has_local {
                // Local branch exists — no fetch needed
                run_git(&["worktree", "add", &wt_path, &branch_name]).await
            } else {
                // Remote-only branch — fetch first
                match run_git(&["fetch", "origin", &branch_name]).await {
                    Ok(_) => run_git(&["worktree", "add", &wt_path, &branch_name]).await,
                    Err(e) => Err(e),
                }
            };
            match result {
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
            }
            refresh_entries(&mut app).await;
        }

        // Delete selected branches if requested
        if app.branch_delete_requested {
            app.branch_delete_requested = false;
            let selected: Vec<(String, bool)> = app
                .branch_selected
                .drain()
                .map(|name| {
                    let force = app
                        .entries
                        .iter()
                        .any(|e| e.name == name && (e.is_merged() || e.pr_is_merged()));
                    (name, force)
                })
                .collect();
            let mut deleted = Vec::new();
            let mut failed = Vec::new();
            for (name, force) in &selected {
                let flag = if *force { "-D" } else { "-d" };
                match run_git(&["branch", flag, name]).await {
                    Ok(_) => deleted.push(name.as_str()),
                    Err(e) => failed.push(format!("{name}: {e}")),
                }
            }
            if !failed.is_empty() {
                // Show first line only in notification (git errors can be multi-line)
                let short: Vec<String> = failed
                    .iter()
                    .map(|e| e.lines().next().unwrap_or(e).to_string())
                    .collect();
                app.notification = Some(Notification::error(format!(
                    "Failed to delete: {}",
                    short.join("; ")
                )));
                if app.verbose {
                    let full_msg = format!("Failed to delete: {}", failed.join("; "));
                    if !app.verbose_errors.contains(&full_msg) {
                        app.verbose_errors.push(full_msg);
                    }
                }
            } else if !deleted.is_empty() {
                app.notification = Some(Notification::success(format!(
                    "Deleted {} branch(es)",
                    deleted.len()
                )));
            }
            refresh_entries(&mut app).await;
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

fn print_shell_init(shell: &str) {
    match shell {
        "zsh" | "bash" => {
            print!(
                r#"gct() {{
    local output
    output=$(command gct "$@")
    local status=$?
    if [[ $status -eq 0 && -n "$output" && -d "$output" ]]; then
        cd "$output" || return $?
    elif [[ -n "$output" ]]; then
        echo "$output"
    fi
    return $status
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
        cd "$output"; or return $status
    else if test -n "$output"
        echo "$output"
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
    use base64::Engine;
    use std::io::Write;
    let encoded = base64::engine::general_purpose::STANDARD.encode(text);
    let osc52 = format!("\x1b]52;c;{encoded}\x07");
    // Write to /dev/tty to bypass shell function stdout capture
    if let Ok(mut tty) = std::fs::OpenOptions::new().write(true).open("/dev/tty") {
        let _ = tty.write_all(osc52.as_bytes());
        let _ = tty.flush();
    } else {
        // Fallback to stdout
        let _ = std::io::stdout().write_all(osc52.as_bytes());
        let _ = std::io::stdout().flush();
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
