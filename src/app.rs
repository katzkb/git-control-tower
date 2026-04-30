use crossterm::event::{KeyCode, KeyEvent};

use std::collections::{BTreeMap, HashMap, HashSet};
use std::time::Instant;

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
    CreateBranch,
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
            Self::CreateBranch => "Create branch from this",
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

pub struct BranchCreateInput {
    pub source: String,
    pub name: String,
    pub cursor: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpStep {
    RunningWtRemove,
    RunningWtForceRemove,
    RunningBranchDelete,
    Done { success: bool },
}

#[derive(Debug, Clone)]
pub struct OpProgress {
    pub label: String,
    #[allow(dead_code)]
    pub wt_path: Option<String>,
    #[allow(dead_code)]
    pub branch_name: Option<String>,
    pub current_step: OpStep,
    pub step_started_at: Instant,
    pub op_started_at: Instant,
    pub last_command: Option<String>,
    pub error: Option<String>,
}

impl OpProgress {
    pub fn new(label: String, wt_path: Option<String>, branch_name: Option<String>) -> Self {
        let now = Instant::now();
        Self {
            label,
            wt_path,
            branch_name,
            current_step: OpStep::RunningWtRemove, // overwritten by first OpStepBegin
            step_started_at: now,
            op_started_at: now,
            last_command: None,
            error: None,
        }
    }

    pub fn is_done(&self) -> bool {
        matches!(self.current_step, OpStep::Done { .. })
    }
}

#[derive(Debug, Default)]
pub struct ProgressTracker {
    pub ops: BTreeMap<u64, OpProgress>,
    pub next_id: u64,
    pub started_at: Option<Instant>,
}

impl ProgressTracker {
    pub fn is_active(&self) -> bool {
        !self.ops.is_empty()
    }

    pub fn total(&self) -> usize {
        self.ops.len()
    }

    pub fn done_count(&self) -> usize {
        self.ops.values().filter(|p| p.is_done()).count()
    }

    pub fn allocate_ids(&mut self, n: usize) -> std::ops::Range<u64> {
        let start = self.next_id;
        self.next_id += n as u64;
        if self.started_at.is_none() && n > 0 {
            self.started_at = Some(Instant::now());
        }
        start..self.next_id
    }

    pub fn insert(&mut self, op_id: u64, op: OpProgress) {
        self.ops.insert(op_id, op);
    }

    pub fn update_step(&mut self, op_id: u64, step: OpStep, command: String) {
        if let Some(op) = self.ops.get_mut(&op_id) {
            op.current_step = step;
            op.last_command = Some(command);
            op.step_started_at = Instant::now();
        }
    }

    pub fn finish(&mut self, op_id: u64, success: bool, error: Option<String>) {
        if let Some(op) = self.ops.get_mut(&op_id) {
            op.current_step = OpStep::Done { success };
            op.error = error;
        }
    }

    /// Force-finish any non-Done ops as failures. Used when OpAllDone arrives
    /// but some tasks panicked and never sent OpFinished.
    pub fn sweep_unfinished(&mut self) {
        for op in self.ops.values_mut() {
            if !op.is_done() {
                op.current_step = OpStep::Done { success: false };
                if op.error.is_none() {
                    op.error = Some("interrupted".to_string());
                }
            }
        }
    }

    pub fn clear(&mut self) {
        self.ops.clear();
        self.started_at = None;
    }
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
    pub branch_create_input: Option<BranchCreateInput>,
    pub notification: Option<Notification>,
    pub show_help: bool,

    // Exit with cd path
    pub cd_path: Option<String>,

    // Action requests
    pub wt_delete_requested: Option<String>,
    pub wt_delete_pending_path: Option<String>, // path stored when confirm dialog shown
    pub wt_force_delete_requested: Option<String>,
    pub wt_force_delete_pending_path: Option<String>,
    pub wt_cd_pending_path: Option<String>,
    pub wt_create_requested: Option<String>,
    /// Worktree paths with an in-flight create/delete, gated per-path so unrelated
    /// worktrees stay actionable in the UI while one is still running.
    pub wt_inflight: HashSet<String>,
    pub progress: ProgressTracker,
    pub quit_pressed_during_progress: bool,
    pub branch_selected: HashSet<String>,
    pub branch_delete_requested: bool,
    pub open_pr_requested: Option<u64>,
    pub copy_branch_requested: Option<String>,
    pub branch_create_requested: Option<(String, String)>, // (source, name)
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
            branch_create_input: None,
            notification: None,
            show_help: false,
            cd_path: None,
            wt_delete_requested: None,
            wt_delete_pending_path: None,
            wt_force_delete_requested: None,
            wt_force_delete_pending_path: None,
            wt_cd_pending_path: None,
            wt_create_requested: None,
            wt_inflight: HashSet::new(),
            progress: ProgressTracker::default(),
            quit_pressed_during_progress: false,
            branch_selected: HashSet::new(),
            branch_delete_requested: false,
            open_pr_requested: None,
            copy_branch_requested: None,
            branch_create_requested: None,
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
        if self.wt_inflight.is_empty()
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

        // Branch-create input modal takes priority
        if self.branch_create_input.is_some() {
            self.handle_branch_create_input_key(key.code);
            return;
        }

        // Search mode takes priority in Main view
        if self.search_active && self.active_view == ActiveView::Main {
            self.handle_search_key(key.code);
            return;
        }

        match key.code {
            KeyCode::Char('?') => self.show_help = true,
            KeyCode::Char('q') => {
                if !self.progress.is_active() || self.quit_pressed_during_progress {
                    self.should_quit = true;
                } else {
                    self.quit_pressed_during_progress = true;
                }
            }
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
            KeyCode::Char('j') | KeyCode::Down
                if filtered_len > 0 && self.sidebar_scroll + 1 < filtered_len =>
            {
                self.sidebar_scroll += 1;
                self.request_details_for_selection();
            }
            KeyCode::Char('k') | KeyCode::Up if self.sidebar_scroll > 0 => {
                self.sidebar_scroll = self.sidebar_scroll.saturating_sub(1);
                self.request_details_for_selection();
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
                // Only entries that will actually be processed (i.e. have a
                // local branch or a worktree) contribute to the dialog — PR-only
                // selections are silently ignored by the dispatcher, so they
                // must not appear in the preview or inflate the counts.
                let mut branch_count = 0usize;
                let mut worktree_count = 0usize;
                let mut unmerged_count = 0usize;
                let mut deletable_names: Vec<&str> = Vec::new();
                if !self.branch_selected.is_empty() {
                    for name in &self.branch_selected {
                        if let Some(entry) = self.entries.iter().find(|e| &e.name == name) {
                            let has_branch = entry.local_branch.is_some();
                            let has_worktree = entry.worktree.is_some();
                            if !has_branch && !has_worktree {
                                continue;
                            }
                            if has_branch {
                                branch_count += 1;
                                if !entry.is_merged() && !entry.pr_is_merged() {
                                    unmerged_count += 1;
                                }
                            }
                            if has_worktree {
                                worktree_count += 1;
                            }
                            deletable_names.push(name.as_str());
                        }
                    }
                }

                if branch_count + worktree_count > 0 {
                    let count = deletable_names.len();
                    let preview = if count <= 3 {
                        deletable_names.join(", ")
                    } else {
                        format!("{} and {} more", deletable_names[..2].join(", "), count - 2)
                    };
                    let msg = compose_bulk_delete_message(
                        branch_count,
                        worktree_count,
                        unmerged_count,
                        &preview,
                    );
                    let title = if worktree_count == 0 {
                        "Delete Branches"
                    } else if branch_count == 0 {
                        "Delete Worktrees"
                    } else {
                        "Delete Branches + Worktrees"
                    };
                    self.confirm_dialog = Some(ConfirmDialog::new(title, msg));
                } else if !self.branch_selected.is_empty() {
                    // Non-empty selection but nothing deletable (e.g. PR-only entries
                    // with no local branch and no worktree). Tell the user rather
                    // than silently no-op.
                    self.notification = Some(Notification::error(
                        "Nothing to delete in selection".to_string(),
                    ));
                } else if let Some(entry) = self.selected_entry().cloned()
                    && let Some(wt_path) = entry.worktree_path()
                    && !entry.is_current()
                    && !self.wt_inflight.contains(wt_path)
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
                if let Some(entry) = self.selected_entry()
                    && entry.worktree.is_none()
                    && !entry.is_current()
                    && (entry.local_branch.is_some() || entry.pull_request.is_some())
                    && !self
                        .wt_inflight
                        .contains(&self.config.worktree_path(&entry.name))
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
            KeyCode::Char('t') if self.main_filter == MainFilter::ReviewRequested => {
                self.include_team_reviews = !self.include_team_reviews;
                self.review_prs.clear();
                self.review_prs_loaded = false;
                self.rebuild_entries();
                self.pr_fetch_requested = Some(MainFilter::ReviewRequested);
                self.sidebar_scroll = 0;
                self.sidebar_offset = 0;
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
            KeyCode::Char('j') | KeyCode::Down if self.log_scroll + 1 < self.commits.len() => {
                self.log_scroll += 1;
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
            KeyCode::Char('j') | KeyCode::Down if len > 0 && self.history_scroll + 1 < len => {
                self.history_scroll += 1;
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
                self.open_action_menu();
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
            KeyCode::Up if self.sidebar_scroll > 0 => {
                self.sidebar_scroll -= 1;
                self.request_details_for_selection();
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
        if entry.worktree.is_none()
            && !entry.is_current()
            && (entry.local_branch.is_some() || entry.pull_request.is_some())
            && !self
                .wt_inflight
                .contains(&self.config.worktree_path(&entry.name))
        {
            items.push(ActionItem::CreateWorktree);
        }
        if let Some(wt_path) = entry.worktree_path()
            && !entry.is_current()
            && !self.wt_inflight.contains(wt_path)
        {
            items.push(ActionItem::DeleteWorktree);
        }
        if entry.local_branch.is_some() {
            items.push(ActionItem::CreateBranch);
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
            KeyCode::Char('j') | KeyCode::Down if menu.scroll + 1 < menu.items.len() => {
                menu.scroll += 1;
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
            ActionItem::CreateBranch => {
                self.branch_create_input = Some(BranchCreateInput {
                    source: entry.name.clone(),
                    name: String::new(),
                    cursor: 0,
                });
            }
            ActionItem::DeleteBranch => {
                let name = entry.name.clone();
                let is_unmerged = !entry.is_merged() && !entry.pr_is_merged();
                self.branch_selected.clear();
                self.branch_selected.insert(name.clone());
                let msg = if is_unmerged {
                    format!("Delete branch {name}? (unmerged — will force delete)")
                } else {
                    format!("Delete branch {name}?")
                };
                self.confirm_dialog = Some(ConfirmDialog::new("Delete Branch", msg));
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

    fn handle_branch_create_input_key(&mut self, code: KeyCode) {
        let input = match &mut self.branch_create_input {
            Some(i) => i,
            None => return,
        };
        let char_len = input.name.chars().count();
        input.cursor = input.cursor.min(char_len);
        match code {
            KeyCode::Esc => {
                self.branch_create_input = None;
            }
            KeyCode::Enter if !input.name.is_empty() => {
                let source = input.source.clone();
                let name = input.name.clone();
                self.branch_create_input = None;
                self.branch_create_requested = Some((source, name));
            }
            KeyCode::Left => {
                input.cursor = input.cursor.saturating_sub(1);
            }
            KeyCode::Right if input.cursor < char_len => {
                input.cursor += 1;
            }
            KeyCode::Home => {
                input.cursor = 0;
            }
            KeyCode::End => {
                input.cursor = char_len;
            }
            KeyCode::Backspace if input.cursor > 0 => {
                remove_char_at(&mut input.name, input.cursor - 1);
                input.cursor -= 1;
            }
            KeyCode::Delete if input.cursor < char_len => {
                remove_char_at(&mut input.name, input.cursor);
            }
            KeyCode::Char(c) => {
                insert_char_at(&mut input.name, input.cursor, c);
                input.cursor += 1;
            }
            _ => {}
        }
    }

    fn handle_confirm_key(&mut self, code: KeyCode) {
        match code {
            KeyCode::Char('y') => {
                // Move-to-worktree takes precedence — it can race with a
                // stale `branch_selected` from before the create finished.
                if let Some(path) = self.wt_cd_pending_path.take() {
                    self.cd_path = Some(path);
                    self.should_quit = true;
                } else if !self.branch_selected.is_empty() {
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
                self.wt_cd_pending_path = None;
                // Declining force-delete ends the op — release the path so its
                // action items reappear.
                if let Some(path) = self.wt_force_delete_pending_path.take() {
                    self.wt_inflight.remove(&path);
                }
                self.confirm_dialog = None;
            }
            _ => {}
        }
    }

    pub fn is_protected_branch(&self, name: &str) -> bool {
        self.config.protected_branches.iter().any(|b| b == name)
    }
}

fn insert_char_at(s: &mut String, char_idx: usize, c: char) {
    let byte_idx = char_to_byte_idx(s, char_idx);
    s.insert(byte_idx, c);
}

fn remove_char_at(s: &mut String, char_idx: usize) {
    let byte_idx = char_to_byte_idx(s, char_idx);
    if byte_idx < s.len() {
        s.remove(byte_idx);
    }
}

fn char_to_byte_idx(s: &str, char_idx: usize) -> usize {
    s.char_indices()
        .nth(char_idx)
        .map(|(b, _)| b)
        .unwrap_or(s.len())
}

pub(crate) fn compose_bulk_delete_message(
    branches: usize,
    worktrees: usize,
    unmerged: usize,
    preview: &str,
) -> String {
    let branch_label = if branches == 1 { "branch" } else { "branches" };
    let worktree_label = if worktrees == 1 {
        "worktree"
    } else {
        "worktrees"
    };
    let mut head_parts = Vec::with_capacity(2);
    if branches > 0 {
        head_parts.push(format!("{branches} {branch_label}"));
    }
    if worktrees > 0 {
        head_parts.push(format!("{worktrees} {worktree_label}"));
    }
    let head = head_parts.join(" + ");

    let head_with_clauses = if unmerged > 0 {
        format!("Delete {head}? ({unmerged} unmerged — will force delete)")
    } else {
        format!("Delete {head}?")
    };
    format!("{head_with_clauses}\n[{preview}]")
}

#[cfg(test)]
mod text_edit_tests {
    use super::*;

    #[test]
    fn insert_at_start() {
        let mut s = String::from("bc");
        insert_char_at(&mut s, 0, 'a');
        assert_eq!(s, "abc");
    }

    #[test]
    fn insert_at_middle() {
        let mut s = String::from("ac");
        insert_char_at(&mut s, 1, 'b');
        assert_eq!(s, "abc");
    }

    #[test]
    fn insert_at_end() {
        let mut s = String::from("ab");
        insert_char_at(&mut s, 2, 'c');
        assert_eq!(s, "abc");
    }

    #[test]
    fn insert_unicode() {
        let mut s = String::from("αγ");
        insert_char_at(&mut s, 1, 'β');
        assert_eq!(s, "αβγ");
    }

    #[test]
    fn remove_at_start() {
        let mut s = String::from("abc");
        remove_char_at(&mut s, 0);
        assert_eq!(s, "bc");
    }

    #[test]
    fn remove_at_middle() {
        let mut s = String::from("abc");
        remove_char_at(&mut s, 1);
        assert_eq!(s, "ac");
    }

    #[test]
    fn remove_at_end_is_noop() {
        let mut s = String::from("abc");
        remove_char_at(&mut s, 3);
        assert_eq!(s, "abc");
    }

    #[test]
    fn remove_unicode() {
        let mut s = String::from("αβγ");
        remove_char_at(&mut s, 1);
        assert_eq!(s, "αγ");
    }

    #[test]
    fn progress_tracker_allocate_ids_advances() {
        let mut t = ProgressTracker::default();
        let r = t.allocate_ids(3);
        assert_eq!(r, 0..3);
        let r2 = t.allocate_ids(2);
        assert_eq!(r2, 3..5);
        assert_eq!(t.next_id, 5);
    }

    #[test]
    fn progress_tracker_allocate_ids_sets_started_at_once() {
        let mut t = ProgressTracker::default();
        assert!(t.started_at.is_none());
        let _ = t.allocate_ids(2);
        let first = t
            .started_at
            .expect("started_at set on first non-empty allocation");
        let _ = t.allocate_ids(1);
        assert_eq!(t.started_at, Some(first));
    }

    #[test]
    fn progress_tracker_state_transitions() {
        let mut t = ProgressTracker::default();
        let ids: Vec<u64> = t.allocate_ids(2).collect();
        t.insert(
            ids[0],
            OpProgress::new("a".into(), Some("/wt/a".into()), Some("a".into())),
        );
        t.insert(
            ids[1],
            OpProgress::new("b".into(), Some("/wt/b".into()), Some("b".into())),
        );

        assert_eq!(t.total(), 2);
        assert_eq!(t.done_count(), 0);
        assert!(t.is_active());

        t.update_step(
            ids[0],
            OpStep::RunningWtForceRemove,
            "git worktree remove --force /wt/a".into(),
        );
        assert_eq!(t.ops[&ids[0]].current_step, OpStep::RunningWtForceRemove);
        assert_eq!(
            t.ops[&ids[0]].last_command.as_deref(),
            Some("git worktree remove --force /wt/a")
        );

        t.finish(ids[0], true, None);
        assert!(t.ops[&ids[0]].is_done());
        assert_eq!(t.done_count(), 1);

        t.finish(ids[1], false, Some("nope".into()));
        assert_eq!(t.done_count(), 2);
        assert_eq!(t.ops[&ids[1]].error.as_deref(), Some("nope"));
    }

    #[test]
    fn progress_tracker_sweep_unfinished_marks_remaining_as_failed() {
        let mut t = ProgressTracker::default();
        let ids: Vec<u64> = t.allocate_ids(2).collect();
        t.insert(ids[0], OpProgress::new("a".into(), None, Some("a".into())));
        t.insert(ids[1], OpProgress::new("b".into(), None, Some("b".into())));
        t.finish(ids[0], true, None);

        t.sweep_unfinished();
        assert!(t.ops[&ids[1]].is_done());
        assert_eq!(t.ops[&ids[1]].current_step, OpStep::Done { success: false });
        assert_eq!(t.ops[&ids[1]].error.as_deref(), Some("interrupted"));
    }

    #[test]
    fn progress_tracker_clear_resets_state() {
        let mut t = ProgressTracker::default();
        let ids: Vec<u64> = t.allocate_ids(1).collect();
        t.insert(ids[0], OpProgress::new("a".into(), None, None));
        t.clear();
        assert!(!t.is_active());
        assert!(t.started_at.is_none());
        assert_eq!(t.total(), 0);
    }

    #[test]
    fn quit_during_progress_requires_two_presses() {
        use crate::config::Config;
        let mut app = App::new(Config::default());
        let id = app.progress.allocate_ids(1).start;
        app.progress
            .insert(id, OpProgress::new("a".into(), None, None));

        // First 'q': sets the flag, no quit.
        app.handle_key(crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char('q'),
            crossterm::event::KeyModifiers::NONE,
        ));
        assert!(!app.should_quit);
        assert!(app.quit_pressed_during_progress);

        // Second 'q': force quits.
        app.handle_key(crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char('q'),
            crossterm::event::KeyModifiers::NONE,
        ));
        assert!(app.should_quit);
    }

    #[test]
    fn quit_when_no_progress_quits_immediately() {
        use crate::config::Config;
        let mut app = App::new(Config::default());
        app.handle_key(crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char('q'),
            crossterm::event::KeyModifiers::NONE,
        ));
        assert!(app.should_quit);
    }
}

#[cfg(test)]
mod bulk_delete_message_tests {
    use super::compose_bulk_delete_message;

    #[test]
    fn branches_only_clean() {
        assert_eq!(
            compose_bulk_delete_message(3, 0, 0, "a, b, c"),
            "Delete 3 branches?\n[a, b, c]"
        );
    }

    #[test]
    fn single_branch_pluralizes() {
        assert_eq!(
            compose_bulk_delete_message(1, 0, 0, "a"),
            "Delete 1 branch?\n[a]"
        );
    }

    #[test]
    fn branches_with_unmerged() {
        assert_eq!(
            compose_bulk_delete_message(3, 0, 1, "a, b, c"),
            "Delete 3 branches? (1 unmerged — will force delete)\n[a, b, c]"
        );
    }

    #[test]
    fn branches_plus_worktrees() {
        assert_eq!(
            compose_bulk_delete_message(3, 2, 0, "a, b, c"),
            "Delete 3 branches + 2 worktrees?\n[a, b, c]"
        );
    }

    #[test]
    fn branches_worktrees_and_unmerged() {
        assert_eq!(
            compose_bulk_delete_message(3, 2, 1, "a, b, c"),
            "Delete 3 branches + 2 worktrees? (1 unmerged — will force delete)\n[a, b, c]"
        );
    }

    #[test]
    fn worktree_only_singular() {
        assert_eq!(
            compose_bulk_delete_message(0, 1, 0, "a"),
            "Delete 1 worktree?\n[a]"
        );
    }

    #[test]
    fn single_branch_and_worktree() {
        assert_eq!(
            compose_bulk_delete_message(1, 1, 0, "a"),
            "Delete 1 branch + 1 worktree?\n[a]"
        );
    }
}
