use crossterm::event::{KeyCode, KeyEvent};

use std::collections::{HashMap, HashSet};

use crate::git::types::{Branch, BranchEntry, Commit, PrDetail, PullRequest, Worktree};
use crate::ui::confirm_dialog::ConfirmDialog;
use crate::ui::notification::Notification;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum ActiveView {
    #[default]
    Main,
    Log,
    History,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum MainFilter {
    #[default]
    Local,
    MyPr,
    ReviewRequested,
}

impl MainFilter {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Local => "Local",
            Self::MyPr => "My PR",
            Self::ReviewRequested => "Review",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionItem {
    CreateWorktree,
    CdIntoWorktree,
    DeleteWorktree,
    DeleteBranch,
    OpenPrInBrowser,
    CopyBranchName,
}

impl ActionItem {
    pub fn label(&self) -> &'static str {
        match self {
            Self::CreateWorktree => "Create worktree",
            Self::CdIntoWorktree => "cd into worktree",
            Self::DeleteWorktree => "Delete worktree",
            Self::DeleteBranch => "Delete branch",
            Self::OpenPrInBrowser => "Open PR in browser",
            Self::CopyBranchName => "Copy branch name",
        }
    }
}

pub struct ActionMenu {
    pub items: Vec<ActionItem>,
    pub scroll: usize,
    pub target_name: String, // branch name captured when menu was opened
}

pub struct App {
    pub active_view: ActiveView,
    pub should_quit: bool,

    // Log View
    pub commits: Vec<Commit>,
    pub log_scroll: usize,

    // History View
    pub history_scroll: usize,

    // Main View — unified entries
    pub entries: Vec<BranchEntry>,
    pub entries_loaded: bool,
    pub main_filter: MainFilter,
    pub sidebar_scroll: usize,
    pub sidebar_offset: usize,
    pub search_active: bool,
    pub search_query: String,
    search_pre_scroll: usize, // saved scroll position before search

    // Raw data sources (for merge_entries and async reload)
    pub branches: Vec<Branch>,
    pub worktrees: Vec<Worktree>,
    pub gh_user: String,
    pub gh_user_load_failed: bool,

    // Per-view PR caches
    pub local_prs: Vec<PullRequest>,
    pub my_prs: Vec<PullRequest>,
    pub review_prs: Vec<PullRequest>,
    pub local_prs_loaded: bool,
    pub my_prs_loaded: bool,
    pub review_prs_loaded: bool,
    pub show_merged: bool,
    pub include_team_reviews: bool,
    pub pr_fetch_requested: Option<MainFilter>,

    // PR Detail (for detail pane, cached by PR number)
    pub pr_detail_cache: HashMap<u64, PrDetail>,
    pub pr_detail_scroll: usize,
    pub pr_detail_requested: Option<u64>,

    // Git status loading
    pub git_status_requested: Option<String>, // worktree path

    // Verbose mode
    pub verbose: bool,
    pub verbose_errors: Vec<String>,

    // Overlays
    pub confirm_dialog: Option<ConfirmDialog>,
    pub action_menu: Option<ActionMenu>,
    pub notification: Option<Notification>,
    pub show_help: bool,

    // Exit with cd path
    pub cd_path: Option<String>,

    // Action requests
    pub wt_delete_requested: Option<String>,
    pub wt_delete_pending_path: Option<String>, // path stored when confirm dialog shown
    pub wt_force_delete_requested: Option<String>,
    pub wt_force_delete_pending_path: Option<String>,
    pub wt_create_requested: Option<String>,
    pub wt_loading: bool,
    pub branch_selected: HashSet<String>,
    pub branch_delete_requested: bool,
    pub open_pr_requested: Option<u64>,
    pub copy_branch_requested: Option<String>,
    pub branches_reload_requested: bool,
    pub commits_reload_requested: bool,

    // Spinner animation
    spinner_tick: usize,

    // Loaded TOML config (protected_branches, worktree, …)
    pub config: crate::config::Config,
}

impl App {
    pub fn new(config: crate::config::Config) -> Self {
        Self {
            active_view: ActiveView::default(),
            should_quit: false,
            commits: Vec::new(),
            log_scroll: 0,
            history_scroll: 0,
            entries: Vec::new(),
            entries_loaded: false,
            main_filter: MainFilter::default(),
            sidebar_scroll: 0,
            sidebar_offset: 0,
            search_active: false,
            search_query: String::new(),
            search_pre_scroll: 0,
            branches: Vec::new(),
            worktrees: Vec::new(),
            gh_user: String::new(),
            gh_user_load_failed: false,
            local_prs: Vec::new(),
            my_prs: Vec::new(),
            review_prs: Vec::new(),
            local_prs_loaded: false,
            my_prs_loaded: false,
            review_prs_loaded: false,
            show_merged: false,
            include_team_reviews: false,
            pr_fetch_requested: None,
            pr_detail_cache: HashMap::new(),
            pr_detail_scroll: 0,
            pr_detail_requested: None,
            git_status_requested: None,
            verbose: false,
            verbose_errors: Vec::new(),
            confirm_dialog: None,
            action_menu: None,
            notification: None,
            show_help: false,
            cd_path: None,
            wt_delete_requested: None,
            wt_delete_pending_path: None,
            wt_force_delete_requested: None,
            wt_force_delete_pending_path: None,
            wt_create_requested: None,
            wt_loading: false,
            branch_selected: HashSet::new(),
            branch_delete_requested: false,
            open_pr_requested: None,
            copy_branch_requested: None,
            branches_reload_requested: false,
            commits_reload_requested: false,
            spinner_tick: 0,
            config,
        }
    }

    pub fn current_prs(&self) -> &[PullRequest] {
        match self.main_filter {
            MainFilter::Local => &self.local_prs,
            MainFilter::MyPr => &self.my_prs,
            MainFilter::ReviewRequested => &self.review_prs,
        }
    }

    const SPINNER_FRAMES: &'static [&'static str] =
        &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

    pub fn tick(&mut self) {
        self.spinner_tick = self.spinner_tick.wrapping_add(1);
        // Don't auto-dismiss while a worktree operation is in progress
        if !self.wt_loading
            && let Some(ref mut n) = self.notification
        {
            if n.ticks_remaining > 0 {
                n.ticks_remaining -= 1;
            } else {
                self.notification = None;
            }
        }
    }

    pub fn spinner_frame(&self) -> &'static str {
        Self::SPINNER_FRAMES[self.spinner_tick % Self::SPINNER_FRAMES.len()]
    }

    pub fn adjust_sidebar_offset(&mut self, visible_height: usize, item_count: usize) {
        if visible_height == 0 {
            return;
        }
        // Clamp scroll to valid range
        if item_count > 0 {
            self.sidebar_scroll = self.sidebar_scroll.min(item_count - 1);
        } else {
            self.sidebar_scroll = 0;
        }
        // Adjust offset when cursor exceeds viewport bounds
        if self.sidebar_scroll >= self.sidebar_offset + visible_height {
            self.sidebar_offset = self.sidebar_scroll - visible_height + 1;
        }
        if self.sidebar_scroll < self.sidebar_offset {
            self.sidebar_offset = self.sidebar_scroll;
        }
        // Clamp offset so list doesn't show blank space
        let max_offset = item_count.saturating_sub(visible_height);
        self.sidebar_offset = self.sidebar_offset.min(max_offset);
    }

    pub fn is_current_view_loading(&self) -> bool {
        match self.main_filter {
            MainFilter::Local => !self.local_prs_loaded,
            MainFilter::MyPr => !self.my_prs_loaded,
            MainFilter::ReviewRequested => !self.review_prs_loaded,
        }
    }

    pub fn rebuild_entries(&mut self) {
        self.entries =
            crate::data::merge_entries(&self.branches, &self.worktrees, self.current_prs());
    }

    pub fn filtered_entries(&self) -> Vec<&BranchEntry> {
        let search_query = if !self.search_query.is_empty() {
            Some(self.search_query.to_lowercase())
        } else {
            None
        };
        self.entries
            .iter()
            .filter(|entry| {
                let passes_filter = match self.main_filter {
                    MainFilter::Local => entry.has_local(),
                    // My PR / Review: server-side filtered, just check PR exists
                    MainFilter::MyPr | MainFilter::ReviewRequested => entry.pull_request.is_some(),
                };
                let passes_search = match &search_query {
                    Some(q) => entry.name.to_lowercase().contains(q.as_str()),
                    None => true,
                };
                passes_filter && passes_search
            })
            .collect()
    }

    pub fn selected_entry(&self) -> Option<&BranchEntry> {
        self.filtered_entries().into_iter().nth(self.sidebar_scroll)
    }

    /// Return the cached PR detail for the currently selected entry, if available.
    pub fn selected_pr_detail(&self) -> Option<&PrDetail> {
        let entry = self.selected_entry()?;
        let pr_num = entry.pr_number()?;
        self.pr_detail_cache.get(&pr_num)
    }

    /// Signal that the selection changed; request PR detail and git status as needed.
    pub fn request_details_for_selection(&mut self) {
        self.pr_detail_scroll = 0;

        let selected = self.selected_entry().cloned();
        if let Some(entry) = selected {
            // Request PR detail if entry has a PR and it's not cached
            if let Some(pr_num) = entry.pr_number()
                && !self.pr_detail_cache.contains_key(&pr_num)
            {
                self.pr_detail_requested = Some(pr_num);
            }

            // Request git status if entry has a worktree and status not yet loaded
            if let Some(wt_path) = entry.worktree_path()
                && entry.git_status.is_none()
            {
                self.git_status_requested = Some(wt_path.to_string());
            }
        }
    }

    pub fn handle_key(&mut self, key: KeyEvent) {
        // Help overlay takes priority
        if self.show_help {
            match key.code {
                KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('?') => {
                    self.show_help = false;
                }
                _ => {}
            }
            return;
        }

        // Confirm dialog takes priority
        if self.confirm_dialog.is_some() {
            self.handle_confirm_key(key.code);
            return;
        }

        // Action menu takes priority
        if self.action_menu.is_some() {
            self.handle_action_menu_key(key.code);
            return;
        }

        // Search mode takes priority in Main view
        if self.search_active && self.active_view == ActiveView::Main {
            self.handle_search_key(key.code);
            return;
        }

        match key.code {
            KeyCode::Char('?') => self.show_help = true,
            KeyCode::Char('q') => self.should_quit = true,
            KeyCode::Esc => {
                if !self.search_query.is_empty() {
                    // Clear search filter and restore scroll
                    self.search_query.clear();
                    self.sidebar_scroll = self.search_pre_scroll;
                    self.sidebar_offset = 0;
                    self.request_details_for_selection();
                } else if matches!(self.active_view, ActiveView::Log | ActiveView::History) {
                    self.active_view = ActiveView::Main;
                } else {
                    self.should_quit = true;
                }
            }
            KeyCode::Char('l') => self.active_view = ActiveView::Log,
            KeyCode::Char('h') => {
                if self.active_view != ActiveView::History {
                    self.active_view = ActiveView::History;
                    self.history_scroll = 0;
                }
            }
            KeyCode::Char('1') => {
                self.main_filter = MainFilter::Local;
                self.active_view = ActiveView::Main;
                self.search_query.clear();
                self.sidebar_scroll = 0;
                self.sidebar_offset = 0;
                self.rebuild_entries();
                if !self.local_prs_loaded {
                    self.pr_fetch_requested = Some(MainFilter::Local);
                }
                self.request_details_for_selection();
            }
            KeyCode::Char('2') => {
                self.main_filter = MainFilter::MyPr;
                self.active_view = ActiveView::Main;
                self.search_query.clear();
                self.sidebar_scroll = 0;
                self.sidebar_offset = 0;
                self.rebuild_entries();
                if !self.my_prs_loaded {
                    self.pr_fetch_requested = Some(MainFilter::MyPr);
                }
                self.request_details_for_selection();
            }
            KeyCode::Char('3') => {
                self.main_filter = MainFilter::ReviewRequested;
                self.active_view = ActiveView::Main;
                self.search_query.clear();
                self.sidebar_scroll = 0;
                self.sidebar_offset = 0;
                self.rebuild_entries();
                if !self.review_prs_loaded {
                    self.pr_fetch_requested = Some(MainFilter::ReviewRequested);
                }
                self.request_details_for_selection();
            }
            KeyCode::Char('r') => match self.active_view {
                ActiveView::Main => {
                    // Invalidate current filter's PR cache so the fetch is forced
                    match self.main_filter {
                        MainFilter::Local => {
                            self.local_prs.clear();
                            self.local_prs_loaded = false;
                        }
                        MainFilter::MyPr => {
                            self.my_prs.clear();
                            self.my_prs_loaded = false;
                        }
                        MainFilter::ReviewRequested => {
                            self.review_prs.clear();
                            self.review_prs_loaded = false;
                        }
                    }
                    // Clear PR detail cache for ALL filters, not just the current
                    // one — stale detail bodies are risky after a refresh, and the
                    // detail pane will refetch on the next selection.
                    self.pr_detail_cache.clear();
                    // Signal branches/worktrees reload + PR fetch
                    self.branches_reload_requested = true;
                    self.pr_fetch_requested = Some(self.main_filter);
                    self.notification = Some(Notification::success("Refreshing…"));
                }
                ActiveView::Log => {
                    self.commits_reload_requested = true;
                    self.notification = Some(Notification::success("Refreshing…"));
                }
                ActiveView::History => {
                    // History updates live as commands run — no manual refresh needed.
                }
            },
            _ => match self.active_view {
                ActiveView::Main => self.handle_main_key(key.code),
                ActiveView::Log => self.handle_log_key(key.code),
                ActiveView::History => self.handle_history_key(key.code),
            },
        }
    }

    fn handle_main_key(&mut self, code: KeyCode) {
        let filtered_len = self.filtered_entries().len();
        match code {
            KeyCode::Char('j') | KeyCode::Down => {
                if filtered_len > 0 && self.sidebar_scroll + 1 < filtered_len {
                    self.sidebar_scroll += 1;
                    self.request_details_for_selection();
                }
            }
            KeyCode::Char('k') | KeyCode::Up => {
                if self.sidebar_scroll > 0 {
                    self.sidebar_scroll = self.sidebar_scroll.saturating_sub(1);
                    self.request_details_for_selection();
                }
            }
            KeyCode::Char(' ') => {
                if let Some(entry) = self.selected_entry().cloned()
                    && !entry.is_current()
                    && !self.is_protected_branch(&entry.name)
                {
                    if self.branch_selected.contains(&entry.name) {
                        self.branch_selected.remove(&entry.name);
                    } else {
                        self.branch_selected.insert(entry.name);
                    }
                }
            }
            KeyCode::Char('a') => {
                let to_select: Vec<String> = self
                    .entries
                    .iter()
                    .filter(|e| {
                        (e.is_merged() || e.pr_is_merged())
                            && !e.is_current()
                            && !self.is_protected_branch(&e.name)
                    })
                    .map(|e| e.name.clone())
                    .collect();
                self.branch_selected.extend(to_select);
            }
            KeyCode::Char('d') => {
                if !self.branch_selected.is_empty() {
                    let count = self.branch_selected.len();
                    let names: Vec<&str> =
                        self.branch_selected.iter().map(|s| s.as_str()).collect();
                    let preview = if count <= 3 {
                        names.join(", ")
                    } else {
                        format!("{} and {} more", names[..2].join(", "), count - 2)
                    };
                    self.confirm_dialog = Some(ConfirmDialog::new(
                        "Delete Branches",
                        format!("Delete {count} branch(es)? [{preview}]"),
                    ));
                } else if !self.wt_loading
                    && let Some(entry) = self.selected_entry().cloned()
                    && let Some(wt_path) = entry.worktree_path()
                    && !entry.is_current()
                {
                    let path = wt_path.to_string();
                    self.confirm_dialog = Some(ConfirmDialog::new(
                        "Delete Worktree",
                        format!("Remove worktree at {path}?"),
                    ));
                    self.wt_delete_pending_path = Some(path);
                }
            }
            KeyCode::Char('w') => {
                if !self.wt_loading
                    && let Some(entry) = self.selected_entry()
                    && entry.worktree.is_none()
                    && !entry.is_current()
                    && (entry.local_branch.is_some() || entry.pull_request.is_some())
                {
                    self.wt_create_requested = Some(entry.name.clone());
                    self.notification =
                        Some(Notification::success("Creating worktree...".to_string()));
                }
            }
            KeyCode::Enter => {
                self.open_action_menu();
            }
            KeyCode::Char('m') => {
                if matches!(
                    self.main_filter,
                    MainFilter::MyPr | MainFilter::ReviewRequested
                ) {
                    self.show_merged = !self.show_merged;
                    // Invalidate both caches since merged state changed
                    self.my_prs.clear();
                    self.my_prs_loaded = false;
                    self.review_prs.clear();
                    self.review_prs_loaded = false;
                    self.rebuild_entries();
                    self.pr_fetch_requested = Some(self.main_filter);
                    self.sidebar_scroll = 0;
                    self.sidebar_offset = 0;
                }
            }
            KeyCode::Char('t') => {
                if self.main_filter == MainFilter::ReviewRequested {
                    self.include_team_reviews = !self.include_team_reviews;
                    self.review_prs.clear();
                    self.review_prs_loaded = false;
                    self.rebuild_entries();
                    self.pr_fetch_requested = Some(MainFilter::ReviewRequested);
                    self.sidebar_scroll = 0;
                    self.sidebar_offset = 0;
                }
            }
            KeyCode::Char('/') => {
                self.search_pre_scroll = self.sidebar_scroll;
                self.search_active = true;
                self.search_query.clear();
            }
            _ => {}
        }
    }

    fn handle_log_key(&mut self, code: KeyCode) {
        match code {
            KeyCode::Char('j') | KeyCode::Down => {
                if self.log_scroll + 1 < self.commits.len() {
                    self.log_scroll += 1;
                }
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.log_scroll = self.log_scroll.saturating_sub(1);
            }
            _ => {}
        }
    }

    fn handle_history_key(&mut self, code: KeyCode) {
        let len = crate::git::command::command_history_len();
        match code {
            KeyCode::Char('j') | KeyCode::Down => {
                if len > 0 && self.history_scroll + 1 < len {
                    self.history_scroll += 1;
                }
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.history_scroll = self.history_scroll.saturating_sub(1);
            }
            _ => {}
        }
    }

    fn handle_search_key(&mut self, code: KeyCode) {
        match code {
            KeyCode::Esc => {
                self.search_active = false;
                self.search_query.clear();
                self.sidebar_scroll = self.search_pre_scroll;
                self.request_details_for_selection();
            }
            KeyCode::Enter => {
                self.search_active = false;
                self.request_details_for_selection();
            }
            KeyCode::Backspace => {
                self.search_query.pop();
                self.sidebar_scroll = 0;
                self.sidebar_offset = 0;
                self.request_details_for_selection();
            }
            KeyCode::Char(c) => {
                self.search_query.push(c);
                self.sidebar_scroll = 0;
                self.sidebar_offset = 0;
                self.request_details_for_selection();
            }
            KeyCode::Down => {
                let len = self.filtered_entries().len();
                if len > 0 && self.sidebar_scroll + 1 < len {
                    self.sidebar_scroll += 1;
                    self.request_details_for_selection();
                }
            }
            KeyCode::Up => {
                if self.sidebar_scroll > 0 {
                    self.sidebar_scroll -= 1;
                    self.request_details_for_selection();
                }
            }
            _ => {}
        }
    }

    fn open_action_menu(&mut self) {
        let entry = match self.selected_entry().cloned() {
            Some(e) => e,
            None => return,
        };
        let mut items = Vec::new();

        if entry.pull_request.is_some() {
            items.push(ActionItem::OpenPrInBrowser);
        }
        items.push(ActionItem::CopyBranchName);
        if entry.worktree.is_some() {
            items.push(ActionItem::CdIntoWorktree);
        }
        if !self.wt_loading
            && entry.worktree.is_none()
            && !entry.is_current()
            && (entry.local_branch.is_some() || entry.pull_request.is_some())
        {
            items.push(ActionItem::CreateWorktree);
        }
        if !self.wt_loading && entry.worktree.is_some() && !entry.is_current() {
            items.push(ActionItem::DeleteWorktree);
        }
        if entry.local_branch.is_some()
            && !entry.is_current()
            && !self.is_protected_branch(&entry.name)
        {
            items.push(ActionItem::DeleteBranch);
        }

        if !items.is_empty() {
            self.action_menu = Some(ActionMenu {
                items,
                scroll: 0,
                target_name: entry.name.clone(),
            });
        }
    }

    fn handle_action_menu_key(&mut self, code: KeyCode) {
        let menu = match &mut self.action_menu {
            Some(m) => m,
            None => return,
        };
        match code {
            KeyCode::Char('j') | KeyCode::Down => {
                if menu.scroll + 1 < menu.items.len() {
                    menu.scroll += 1;
                }
            }
            KeyCode::Char('k') | KeyCode::Up => {
                menu.scroll = menu.scroll.saturating_sub(1);
            }
            KeyCode::Enter => {
                let action = menu.items[menu.scroll];
                let target = menu.target_name.clone();
                self.action_menu = None;
                self.execute_action(action, &target);
            }
            KeyCode::Esc => {
                self.action_menu = None;
            }
            _ => {}
        }
    }

    fn execute_action(&mut self, action: ActionItem, target_name: &str) {
        let entry = match self.entries.iter().find(|e| e.name == target_name).cloned() {
            Some(e) => e,
            None => return,
        };
        match action {
            ActionItem::CreateWorktree => {
                self.wt_create_requested = Some(entry.name.clone());
                self.notification = Some(Notification::success("Creating worktree...".to_string()));
            }
            ActionItem::CdIntoWorktree => {
                if let Some(path) = entry.worktree_path() {
                    self.cd_path = Some(path.to_string());
                    self.should_quit = true;
                }
            }
            ActionItem::DeleteWorktree => {
                if let Some(wt_path) = entry.worktree_path() {
                    let path = wt_path.to_string();
                    // Clear branch_selected to avoid confirm_key misrouting
                    self.branch_selected.clear();
                    self.confirm_dialog = Some(ConfirmDialog::new(
                        "Delete Worktree",
                        format!("Remove worktree at {path}?"),
                    ));
                    self.wt_delete_pending_path = Some(path);
                }
            }
            ActionItem::DeleteBranch => {
                let name = entry.name.clone();
                self.branch_selected.clear();
                self.branch_selected.insert(name.clone());
                self.confirm_dialog = Some(ConfirmDialog::new(
                    "Delete Branch",
                    format!("Delete branch {name}?"),
                ));
            }
            ActionItem::OpenPrInBrowser => {
                if let Some(pr) = &entry.pull_request {
                    self.open_pr_requested = Some(pr.number);
                }
            }
            ActionItem::CopyBranchName => {
                self.copy_branch_requested = Some(entry.name.clone());
                self.notification = Some(Notification::success(format!("Copied: {}", entry.name)));
            }
        }
    }

    fn handle_confirm_key(&mut self, code: KeyCode) {
        match code {
            KeyCode::Char('y') => {
                if !self.branch_selected.is_empty() {
                    self.branch_delete_requested = true;
                } else if let Some(path) = self.wt_force_delete_pending_path.take() {
                    self.wt_force_delete_requested = Some(path);
                } else if let Some(path) = self.wt_delete_pending_path.take() {
                    self.wt_delete_requested = Some(path);
                }
                self.confirm_dialog = None;
            }
            KeyCode::Char('n') | KeyCode::Esc => {
                self.wt_delete_pending_path = None;
                self.wt_force_delete_pending_path = None;
                self.confirm_dialog = None;
            }
            _ => {}
        }
    }

    pub fn is_protected_branch(&self, name: &str) -> bool {
        self.config.protected_branches.iter().any(|b| b == name)
    }
}
