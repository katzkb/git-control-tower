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

pub enum SidebarRow<'a> {
    Header { repo_id: crate::git::types::RepoId },
    Entry(&'a crate::git::types::BranchEntry),
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
    /// `(repo_id, branch_name)` captured when the menu was opened. Carrying
    /// both fields prevents wrong-repo lookup when two repos share a branch name.
    pub target: (crate::git::types::RepoId, String),
    pub footer: Option<String>,
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
    pub current_step: OpStep,
    pub op_started_at: Instant,
    /// Set when the op reaches `Done`; freezes the elapsed-time display.
    pub finished_at: Option<Instant>,
    pub last_command: Option<String>,
    pub error: Option<String>,
}

impl OpProgress {
    pub fn new(label: String) -> Self {
        Self {
            label,
            current_step: OpStep::RunningWtRemove, // overwritten by first OpStepBegin
            op_started_at: Instant::now(),
            finished_at: None,
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
    next_id: u64,
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
        }
    }

    pub fn finish(&mut self, op_id: u64, success: bool, error: Option<String>) {
        if let Some(op) = self.ops.get_mut(&op_id) {
            op.current_step = OpStep::Done { success };
            op.error = error;
            if op.finished_at.is_none() {
                op.finished_at = Some(Instant::now());
            }
        }
    }

    /// Force-finish any non-Done ops as failures. Used when OpAllDone arrives
    /// but some tasks panicked and never sent OpFinished.
    pub fn sweep_unfinished(&mut self) {
        for op in self.ops.values_mut() {
            if !op.is_done() {
                op.current_step = OpStep::Done { success: false };
                op.finished_at = Some(Instant::now());
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

    // PR Detail (for detail pane, cached by (RepoId, PR number))
    pub pr_detail_cache: HashMap<(crate::git::types::RepoId, u64), PrDetail>,
    pub pr_detail_scroll: usize,
    pub pr_detail_requested: Option<(crate::git::types::RepoId, u64)>,

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
    /// `(repo_id, branch_name)` for the worktree to create. Carries `RepoId` so
    /// the main-loop lookup matches the correct repo when branch names collide.
    pub wt_create_requested: Option<(crate::git::types::RepoId, String)>,
    /// Worktree paths with an in-flight create/delete, gated per-path so unrelated
    /// worktrees stay actionable in the UI while one is still running.
    pub wt_inflight: HashSet<String>,
    pub progress: ProgressTracker,
    pub quit_pressed_during_progress: bool,
    pub branch_selected: HashSet<String>,
    pub branch_delete_requested: bool,
    pub open_pr_requested: Option<(crate::git::types::RepoId, u64)>,
    pub copy_branch_requested: Option<String>,
    pub branch_create_requested: Option<(String, String)>, // (source, name)
    pub branches_reload_requested: bool,
    pub commits_reload_requested: bool,

    // Spinner animation
    spinner_tick: usize,

    // Loaded TOML config (protected_branches, worktree, …)
    pub config: crate::config::Config,

    // Cross-repo context (set at startup)
    pub active_repo: Option<crate::git::types::RepoId>,
    pub clone_root: Option<std::path::PathBuf>,

    // Per-repo metadata (populated lazily as repos are selected)
    pub repos: std::collections::HashMap<crate::git::types::RepoId, crate::git::types::RepoMeta>,

    // Worktree lists per repo (populated lazily as cross-repo PRs are selected)
    pub wt_lists_per_repo:
        std::collections::HashMap<crate::git::types::RepoId, Vec<crate::git::types::Worktree>>,

    // Cross-repo worktree list lazy-load state
    pub wt_list_inflight: HashSet<crate::git::types::RepoId>,
    pub wt_list_requested: Option<crate::git::types::RepoId>,
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
            active_repo: None,
            clone_root: None,
            repos: HashMap::new(),
            wt_lists_per_repo: HashMap::new(),
            wt_list_inflight: HashSet::new(),
            wt_list_requested: None,
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
        // When `active_repo` is absent (startup couldn't infer one), unwrap_or_default
        // produces a sentinel empty RepoId. It can't collide with any real PR's repo_id,
        // so cross-repo entries are still keyed correctly and worktree injection no-ops
        // safely.
        let active = self.active_repo.clone().unwrap_or_default();
        self.entries = crate::data::merge_entries(
            &active,
            &self.branches,
            &self.worktrees,
            self.current_prs(),
            &self.wt_lists_per_repo,
        );
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

    /// Build the sidebar rendering rows. Returns a flat list of headers and
    /// entries; cross-repo grouping kicks in only for My PR / Review when
    /// `entries` span more than one repo.
    pub fn sidebar_rows(&self) -> Vec<SidebarRow<'_>> {
        let filtered = self.filtered_entries();
        let group_active = matches!(
            self.main_filter,
            MainFilter::MyPr | MainFilter::ReviewRequested
        );
        let repo_set: std::collections::HashSet<_> =
            filtered.iter().map(|e| e.repo_id.clone()).collect();
        let do_group = group_active && repo_set.len() > 1;

        let mut rows = Vec::with_capacity(filtered.len() + repo_set.len());
        if !do_group {
            for e in filtered {
                rows.push(SidebarRow::Entry(e));
            }
            return rows;
        }
        let mut last_repo: Option<crate::git::types::RepoId> = None;
        for e in filtered {
            if last_repo.as_ref() != Some(&e.repo_id) {
                rows.push(SidebarRow::Header {
                    repo_id: e.repo_id.clone(),
                });
                last_repo = Some(e.repo_id.clone());
            }
            rows.push(SidebarRow::Entry(e));
        }
        rows
    }

    pub fn selected_entry(&self) -> Option<&BranchEntry> {
        let rows = self.sidebar_rows();
        match rows.get(self.sidebar_scroll)? {
            SidebarRow::Entry(e) => Some(*e),
            SidebarRow::Header { .. } => None,
        }
    }

    /// If `sidebar_scroll` lands on a Header, advance to the next Entry. If past
    /// the end, clamp to the last Entry. No-op if already on an Entry.
    pub fn snap_scroll_to_entry(&mut self) {
        // Collect row kinds into a plain bool vec (true = is_header) to avoid holding
        // a borrow on self while mutating sidebar_scroll.
        let is_header: Vec<bool> = self
            .sidebar_rows()
            .iter()
            .map(|r| matches!(r, SidebarRow::Header { .. }))
            .collect();
        if is_header.is_empty() {
            self.sidebar_scroll = 0;
            return;
        }
        while self.sidebar_scroll < is_header.len() && is_header[self.sidebar_scroll] {
            self.sidebar_scroll += 1;
        }
        if self.sidebar_scroll >= is_header.len() {
            self.sidebar_scroll = is_header.len().saturating_sub(1);
        }
    }

    /// Find the next entry-row index after `from`, skipping headers. None if no entry follows.
    fn next_entry_index(&self, from: usize) -> Option<usize> {
        let rows = self.sidebar_rows();
        let mut next = from + 1;
        while next < rows.len() && matches!(rows[next], SidebarRow::Header { .. }) {
            next += 1;
        }
        if next < rows.len() { Some(next) } else { None }
    }

    /// Find the previous entry-row index before `from`, skipping headers. None if no entry precedes.
    fn prev_entry_index(&self, from: usize) -> Option<usize> {
        if from == 0 {
            return None;
        }
        let rows = self.sidebar_rows();
        let mut prev = from - 1;
        while matches!(rows.get(prev), Some(SidebarRow::Header { .. })) {
            if prev == 0 {
                return None;
            }
            prev -= 1;
        }
        Some(prev)
    }

    /// Return the cached PR detail for the currently selected entry, if available.
    pub fn selected_pr_detail(&self) -> Option<&PrDetail> {
        let entry = self.selected_entry()?;
        let pr_num = entry.pr_number()?;
        self.pr_detail_cache.get(&(entry.repo_id.clone(), pr_num))
    }

    /// Signal that the selection changed; request PR detail and git status as needed.
    pub fn request_details_for_selection(&mut self) {
        self.pr_detail_scroll = 0;
        let selected = self.selected_entry().cloned();

        if let Some(entry) = &selected {
            // Request PR detail if entry has a PR and it's not cached
            if let Some(pr_num) = entry.pr_number()
                && !self
                    .pr_detail_cache
                    .contains_key(&(entry.repo_id.clone(), pr_num))
            {
                self.pr_detail_requested = Some((entry.repo_id.clone(), pr_num));
            }

            // Request git status if entry has a worktree and status not yet loaded
            if let Some(wt_path) = entry.worktree_path()
                && entry.git_status.is_none()
            {
                self.git_status_requested = Some(wt_path.to_string());
            }
        }

        // Signal lazy load of cross-repo worktree list if not yet fetched (use the same `selected` binding)
        if let Some(entry) = &selected
            && self.active_repo.as_ref() != Some(&entry.repo_id)
            && !self.wt_lists_per_repo.contains_key(&entry.repo_id)
            && !self.wt_list_inflight.contains(&entry.repo_id)
        {
            self.resolve_local_path(&entry.repo_id);
            if self
                .repos
                .get(&entry.repo_id)
                .and_then(|m| m.local_path.as_ref())
                .is_some()
            {
                self.wt_list_requested = Some(entry.repo_id.clone());
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
                    self.snap_scroll_to_entry();
                    self.request_details_for_selection();
                } else if matches!(self.active_view, ActiveView::Log | ActiveView::History) {
                    self.active_view = ActiveView::Main;
                } else if !self.progress.is_active() || self.quit_pressed_during_progress {
                    self.should_quit = true;
                } else {
                    self.quit_pressed_during_progress = true;
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
                self.snap_scroll_to_entry();
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
                self.snap_scroll_to_entry();
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
                self.snap_scroll_to_entry();
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
                    // Invalidate cross-repo worktree list caches so they re-fetch.
                    self.wt_lists_per_repo.clear();
                    self.wt_list_inflight.clear();
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
        match code {
            KeyCode::Char('j') | KeyCode::Down => {
                if let Some(next) = self.next_entry_index(self.sidebar_scroll) {
                    self.sidebar_scroll = next;
                    self.request_details_for_selection();
                }
            }
            KeyCode::Char('k') | KeyCode::Up if self.sidebar_scroll > 0 => {
                if let Some(prev) = self.prev_entry_index(self.sidebar_scroll) {
                    self.sidebar_scroll = prev;
                    self.request_details_for_selection();
                }
            }
            KeyCode::Char(' ') => {
                if let Some(entry) = self.selected_entry().cloned()
                    && !entry.is_current()
                    && !self.is_protected_branch(&entry.name)
                    && self.active_repo.as_ref() == Some(&entry.repo_id)
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
                            && self.active_repo.as_ref() == Some(&e.repo_id)
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
                let Some(entry) = self.selected_entry().cloned() else {
                    return;
                };
                if entry.worktree.is_some()
                    || entry.is_current()
                    || (entry.local_branch.is_none() && entry.pull_request.is_none())
                {
                    return;
                }
                let is_active = self.active_repo.as_ref() == Some(&entry.repo_id);
                let clone_path: Option<std::path::PathBuf> = if is_active {
                    None
                } else {
                    self.resolve_local_path(&entry.repo_id);
                    self.repos
                        .get(&entry.repo_id)
                        .and_then(|m| m.local_path.clone())
                };
                let cross_repo_no_clone = !is_active && clone_path.is_none();
                if cross_repo_no_clone {
                    self.notification = Some(Notification::error(format!(
                        "{} not cloned. Set [workspace] clone_root.",
                        entry.repo_id
                    )));
                    return;
                }
                let wt_path = if is_active {
                    self.config.worktree_path(&entry.name)
                } else {
                    // unwrap is safe: cross_repo_no_clone is false above
                    self.config
                        .worktree_path_for(clone_path.as_ref().unwrap(), &entry.name)
                };
                if self.wt_inflight.contains(&wt_path) {
                    return;
                }
                self.wt_create_requested = Some((entry.repo_id.clone(), entry.name.clone()));
                self.notification = Some(Notification::success("Creating worktree...".to_string()));
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
                    self.snap_scroll_to_entry();
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
                self.snap_scroll_to_entry();
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
                self.snap_scroll_to_entry();
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
                self.snap_scroll_to_entry();
                self.request_details_for_selection();
            }
            KeyCode::Char(c) => {
                self.search_query.push(c);
                self.sidebar_scroll = 0;
                self.sidebar_offset = 0;
                self.snap_scroll_to_entry();
                self.request_details_for_selection();
            }
            KeyCode::Down => {
                if let Some(next) = self.next_entry_index(self.sidebar_scroll) {
                    self.sidebar_scroll = next;
                    self.request_details_for_selection();
                }
            }
            KeyCode::Up if self.sidebar_scroll > 0 => {
                if let Some(prev) = self.prev_entry_index(self.sidebar_scroll) {
                    self.sidebar_scroll = prev;
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
        let is_active_repo = self.active_repo.as_ref() == Some(&entry.repo_id);
        let clone_path: Option<std::path::PathBuf> = if is_active_repo {
            None // active repo runs in CWD, no clone path needed
        } else {
            // cross-repo: resolve once, then read
            self.resolve_local_path(&entry.repo_id);
            self.repos
                .get(&entry.repo_id)
                .and_then(|m| m.local_path.clone())
        };
        let cross_repo_no_clone = !is_active_repo && clone_path.is_none();

        let mut items = Vec::new();
        let mut footer = None;

        if entry.pull_request.is_some() {
            items.push(ActionItem::OpenPrInBrowser);
        }
        items.push(ActionItem::CopyBranchName);

        let wt_already_exists = entry.worktree.is_some();
        let wt_path_for_inflight: Option<String> = if cross_repo_no_clone {
            None
        } else if is_active_repo {
            Some(self.config.worktree_path(&entry.name))
        } else if let Some(ref root) = clone_path {
            Some(self.config.worktree_path_for(root, &entry.name))
        } else {
            None
        };

        if wt_already_exists {
            items.push(ActionItem::CdIntoWorktree);
        }

        let can_create_wt = !wt_already_exists
            && !entry.is_current()
            && (entry.local_branch.is_some() || entry.pull_request.is_some())
            && !cross_repo_no_clone
            && wt_path_for_inflight
                .as_deref()
                .map(|p| !self.wt_inflight.contains(p))
                .unwrap_or(false);
        if can_create_wt {
            items.push(ActionItem::CreateWorktree);
        }

        // DeleteWorktree is active-repo only (cross-repo wt management is OUT for v1).
        if is_active_repo
            && let Some(wt_path) = entry.worktree_path()
            && !entry.is_current()
            && !self.wt_inflight.contains(wt_path)
        {
            items.push(ActionItem::DeleteWorktree);
        }
        if is_active_repo && entry.local_branch.is_some() {
            items.push(ActionItem::CreateBranch);
        }
        if is_active_repo
            && entry.local_branch.is_some()
            && !entry.is_current()
            && !self.is_protected_branch(&entry.name)
        {
            items.push(ActionItem::DeleteBranch);
        }

        if cross_repo_no_clone {
            let hint_root = self
                .clone_root
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| "[workspace] clone_root".to_string());
            footer = Some(format!(
                "{} not cloned. Clone under {hint_root}.",
                entry.repo_id
            ));
        }

        if !items.is_empty() {
            self.action_menu = Some(ActionMenu {
                items,
                scroll: 0,
                target: (entry.repo_id.clone(), entry.name.clone()),
                footer,
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
                let (repo_id, branch_name) = menu.target.clone();
                self.action_menu = None;
                self.execute_action(action, &repo_id, &branch_name);
            }
            KeyCode::Esc => {
                self.action_menu = None;
            }
            _ => {}
        }
    }

    pub(crate) fn execute_action(
        &mut self,
        action: ActionItem,
        repo_id: &crate::git::types::RepoId,
        name: &str,
    ) {
        let entry = match self
            .entries
            .iter()
            .find(|e| e.repo_id == *repo_id && e.name == name)
            .cloned()
        {
            Some(e) => e,
            None => return,
        };
        match action {
            ActionItem::CreateWorktree => {
                self.wt_create_requested = Some((entry.repo_id.clone(), entry.name.clone()));
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
                    self.open_pr_requested = Some((entry.repo_id.clone(), pr.number));
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

    /// Hosts the user can reasonably be expected to have PRs on, derived from
    /// the origin remotes of every repo we have metadata for. Empty repo map
    /// falls back to `[None]` (default host = github.com) so cross-repo
    /// aggregation behaves identically to the single-host case before any
    /// repo metadata is collected. The output is unique-by-host and sorted
    /// (None first) for deterministic ordering.
    pub fn known_hosts(&self) -> Vec<Option<String>> {
        use std::collections::BTreeSet;
        let set: BTreeSet<Option<String>> = self.repos.keys().map(|id| id.host.clone()).collect();
        if set.is_empty() {
            vec![None]
        } else {
            set.into_iter().collect()
        }
    }

    /// Resolve a repo's local clone path under `clone_root`. Idempotent: only
    /// hits the filesystem once per repo. Sets `local_path_resolved = true`
    /// regardless of outcome to prevent re-tries.
    pub fn resolve_local_path(&mut self, id: &crate::git::types::RepoId) {
        // Snapshot clone_root first (no borrow on self.repos held).
        let root = self.clone_root.clone();
        let Some(meta) = self.repos.get_mut(id) else {
            return;
        };
        if meta.local_path_resolved {
            return;
        }
        meta.local_path_resolved = true;
        let Some(root) = root else {
            return;
        };
        let host = id.host.as_deref().unwrap_or("github.com");
        let candidate = root.join(host).join(&id.owner).join(&id.name);
        if candidate.is_dir() {
            meta.local_path = Some(candidate);
        }
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
        t.insert(ids[0], OpProgress::new("a".into()));
        t.insert(ids[1], OpProgress::new("b".into()));

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
        t.insert(ids[0], OpProgress::new("a".into()));
        t.insert(ids[1], OpProgress::new("b".into()));
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
        t.insert(ids[0], OpProgress::new("a".into()));
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
        app.progress.insert(id, OpProgress::new("a".into()));

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

    #[test]
    fn esc_during_progress_requires_two_presses() {
        use crate::config::Config;
        let mut app = App::new(Config::default());
        let id = app.progress.allocate_ids(1).start;
        app.progress.insert(id, OpProgress::new("a".into()));

        // First Esc: sets the flag, no quit.
        app.handle_key(crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Esc,
            crossterm::event::KeyModifiers::NONE,
        ));
        assert!(!app.should_quit);
        assert!(app.quit_pressed_during_progress);

        // Second Esc: force quits.
        app.handle_key(crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Esc,
            crossterm::event::KeyModifiers::NONE,
        ));
        assert!(app.should_quit);
    }
}

#[cfg(test)]
mod pr_detail_cache_tests {
    use super::*;

    #[test]
    fn pr_detail_cache_keyed_by_repo_id() {
        use crate::config::Config;
        use crate::git::types::{PrDetail, RepoId};
        let mut app = App::new(Config::default());
        let id_a = RepoId {
            host: None,
            owner: "a".into(),
            name: "x".into(),
        };
        let id_b = RepoId {
            host: None,
            owner: "b".into(),
            name: "x".into(),
        };
        let detail_a = PrDetail {
            number: 1,
            title: "A".into(),
            author: "u".into(),
            state: "OPEN".into(),
            body: String::new(),
            additions: 0,
            deletions: 0,
            head_ref: "f".into(),
        };
        let detail_b = PrDetail {
            number: 1,
            title: "B".into(),
            author: "u".into(),
            state: "OPEN".into(),
            body: String::new(),
            additions: 0,
            deletions: 0,
            head_ref: "f".into(),
        };
        app.pr_detail_cache
            .insert((id_a.clone(), 1), detail_a.clone());
        app.pr_detail_cache
            .insert((id_b.clone(), 1), detail_b.clone());
        assert_eq!(app.pr_detail_cache.len(), 2);
        assert_eq!(app.pr_detail_cache.get(&(id_a, 1)).unwrap().title, "A");
        assert_eq!(app.pr_detail_cache.get(&(id_b, 1)).unwrap().title, "B");
    }
}

#[cfg(test)]
mod known_hosts_tests {
    use super::*;
    use crate::config::Config;
    use crate::git::types::{RepoId, RepoMeta};

    fn meta() -> RepoMeta {
        RepoMeta {
            local_path: None,
            local_path_resolved: false,
        }
    }

    #[test]
    fn known_hosts_empty_repos_returns_default_host() {
        // Without any repo metadata we still want a single-host search against
        // the default host (github.com) so My PR / My Review behave as before.
        let app = App::new(Config::default());
        assert_eq!(app.known_hosts(), vec![None]);
    }

    #[test]
    fn known_hosts_dedups_multiple_repos_per_host() {
        let mut app = App::new(Config::default());
        for (owner, name) in [("a", "x"), ("b", "y"), ("c", "z")] {
            app.repos.insert(
                RepoId {
                    host: None,
                    owner: owner.into(),
                    name: name.into(),
                },
                meta(),
            );
        }
        assert_eq!(app.known_hosts(), vec![None]);
    }

    #[test]
    fn known_hosts_returns_unique_hosts_across_mixed_repos() {
        let mut app = App::new(Config::default());
        // Two github.com repos and two GHES repos on the same host.
        for (host, owner, name) in [
            (None, "katzkb", "gct"),
            (None, "katzkb", "dotfiles"),
            (Some("ghe.example.com"), "team", "svc"),
            (Some("ghe.example.com"), "team", "infra"),
            (Some("ghe.other.com"), "alice", "tools"),
        ] {
            app.repos.insert(
                RepoId {
                    host: host.map(|s| s.to_string()),
                    owner: owner.into(),
                    name: name.into(),
                },
                meta(),
            );
        }
        // `known_hosts` collects via BTreeSet, so the output is already sorted
        // with `None` (= github.com) before any `Some(host)` entries — the
        // sort here would be a no-op and is intentionally omitted.
        assert_eq!(
            app.known_hosts(),
            vec![
                None,
                Some("ghe.example.com".to_string()),
                Some("ghe.other.com".to_string()),
            ]
        );
    }
}

#[cfg(test)]
mod resolve_local_path_tests {
    use super::*;

    #[test]
    fn resolve_local_path_ghq_layout_hits() {
        use crate::config::Config;
        use crate::git::types::RepoId;
        let tmp = tempfile::tempdir().unwrap();
        let host_dir = tmp.path().join("github.com").join("owner").join("name");
        std::fs::create_dir_all(&host_dir).unwrap();

        let mut app = App::new(Config::default());
        app.clone_root = Some(tmp.path().to_path_buf());
        let id = RepoId {
            host: None,
            owner: "owner".into(),
            name: "name".into(),
        };
        app.repos.insert(
            id.clone(),
            crate::git::types::RepoMeta {
                local_path: None,
                local_path_resolved: false,
            },
        );
        app.resolve_local_path(&id);
        let meta = app.repos.get(&id).unwrap();
        assert!(meta.local_path_resolved);
        assert_eq!(meta.local_path.as_ref().unwrap(), &host_dir);
    }

    #[test]
    fn resolve_local_path_misses_when_dir_absent() {
        use crate::config::Config;
        use crate::git::types::RepoId;
        let tmp = tempfile::tempdir().unwrap();
        let mut app = App::new(Config::default());
        app.clone_root = Some(tmp.path().to_path_buf());
        let id = RepoId {
            host: None,
            owner: "x".into(),
            name: "y".into(),
        };
        app.repos.insert(
            id.clone(),
            crate::git::types::RepoMeta {
                local_path: None,
                local_path_resolved: false,
            },
        );
        app.resolve_local_path(&id);
        let meta = app.repos.get(&id).unwrap();
        assert!(meta.local_path_resolved);
        assert!(meta.local_path.is_none());
    }
}

#[cfg(test)]
mod cursor_skip_tests {
    use super::*;

    #[test]
    fn cursor_moves_skips_headers_in_grouped_view() {
        use crate::app::SidebarRow;
        use crate::config::Config;
        use crate::git::types::{BranchEntry, PullRequest, RepoId};
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let mut app = App::new(Config::default());
        let active = RepoId {
            host: None,
            owner: "a".into(),
            name: "x".into(),
        };
        let other = RepoId {
            host: None,
            owner: "b".into(),
            name: "y".into(),
        };
        app.active_repo = Some(active.clone());
        app.main_filter = MainFilter::ReviewRequested;
        let make_pr = |num: u64, head: &str, repo: &RepoId| PullRequest {
            number: num,
            title: "t".into(),
            author: "u".into(),
            state: "OPEN".into(),
            head_ref: head.into(),
            updated_at: "2024".into(),
            is_draft: false,
            review_requests: vec![],
            latest_reviews: vec![],
            review_status: None,
            repo_id: repo.clone(),
        };
        app.entries = vec![
            BranchEntry {
                name: "e1".into(),
                repo_id: active.clone(),
                local_branch: None,
                worktree: None,
                pull_request: Some(make_pr(1, "e1", &active)),
                git_status: None,
            },
            BranchEntry {
                name: "e2".into(),
                repo_id: other.clone(),
                local_branch: None,
                worktree: None,
                pull_request: Some(make_pr(2, "e2", &other)),
                git_status: None,
            },
        ];
        // rows = [Header(active), Entry(e1), Header(other), Entry(e2)] → indices 0..=3
        // Initial: snap to first Entry (= 1)
        app.snap_scroll_to_entry();
        assert_eq!(app.sidebar_scroll, 1);

        // j → should jump from index 1 to index 3 (skip Header at index 2)
        app.handle_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE));
        let rows = app.sidebar_rows();
        assert_eq!(app.sidebar_scroll, 3);
        assert!(matches!(rows[app.sidebar_scroll], SidebarRow::Entry(_)));

        // k → should jump back from 3 to 1
        app.handle_key(KeyEvent::new(KeyCode::Char('k'), KeyModifiers::NONE));
        let rows = app.sidebar_rows();
        assert_eq!(app.sidebar_scroll, 1);
        assert!(matches!(rows[app.sidebar_scroll], SidebarRow::Entry(_)));
    }
}

#[cfg(test)]
mod sidebar_rows_tests {
    use super::*;

    #[test]
    fn render_rows_groups_when_multi_repo() {
        use crate::app::SidebarRow;
        use crate::config::Config;
        use crate::git::types::{BranchEntry, PullRequest, RepoId};
        let mut app = App::new(Config::default());
        let active = RepoId {
            host: None,
            owner: "a".into(),
            name: "r1".into(),
        };
        let other = RepoId {
            host: None,
            owner: "b".into(),
            name: "r2".into(),
        };
        app.active_repo = Some(active.clone());
        app.main_filter = MainFilter::ReviewRequested;
        let make_pr = |num: u64, head: &str, repo: &RepoId| PullRequest {
            number: num,
            title: "x".into(),
            author: "u".into(),
            state: "OPEN".into(),
            head_ref: head.into(),
            updated_at: "2024".into(),
            is_draft: false,
            review_requests: vec![],
            latest_reviews: vec![],
            review_status: None,
            repo_id: repo.clone(),
        };
        app.entries = vec![
            BranchEntry {
                name: "f1".into(),
                repo_id: active.clone(),
                local_branch: None,
                worktree: None,
                pull_request: Some(make_pr(1, "f1", &active)),
                git_status: None,
            },
            BranchEntry {
                name: "f2".into(),
                repo_id: other.clone(),
                local_branch: None,
                worktree: None,
                pull_request: Some(make_pr(2, "f2", &other)),
                git_status: None,
            },
        ];
        let rows = app.sidebar_rows();
        let header_count = rows
            .iter()
            .filter(|r| matches!(r, SidebarRow::Header { .. }))
            .count();
        let entry_count = rows
            .iter()
            .filter(|r| matches!(r, SidebarRow::Entry(_)))
            .count();
        assert_eq!(header_count, 2);
        assert_eq!(entry_count, 2);
    }

    #[test]
    fn render_rows_no_headers_when_single_repo() {
        use crate::app::SidebarRow;
        use crate::config::Config;
        use crate::git::types::{BranchEntry, PullRequest, RepoId};
        let mut app = App::new(Config::default());
        let active = RepoId {
            host: None,
            owner: "a".into(),
            name: "r1".into(),
        };
        app.active_repo = Some(active.clone());
        app.main_filter = MainFilter::ReviewRequested;
        let make_pr = |num: u64, head: &str| PullRequest {
            number: num,
            title: "x".into(),
            author: "u".into(),
            state: "OPEN".into(),
            head_ref: head.into(),
            updated_at: "2024".into(),
            is_draft: false,
            review_requests: vec![],
            latest_reviews: vec![],
            review_status: None,
            repo_id: active.clone(),
        };
        app.entries = vec![
            BranchEntry {
                name: "f1".into(),
                repo_id: active.clone(),
                local_branch: None,
                worktree: None,
                pull_request: Some(make_pr(1, "f1")),
                git_status: None,
            },
            BranchEntry {
                name: "f2".into(),
                repo_id: active.clone(),
                local_branch: None,
                worktree: None,
                pull_request: Some(make_pr(2, "f2")),
                git_status: None,
            },
        ];
        let rows = app.sidebar_rows();
        assert_eq!(
            rows.iter()
                .filter(|r| matches!(r, SidebarRow::Header { .. }))
                .count(),
            0
        );
        assert_eq!(rows.len(), 2);
    }

    #[test]
    fn render_rows_local_filter_never_groups() {
        use crate::app::SidebarRow;
        use crate::config::Config;
        use crate::git::types::{BranchEntry, RepoId};
        let mut app = App::new(Config::default());
        let active = RepoId {
            host: None,
            owner: "a".into(),
            name: "r1".into(),
        };
        app.active_repo = Some(active.clone());
        app.main_filter = MainFilter::Local;
        // Local mode: filtered_entries requires has_local. Give entries a local branch.
        let make_branch = |name: &str| crate::git::types::Branch {
            name: name.into(),
            is_current: false,
            upstream: None,
            is_merged: false,
        };
        app.entries = vec![BranchEntry {
            name: "f1".into(),
            repo_id: active.clone(),
            local_branch: Some(make_branch("f1")),
            worktree: None,
            pull_request: None,
            git_status: None,
        }];
        let rows = app.sidebar_rows();
        assert_eq!(
            rows.iter()
                .filter(|r| matches!(r, SidebarRow::Header { .. }))
                .count(),
            0
        );
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

#[cfg(test)]
mod action_menu_cross_repo_tests {
    use super::*;

    #[test]
    fn action_menu_cross_repo_no_clone_excludes_worktree_actions() {
        use crate::app::ActionItem;
        use crate::config::Config;
        use crate::git::types::{BranchEntry, PullRequest, RepoId, RepoMeta};
        let mut app = App::new(Config::default());
        let active = RepoId {
            host: None,
            owner: "a".into(),
            name: "x".into(),
        };
        let other = RepoId {
            host: None,
            owner: "b".into(),
            name: "y".into(),
        };
        app.active_repo = Some(active.clone());
        app.repos.insert(
            other.clone(),
            RepoMeta {
                local_path: None,
                local_path_resolved: true,
            },
        );
        app.main_filter = MainFilter::ReviewRequested;
        app.entries = vec![BranchEntry {
            name: "feat".into(),
            repo_id: other.clone(),
            local_branch: None,
            worktree: None,
            pull_request: Some(PullRequest {
                number: 1,
                title: "x".into(),
                author: "u".into(),
                state: "OPEN".into(),
                head_ref: "feat".into(),
                updated_at: "2024".into(),
                is_draft: false,
                review_requests: vec![],
                latest_reviews: vec![],
                review_status: None,
                repo_id: other.clone(),
            }),
            git_status: None,
        }];
        app.snap_scroll_to_entry();
        app.open_action_menu();
        let menu = app.action_menu.as_ref().unwrap();
        assert!(menu.items.contains(&ActionItem::OpenPrInBrowser));
        assert!(menu.items.contains(&ActionItem::CopyBranchName));
        assert!(!menu.items.contains(&ActionItem::CreateWorktree));
        assert!(!menu.items.contains(&ActionItem::DeleteBranch));
        assert!(menu.footer.is_some());
        assert!(menu.footer.as_ref().unwrap().contains("not cloned"));
    }

    #[test]
    fn action_menu_cross_repo_with_clone_enables_worktree_create() {
        use crate::app::ActionItem;
        use crate::config::Config;
        use crate::git::types::{BranchEntry, PullRequest, RepoId, RepoMeta};
        let mut app = App::new(Config::default());
        let active = RepoId {
            host: None,
            owner: "a".into(),
            name: "x".into(),
        };
        let other = RepoId {
            host: None,
            owner: "b".into(),
            name: "y".into(),
        };
        let tmp = tempfile::tempdir().unwrap();
        let clone_path = tmp.path().join("github.com").join("b").join("y");
        std::fs::create_dir_all(&clone_path).unwrap();
        app.active_repo = Some(active.clone());
        app.repos.insert(
            other.clone(),
            RepoMeta {
                local_path: Some(clone_path),
                local_path_resolved: true,
            },
        );
        app.main_filter = MainFilter::ReviewRequested;
        app.entries = vec![BranchEntry {
            name: "feat".into(),
            repo_id: other.clone(),
            local_branch: None,
            worktree: None,
            pull_request: Some(PullRequest {
                number: 1,
                title: "x".into(),
                author: "u".into(),
                state: "OPEN".into(),
                head_ref: "feat".into(),
                updated_at: "2024".into(),
                is_draft: false,
                review_requests: vec![],
                latest_reviews: vec![],
                review_status: None,
                repo_id: other.clone(),
            }),
            git_status: None,
        }];
        app.snap_scroll_to_entry();
        app.open_action_menu();
        let menu = app.action_menu.as_ref().unwrap();
        assert!(menu.items.contains(&ActionItem::CreateWorktree));
        assert!(menu.footer.is_none());
    }

    #[test]
    fn cd_into_worktree_cross_repo_uses_absolute_path() {
        use crate::app::ActionItem;
        use crate::config::Config;
        use crate::git::types::{BranchEntry, RepoId, Worktree};
        let mut app = App::new(Config::default());
        let active = RepoId {
            host: None,
            owner: "a".into(),
            name: "x".into(),
        };
        let other = RepoId {
            host: None,
            owner: "b".into(),
            name: "y".into(),
        };
        app.active_repo = Some(active);
        app.entries = vec![BranchEntry {
            name: "feat".into(),
            repo_id: other.clone(),
            local_branch: None,
            worktree: Some(Worktree {
                path: "/tmp/clones/b/feat".into(),
                head: "abc".into(),
                branch: Some("feat".into()),
                is_bare: false,
            }),
            pull_request: None,
            git_status: None,
        }];
        app.snap_scroll_to_entry();
        app.execute_action(ActionItem::CdIntoWorktree, &other, "feat");
        assert_eq!(app.cd_path.as_deref(), Some("/tmp/clones/b/feat"));
        assert!(app.should_quit);
    }

    #[test]
    fn execute_action_targets_correct_repo_when_branch_names_collide() {
        use crate::app::ActionItem;
        use crate::config::Config;
        use crate::git::types::{Branch, BranchEntry, PullRequest, RepoId, Worktree};
        let mut app = App::new(Config::default());
        let active = RepoId {
            host: None,
            owner: "active".into(),
            name: "x".into(),
        };
        let other = RepoId {
            host: None,
            owner: "other".into(),
            name: "y".into(),
        };
        app.active_repo = Some(active.clone());

        // Active repo has a local branch called feature/auth
        let active_entry = BranchEntry {
            name: "feature/auth".into(),
            repo_id: active.clone(),
            local_branch: Some(Branch {
                name: "feature/auth".into(),
                is_current: false,
                upstream: None,
                is_merged: false,
            }),
            worktree: None,
            pull_request: None,
            git_status: None,
        };
        // Cross-repo also has a PR on feature/auth, with a known worktree
        let cross_entry = BranchEntry {
            name: "feature/auth".into(),
            repo_id: other.clone(),
            local_branch: None,
            worktree: Some(Worktree {
                path: "/cross/repo/feature/auth".into(),
                head: "abc".into(),
                branch: Some("feature/auth".into()),
                is_bare: false,
            }),
            pull_request: Some(PullRequest {
                number: 7,
                title: "x".into(),
                author: "u".into(),
                state: "OPEN".into(),
                head_ref: "feature/auth".into(),
                updated_at: "2024".into(),
                is_draft: false,
                review_requests: vec![],
                latest_reviews: vec![],
                review_status: None,
                repo_id: other.clone(),
            }),
            git_status: None,
        };
        // Active repo first per merge_entries sort
        app.entries = vec![active_entry, cross_entry];

        // Dispatch CdIntoWorktree targeting cross-repo branch.
        // The tuple-based execute_action must NOT pick the active-repo entry.
        app.execute_action(ActionItem::CdIntoWorktree, &other, "feature/auth");
        assert_eq!(app.cd_path.as_deref(), Some("/cross/repo/feature/auth"));
    }
}

#[cfg(test)]
mod wt_list_lazy_load_tests {
    use super::*;

    #[test]
    fn selecting_cross_repo_entry_signals_wt_list_load() {
        use crate::config::Config;
        use crate::git::types::{BranchEntry, PullRequest, RepoId, RepoMeta};
        let tmp = tempfile::tempdir().unwrap();
        let clone_path = tmp.path().join("github.com").join("b").join("y");
        std::fs::create_dir_all(&clone_path).unwrap();

        let mut app = App::new(Config::default());
        let active = RepoId {
            host: None,
            owner: "a".into(),
            name: "x".into(),
        };
        let other = RepoId {
            host: None,
            owner: "b".into(),
            name: "y".into(),
        };
        app.active_repo = Some(active);
        app.clone_root = Some(tmp.path().to_path_buf());
        app.repos.insert(
            other.clone(),
            RepoMeta {
                local_path: None,
                local_path_resolved: false,
            },
        );
        app.main_filter = MainFilter::ReviewRequested;
        app.entries = vec![BranchEntry {
            name: "feat".into(),
            repo_id: other.clone(),
            local_branch: None,
            worktree: None,
            pull_request: Some(PullRequest {
                number: 1,
                title: "x".into(),
                author: "u".into(),
                state: "OPEN".into(),
                head_ref: "feat".into(),
                updated_at: "2024".into(),
                is_draft: false,
                review_requests: vec![],
                latest_reviews: vec![],
                review_status: None,
                repo_id: other.clone(),
            }),
            git_status: None,
        }];
        app.snap_scroll_to_entry();
        app.request_details_for_selection();
        assert_eq!(app.wt_list_requested.as_ref(), Some(&other));
    }

    #[test]
    fn selecting_active_repo_entry_does_not_signal_wt_list_load() {
        use crate::config::Config;
        use crate::git::types::{BranchEntry, RepoId};
        let mut app = App::new(Config::default());
        let active = RepoId {
            host: None,
            owner: "a".into(),
            name: "x".into(),
        };
        app.active_repo = Some(active.clone());
        let make_branch = |name: &str| crate::git::types::Branch {
            name: name.into(),
            is_current: false,
            upstream: None,
            is_merged: false,
        };
        app.entries = vec![BranchEntry {
            name: "feat".into(),
            repo_id: active.clone(),
            local_branch: Some(make_branch("feat")),
            worktree: None,
            pull_request: None,
            git_status: None,
        }];
        app.snap_scroll_to_entry();
        app.request_details_for_selection();
        assert!(app.wt_list_requested.is_none());
    }
}
