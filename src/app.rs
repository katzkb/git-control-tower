use crossterm::event::{KeyCode, KeyEvent};

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
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
    /// Short label shown inside the action menu; the group header
    /// (`group_label`) carries the "worktree"/"branch" context.
    pub fn label(&self) -> &'static str {
        match self {
            Self::CreateWorktree => "Create",
            Self::CdIntoWorktree => "Go to (cd)",
            Self::DeleteWorktree => "Delete",
            Self::CreateBranch => "Create from this",
            Self::DeleteBranch => "Delete",
            Self::OpenPrInBrowser => "Open PR in browser",
            Self::CopyBranchName => "Copy branch name",
        }
    }

    /// Section header shown above this item's group in the action menu;
    /// None = ungrouped top section.
    pub fn group_label(&self) -> Option<&'static str> {
        match self {
            Self::OpenPrInBrowser | Self::CopyBranchName => None,
            Self::CdIntoWorktree | Self::CreateWorktree | Self::DeleteWorktree => Some("Worktree"),
            Self::CreateBranch | Self::DeleteBranch => Some("Branch"),
        }
    }
}

/// A unit of work requested by the UI and executed by the main loop.
///
/// `handle_key` (and async-result handling in `run()`) pushes commands onto
/// `App::commands` via [`App::push_command`]; `run()` drains the queue once
/// per loop iteration and dispatches each variant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    // Read/fetch intents
    FetchPrs(MainFilter),
    FetchPrDetail(crate::git::types::RepoId, u64),
    /// Load `git status` for the worktree at this path.
    LoadGitStatus(String),
    LoadWorktreeList(crate::git::types::RepoId),
    ReloadBranches,
    ReloadCommits,
    // Mutating intents
    DeleteWorktree(String),
    ForceDeleteWorktree(String),
    /// `(repo_id, branch_name)` — carries `RepoId` so the main-loop lookup
    /// matches the correct repo when branch names collide.
    CreateWorktree(crate::git::types::RepoId, String),
    DeleteBranches(Vec<String>),
    CreateBranch {
        source: String,
        name: String,
    },
    OpenPrInBrowser(crate::git::types::RepoId, u64),
    CopyBranchName(String),
    /// Quit the TUI and emit this path for the shell-integration `cd`.
    CdAndQuit(String),
}

/// A confirm dialog plus the command to run when the user accepts it.
/// Carrying the consequence with the dialog means `handle_confirm_key`
/// needs no out-of-band staging state to know what `y` applies to.
pub struct PendingConfirm {
    pub dialog: ConfirmDialog,
    pub on_confirm: Command,
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
    /// `q`/`Esc` was pressed while ops were active; the next press quits.
    /// Reset explicitly by the main loop when the op batch finishes —
    /// deliberately not folded into `clear()`/`sweep_unfinished()` (issue #220).
    pub quit_pressed: bool,
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

/// All async-work dedup state, in one place (issue #236).
///
/// At dispatch time the `HashSet`-tracked fetches are *dropped* when already
/// inflight (a duplicate fetch is pointless), while the two reload bools use
/// *requeue-if-busy* (a reload requested mid-reload must still produce a
/// fresh fetch afterwards).
#[derive(Default)]
pub struct Inflight {
    pub pr_detail: HashSet<(crate::git::types::RepoId, u64)>,
    /// Worktree paths with a running `git status` load.
    pub git_status: HashSet<String>,
    pub branches_reload: bool,
    pub commits_reload: bool,
    /// Worktree paths with an in-flight create/delete, gated per-path so
    /// unrelated worktrees stay actionable in the UI while one is running.
    pub worktrees: HashSet<String>,
    /// Repos with a running cross-repo worktree-list load.
    pub wt_lists: HashSet<crate::git::types::RepoId>,
}

/// What the user is looking at: the main-view entry list and its
/// cursor/filter/search/multi-select state, plus the scroll positions of the
/// other views (issue #220).
#[derive(Default)]
pub struct ViewState {
    pub entries: Vec<BranchEntry>,
    pub main_filter: MainFilter,
    pub sidebar_scroll: usize,
    pub sidebar_offset: usize,
    pub search_active: bool,
    pub search_query: String,
    search_pre_scroll: usize, // saved scroll position before search
    /// Branch names multi-selected in the sidebar (space / `a` keys).
    pub branch_selected: HashSet<String>,
    pub log_scroll: usize,
    pub history_scroll: usize,
    pub pr_detail_scroll: usize,
}

impl ViewState {
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
}

/// Modal/overlay UI state, listed in `handle_key` priority order (issue #220).
#[derive(Default)]
pub struct Overlays {
    pub show_help: bool,
    pub confirm_dialog: Option<PendingConfirm>,
    pub action_menu: Option<ActionMenu>,
    pub branch_create_input: Option<BranchCreateInput>,
    pub notification: Option<Notification>,
}

/// Per-filter PR list caches keyed by `MainFilter`, their fetch parameters,
/// and the PR-detail cache (issue #220).
#[derive(Default)]
pub struct PrCaches {
    pub local: Vec<PullRequest>,
    pub my: Vec<PullRequest>,
    pub review: Vec<PullRequest>,
    pub local_loaded: bool,
    pub my_loaded: bool,
    pub review_loaded: bool,
    pub show_merged: bool,
    pub include_team_reviews: bool,
    /// PR detail bodies for the detail pane, cached by `(RepoId, PR number)`.
    pub detail: HashMap<(crate::git::types::RepoId, u64), PrDetail>,
}

impl PrCaches {
    pub fn current(&self, filter: MainFilter) -> &[PullRequest] {
        match filter {
            MainFilter::Local => &self.local,
            MainFilter::MyPr => &self.my,
            MainFilter::ReviewRequested => &self.review,
        }
    }

    /// A filter's list counts as loading until its first fetch lands.
    pub fn is_loading(&self, filter: MainFilter) -> bool {
        match filter {
            MainFilter::Local => !self.local_loaded,
            MainFilter::MyPr => !self.my_loaded,
            MainFilter::ReviewRequested => !self.review_loaded,
        }
    }

    /// Store a fetched PR list for `filter` and mark it loaded.
    pub fn set(&mut self, filter: MainFilter, prs: Vec<PullRequest>) {
        match filter {
            MainFilter::Local => {
                self.local = prs;
                self.local_loaded = true;
            }
            MainFilter::MyPr => {
                self.my = prs;
                self.my_loaded = true;
            }
            MainFilter::ReviewRequested => {
                self.review = prs;
                self.review_loaded = true;
            }
        }
    }

    /// Drop a filter's cached list so the next fetch is forced.
    pub fn invalidate(&mut self, filter: MainFilter) {
        match filter {
            MainFilter::Local => {
                self.local.clear();
                self.local_loaded = false;
            }
            MainFilter::MyPr => {
                self.my.clear();
                self.my_loaded = false;
            }
            MainFilter::ReviewRequested => {
                self.review.clear();
                self.review_loaded = false;
            }
        }
    }
}

/// Raw data fetched from `git`/`gh`: inputs to `rebuild_entries` and the Log
/// view, plus the gh identity used for review-status computation (issue #220).
#[derive(Default)]
pub struct RawData {
    pub branches: Vec<Branch>,
    pub worktrees: Vec<Worktree>,
    pub commits: Vec<Commit>,
    pub gh_user: String,
    pub gh_user_load_failed: bool,
}

/// Cross-repo context: active repo, clone root, lazily-populated per-repo
/// metadata and worktree lists, and the merged global config layers used to
/// resolve per-repo effective configs (issue #220).
#[derive(Default)]
pub struct CrossRepoState {
    /// Set at startup; `None` when startup couldn't infer a repo.
    pub active_repo: Option<crate::git::types::RepoId>,
    pub clone_root: Option<std::path::PathBuf>,
    /// Per-repo metadata (populated lazily as repos are selected).
    pub repos: std::collections::HashMap<crate::git::types::RepoId, crate::git::types::RepoMeta>,
    /// Worktree lists per repo (populated lazily as cross-repo PRs are selected).
    pub wt_lists_per_repo:
        std::collections::HashMap<crate::git::types::RepoId, Vec<crate::git::types::Worktree>>,
    /// Merged global config layers (home-dir files only), used to resolve a
    /// per-repo effective config for cross-repo worktree operations.
    pub global_layers: toml::Table,
}

impl CrossRepoState {
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

    /// Effective config for a specific repo root: the global layers overlaid
    /// with `<repo_root>/.gct.toml`. Used for cross-repo worktree operations so
    /// the target repo's own `.gct.toml` applies (not the launching repo's).
    pub fn resolve_repo_config(&self, repo_root: &std::path::Path) -> crate::config::Config {
        crate::config::resolve_config(&self.global_layers, Some(repo_root))
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

pub struct App {
    pub active_view: ActiveView,
    pub should_quit: bool,

    // What the user is looking at: entry list, cursor/filter/search state,
    // and per-view scroll positions (issue #220).
    pub view: ViewState,

    // Raw data fetched from git/gh (issue #220).
    pub raw: RawData,

    // Per-filter PR caches and the PR-detail cache (issue #220).
    pub prs: PrCaches,

    // Verbose mode
    pub verbose: bool,
    pub verbose_errors: Vec<String>,

    // Modal/overlay UI state (issue #220).
    pub overlays: Overlays,

    // Exit with cd path
    pub cd_path: Option<String>,

    // Command queue: intents staged by key handling (and async-result
    // handling in `run()`), drained once per loop iteration in `run()`.
    pub commands: VecDeque<Command>,

    // Selection changed; detail fetches are flushed on the next tick
    // (debounce — see request_details_for_selection).
    selection_details_pending: bool,

    // Async-work dedup bookkeeping (fetches, reloads, worktree ops).
    pub inflight: Inflight,
    pub progress: ProgressTracker,

    // Spinner animation
    spinner_tick: usize,

    // Loaded TOML config (protected_branches, worktree, …) for the active repo.
    pub config: crate::config::Config,

    // Cross-repo context and per-repo metadata (issue #220).
    pub cross_repo: CrossRepoState,
}

impl App {
    pub fn new(config: crate::config::Config) -> Self {
        Self {
            active_view: ActiveView::default(),
            should_quit: false,
            view: ViewState::default(),
            raw: RawData::default(),
            prs: PrCaches::default(),
            verbose: false,
            verbose_errors: Vec::new(),
            overlays: Overlays::default(),
            cd_path: None,
            commands: VecDeque::new(),
            selection_details_pending: false,
            inflight: Inflight::default(),
            progress: ProgressTracker::default(),
            spinner_tick: 0,
            config,
            cross_repo: CrossRepoState::default(),
        }
    }

    /// Queue a command for the main loop, coalescing exact duplicates —
    /// pushing an intent that is already pending is a no-op, matching the
    /// one-slot semantics of the request flags this queue replaced.
    pub fn push_command(&mut self, cmd: Command) {
        if !self.commands.contains(&cmd) {
            self.commands.push_back(cmd);
        }
    }

    /// Take a snapshot of the queued commands, leaving the queue empty.
    /// `run()` dispatches the snapshot; commands pushed during dispatch land
    /// on the fresh queue and are serviced next iteration (no busy re-loop).
    pub fn take_commands(&mut self) -> VecDeque<Command> {
        std::mem::take(&mut self.commands)
    }

    pub fn current_prs(&self) -> &[PullRequest] {
        self.prs.current(self.view.main_filter)
    }

    const SPINNER_FRAMES: &'static [&'static str] =
        &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

    pub fn tick(&mut self) {
        self.spinner_tick = self.spinner_tick.wrapping_add(1);
        if std::mem::take(&mut self.selection_details_pending) {
            self.flush_selection_details();
        }
        // Don't auto-dismiss while a worktree operation is in progress
        if self.inflight.worktrees.is_empty()
            && let Some(ref mut n) = self.overlays.notification
        {
            if n.ticks_remaining > 0 {
                n.ticks_remaining -= 1;
            } else {
                self.overlays.notification = None;
            }
        }
    }

    pub fn spinner_frame(&self) -> &'static str {
        Self::SPINNER_FRAMES[self.spinner_tick % Self::SPINNER_FRAMES.len()]
    }

    pub fn adjust_sidebar_offset(&mut self, visible_height: usize, item_count: usize) {
        self.view.adjust_sidebar_offset(visible_height, item_count)
    }

    pub fn is_current_view_loading(&self) -> bool {
        self.prs.is_loading(self.view.main_filter)
    }

    pub fn rebuild_entries(&mut self) {
        // When `active_repo` is absent (startup couldn't infer one), unwrap_or_default
        // produces a sentinel empty RepoId. It can't collide with any real PR's repo_id,
        // so cross-repo entries are still keyed correctly and worktree injection no-ops
        // safely.
        let active = self.cross_repo.active_repo.clone().unwrap_or_default();
        self.view.entries = crate::data::merge_entries(
            &active,
            &self.raw.branches,
            &self.raw.worktrees,
            self.current_prs(),
            &self.cross_repo.wt_lists_per_repo,
        );
    }

    /// Rebuild entries after a data change and clamp the sidebar selection
    /// to the (possibly shorter) filtered list.
    pub fn rebuild_entries_and_clamp(&mut self) {
        self.rebuild_entries();
        let filtered_len = self.filtered_entries().len();
        if self.view.sidebar_scroll >= filtered_len && filtered_len > 0 {
            self.view.sidebar_scroll = filtered_len - 1;
        }
    }

    pub fn filtered_entries(&self) -> Vec<&BranchEntry> {
        self.view.filtered_entries()
    }

    pub fn sidebar_rows(&self) -> Vec<SidebarRow<'_>> {
        self.view.sidebar_rows()
    }

    pub fn selected_entry(&self) -> Option<&BranchEntry> {
        self.view.selected_entry()
    }

    pub fn snap_scroll_to_entry(&mut self) {
        self.view.snap_scroll_to_entry()
    }

    /// Return the cached PR detail for the currently selected entry, if available.
    pub fn selected_pr_detail(&self) -> Option<&PrDetail> {
        let entry = self.selected_entry()?;
        let pr_num = entry.pr_number()?;
        self.prs.detail.get(&(entry.repo_id.clone(), pr_num))
    }

    /// Signal that the selection changed. The actual detail fetches are
    /// deferred to the next `tick()`: ticks are starved while key events are
    /// pending (see `EventHandler`), so holding `j`/`k` coalesces to a single
    /// fetch for the final selection ~80ms after scrolling settles, instead
    /// of spawning a `gh`/`git` subprocess per row passed through.
    pub fn request_details_for_selection(&mut self) {
        self.view.pr_detail_scroll = 0;
        self.selection_details_pending = true;
    }

    /// Queue the fetches the current selection needs (PR detail, git status,
    /// cross-repo worktree list). Called from `tick()` once input settles;
    /// recomputing from the *current* selection here is what discards the
    /// intermediate targets of a rapid scroll.
    fn flush_selection_details(&mut self) {
        let selected = self.selected_entry().cloned();

        if let Some(entry) = &selected {
            // Request PR detail if entry has a PR and it's not cached
            if let Some(pr_num) = entry.pr_number()
                && !self
                    .prs
                    .detail
                    .contains_key(&(entry.repo_id.clone(), pr_num))
            {
                self.push_command(Command::FetchPrDetail(entry.repo_id.clone(), pr_num));
            }

            // Request git status if entry has a worktree and status not yet loaded
            if let Some(wt_path) = entry.worktree_path()
                && entry.git_status.is_none()
            {
                self.push_command(Command::LoadGitStatus(wt_path.to_string()));
            }
        }

        // Signal lazy load of cross-repo worktree list if not yet fetched (use the same `selected` binding)
        if let Some(entry) = &selected
            && self.cross_repo.active_repo.as_ref() != Some(&entry.repo_id)
            && !self
                .cross_repo
                .wt_lists_per_repo
                .contains_key(&entry.repo_id)
            && !self.inflight.wt_lists.contains(&entry.repo_id)
        {
            self.resolve_local_path(&entry.repo_id);
            if self
                .cross_repo
                .repos
                .get(&entry.repo_id)
                .and_then(|m| m.local_path.as_ref())
                .is_some()
            {
                self.push_command(Command::LoadWorktreeList(entry.repo_id.clone()));
            }
        }
    }

    pub fn handle_key(&mut self, key: KeyEvent) {
        // Help overlay takes priority
        if self.overlays.show_help {
            match key.code {
                KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('?') => {
                    self.overlays.show_help = false;
                }
                _ => {}
            }
            return;
        }

        // Confirm dialog takes priority
        if self.overlays.confirm_dialog.is_some() {
            self.handle_confirm_key(key.code);
            return;
        }

        // Action menu takes priority
        if self.overlays.action_menu.is_some() {
            self.handle_action_menu_key(key.code);
            return;
        }

        // Branch-create input modal takes priority
        if self.overlays.branch_create_input.is_some() {
            self.handle_branch_create_input_key(key.code);
            return;
        }

        // Search mode takes priority in Main view
        if self.view.search_active && self.active_view == ActiveView::Main {
            self.handle_search_key(key.code);
            return;
        }

        match key.code {
            KeyCode::Char('?') => self.overlays.show_help = true,
            KeyCode::Char('q') => {
                if !self.progress.is_active() || self.progress.quit_pressed {
                    self.should_quit = true;
                } else {
                    self.progress.quit_pressed = true;
                }
            }
            KeyCode::Esc => {
                if !self.view.search_query.is_empty() {
                    // Clear search filter and restore scroll
                    self.view.search_query.clear();
                    self.view.sidebar_scroll = self.view.search_pre_scroll;
                    self.view.sidebar_offset = 0;
                    self.snap_scroll_to_entry();
                    self.request_details_for_selection();
                } else if matches!(self.active_view, ActiveView::Log | ActiveView::History) {
                    self.active_view = ActiveView::Main;
                } else if !self.progress.is_active() || self.progress.quit_pressed {
                    self.should_quit = true;
                } else {
                    self.progress.quit_pressed = true;
                }
            }
            KeyCode::Char('l') => self.active_view = ActiveView::Log,
            KeyCode::Char('h') => {
                if self.active_view != ActiveView::History {
                    self.active_view = ActiveView::History;
                    self.view.history_scroll = 0;
                }
            }
            KeyCode::Char('1') => {
                self.view.main_filter = MainFilter::Local;
                self.active_view = ActiveView::Main;
                self.view.search_query.clear();
                self.view.sidebar_scroll = 0;
                self.view.sidebar_offset = 0;
                self.rebuild_entries();
                self.snap_scroll_to_entry();
                if !self.prs.local_loaded {
                    self.push_command(Command::FetchPrs(MainFilter::Local));
                }
                self.request_details_for_selection();
            }
            KeyCode::Char('2') => {
                self.view.main_filter = MainFilter::MyPr;
                self.active_view = ActiveView::Main;
                self.view.search_query.clear();
                self.view.sidebar_scroll = 0;
                self.view.sidebar_offset = 0;
                self.rebuild_entries();
                self.snap_scroll_to_entry();
                if !self.prs.my_loaded {
                    self.push_command(Command::FetchPrs(MainFilter::MyPr));
                }
                self.request_details_for_selection();
            }
            KeyCode::Char('3') => {
                self.view.main_filter = MainFilter::ReviewRequested;
                self.active_view = ActiveView::Main;
                self.view.search_query.clear();
                self.view.sidebar_scroll = 0;
                self.view.sidebar_offset = 0;
                self.rebuild_entries();
                self.snap_scroll_to_entry();
                if !self.prs.review_loaded {
                    self.push_command(Command::FetchPrs(MainFilter::ReviewRequested));
                }
                self.request_details_for_selection();
            }
            KeyCode::Char('r') => match self.active_view {
                ActiveView::Main => {
                    // Invalidate current filter's PR cache so the fetch is forced
                    self.prs.invalidate(self.view.main_filter);
                    // Clear PR detail cache for ALL filters, not just the current
                    // one — stale detail bodies are risky after a refresh, and the
                    // detail pane will refetch on the next selection.
                    self.prs.detail.clear();
                    // Invalidate cross-repo worktree list caches so they re-fetch.
                    self.cross_repo.wt_lists_per_repo.clear();
                    self.inflight.wt_lists.clear();
                    // Signal branches/worktrees reload + PR fetch
                    self.push_command(Command::ReloadBranches);
                    self.push_command(Command::FetchPrs(self.view.main_filter));
                    self.overlays.notification = Some(Notification::success("Refreshing…"));
                }
                ActiveView::Log => {
                    self.push_command(Command::ReloadCommits);
                    self.overlays.notification = Some(Notification::success("Refreshing…"));
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
                if let Some(next) = self.view.next_entry_index(self.view.sidebar_scroll) {
                    self.view.sidebar_scroll = next;
                    self.request_details_for_selection();
                }
            }
            KeyCode::Char('k') | KeyCode::Up if self.view.sidebar_scroll > 0 => {
                if let Some(prev) = self.view.prev_entry_index(self.view.sidebar_scroll) {
                    self.view.sidebar_scroll = prev;
                    self.request_details_for_selection();
                }
            }
            KeyCode::Char(' ') => {
                if let Some(entry) = self.selected_entry().cloned()
                    && !entry.is_current()
                    && !self.is_protected_branch(&entry.name)
                    && self.cross_repo.active_repo.as_ref() == Some(&entry.repo_id)
                {
                    if self.view.branch_selected.contains(&entry.name) {
                        self.view.branch_selected.remove(&entry.name);
                    } else {
                        self.view.branch_selected.insert(entry.name);
                    }
                }
            }
            KeyCode::Char('a') => {
                let to_select: Vec<String> = self
                    .view
                    .entries
                    .iter()
                    .filter(|e| {
                        (e.is_merged() || e.pr_is_merged())
                            && !e.is_current()
                            && !self.is_protected_branch(&e.name)
                            && self.cross_repo.active_repo.as_ref() == Some(&e.repo_id)
                    })
                    .map(|e| e.name.clone())
                    .collect();
                self.view.branch_selected.extend(to_select);
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
                if !self.view.branch_selected.is_empty() {
                    for name in &self.view.branch_selected {
                        if let Some(entry) = self.view.entries.iter().find(|e| &e.name == name) {
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
                    // Snapshot the whole selection — the dispatcher re-filters
                    // (current/protected/inflight) at execution time.
                    let names: Vec<String> = self.view.branch_selected.iter().cloned().collect();
                    self.overlays.confirm_dialog = Some(PendingConfirm {
                        dialog: ConfirmDialog::new(title, msg),
                        on_confirm: Command::DeleteBranches(names),
                    });
                } else if !self.view.branch_selected.is_empty() {
                    // Non-empty selection but nothing deletable (e.g. PR-only entries
                    // with no local branch and no worktree). Tell the user rather
                    // than silently no-op.
                    self.overlays.notification = Some(Notification::error(
                        "Nothing to delete in selection".to_string(),
                    ));
                } else if let Some(entry) = self.selected_entry().cloned()
                    && let Some(wt_path) = entry.worktree_path()
                    && !entry.is_current()
                    && !self.inflight.worktrees.contains(wt_path)
                {
                    let path = wt_path.to_string();
                    self.overlays.confirm_dialog = Some(PendingConfirm {
                        dialog: ConfirmDialog::new(
                            "Delete Worktree",
                            format!("Remove worktree at {path}?"),
                        ),
                        on_confirm: Command::DeleteWorktree(path),
                    });
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
                let is_active = self.cross_repo.active_repo.as_ref() == Some(&entry.repo_id);
                let clone_path: Option<std::path::PathBuf> = if is_active {
                    None
                } else {
                    self.resolve_local_path(&entry.repo_id);
                    self.cross_repo
                        .repos
                        .get(&entry.repo_id)
                        .and_then(|m| m.local_path.clone())
                };
                let cross_repo_no_clone = !is_active && clone_path.is_none();
                if cross_repo_no_clone {
                    self.overlays.notification = Some(Notification::error(format!(
                        "{} not cloned. Set [workspace] clone_root.",
                        entry.repo_id
                    )));
                    return;
                }
                let wt_path = if is_active {
                    self.config.worktree_path(&entry.repo_id.name, &entry.name)
                } else {
                    // unwrap is safe: cross_repo_no_clone is false above
                    let root = clone_path.as_ref().unwrap();
                    // Cross-repo: resolve the target repo's own config so its
                    // `.gct.toml` applies (must match main.rs's creation path).
                    self.resolve_repo_config(root).worktree_path_for(
                        root,
                        &entry.repo_id.name,
                        &entry.name,
                    )
                };
                if self.inflight.worktrees.contains(&wt_path) {
                    return;
                }
                self.push_command(Command::CreateWorktree(
                    entry.repo_id.clone(),
                    entry.name.clone(),
                ));
                self.overlays.notification =
                    Some(Notification::success("Creating worktree...".to_string()));
            }
            KeyCode::Enter => {
                self.open_action_menu();
            }
            KeyCode::Char('m') => {
                if matches!(
                    self.view.main_filter,
                    MainFilter::MyPr | MainFilter::ReviewRequested
                ) {
                    self.prs.show_merged = !self.prs.show_merged;
                    // Invalidate both caches since merged state changed
                    self.prs.invalidate(MainFilter::MyPr);
                    self.prs.invalidate(MainFilter::ReviewRequested);
                    self.rebuild_entries();
                    self.push_command(Command::FetchPrs(self.view.main_filter));
                    self.view.sidebar_scroll = 0;
                    self.view.sidebar_offset = 0;
                    self.snap_scroll_to_entry();
                }
            }
            KeyCode::Char('t') if self.view.main_filter == MainFilter::ReviewRequested => {
                self.prs.include_team_reviews = !self.prs.include_team_reviews;
                self.prs.invalidate(MainFilter::ReviewRequested);
                self.rebuild_entries();
                self.push_command(Command::FetchPrs(MainFilter::ReviewRequested));
                self.view.sidebar_scroll = 0;
                self.view.sidebar_offset = 0;
                self.snap_scroll_to_entry();
            }
            KeyCode::Char('/') => {
                self.view.search_pre_scroll = self.view.sidebar_scroll;
                self.view.search_active = true;
                self.view.search_query.clear();
            }
            _ => {}
        }
    }

    fn handle_log_key(&mut self, code: KeyCode) {
        match code {
            KeyCode::Char('j') | KeyCode::Down
                if self.view.log_scroll + 1 < self.raw.commits.len() =>
            {
                self.view.log_scroll += 1;
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.view.log_scroll = self.view.log_scroll.saturating_sub(1);
            }
            _ => {}
        }
    }

    fn handle_history_key(&mut self, code: KeyCode) {
        let len = crate::git::command::command_history_len();
        match code {
            KeyCode::Char('j') | KeyCode::Down if len > 0 && self.view.history_scroll + 1 < len => {
                self.view.history_scroll += 1;
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.view.history_scroll = self.view.history_scroll.saturating_sub(1);
            }
            _ => {}
        }
    }

    fn handle_search_key(&mut self, code: KeyCode) {
        match code {
            KeyCode::Esc => {
                self.view.search_active = false;
                self.view.search_query.clear();
                self.view.sidebar_scroll = self.view.search_pre_scroll;
                self.snap_scroll_to_entry();
                self.request_details_for_selection();
            }
            KeyCode::Enter => {
                self.view.search_active = false;
                self.request_details_for_selection();
                self.open_action_menu();
            }
            KeyCode::Backspace => {
                self.view.search_query.pop();
                self.view.sidebar_scroll = 0;
                self.view.sidebar_offset = 0;
                self.snap_scroll_to_entry();
                self.request_details_for_selection();
            }
            KeyCode::Char(c) => {
                self.view.search_query.push(c);
                self.view.sidebar_scroll = 0;
                self.view.sidebar_offset = 0;
                self.snap_scroll_to_entry();
                self.request_details_for_selection();
            }
            KeyCode::Down => {
                if let Some(next) = self.view.next_entry_index(self.view.sidebar_scroll) {
                    self.view.sidebar_scroll = next;
                    self.request_details_for_selection();
                }
            }
            KeyCode::Up if self.view.sidebar_scroll > 0 => {
                if let Some(prev) = self.view.prev_entry_index(self.view.sidebar_scroll) {
                    self.view.sidebar_scroll = prev;
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
        let is_active_repo = self.cross_repo.active_repo.as_ref() == Some(&entry.repo_id);
        let clone_path: Option<std::path::PathBuf> = if is_active_repo {
            None // active repo runs in CWD, no clone path needed
        } else {
            // cross-repo: resolve once, then read
            self.resolve_local_path(&entry.repo_id);
            self.cross_repo
                .repos
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
            Some(self.config.worktree_path(&entry.repo_id.name, &entry.name))
        } else if let Some(ref root) = clone_path {
            // Cross-repo: resolve the target repo's own config (matches main.rs).
            Some(self.resolve_repo_config(root).worktree_path_for(
                root,
                &entry.repo_id.name,
                &entry.name,
            ))
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
                .map(|p| !self.inflight.worktrees.contains(p))
                .unwrap_or(false);
        if can_create_wt {
            items.push(ActionItem::CreateWorktree);
        }

        // DeleteWorktree is active-repo only (cross-repo wt management is OUT for v1).
        if is_active_repo
            && let Some(wt_path) = entry.worktree_path()
            && !entry.is_current()
            && !self.inflight.worktrees.contains(wt_path)
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
                .cross_repo
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
            self.overlays.action_menu = Some(ActionMenu {
                items,
                scroll: 0,
                target: (entry.repo_id.clone(), entry.name.clone()),
                footer,
            });
        }
    }

    fn handle_action_menu_key(&mut self, code: KeyCode) {
        let menu = match &mut self.overlays.action_menu {
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
                self.overlays.action_menu = None;
                self.execute_action(action, &repo_id, &branch_name);
            }
            KeyCode::Esc => {
                self.overlays.action_menu = None;
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
            .view
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
                self.push_command(Command::CreateWorktree(
                    entry.repo_id.clone(),
                    entry.name.clone(),
                ));
                self.overlays.notification =
                    Some(Notification::success("Creating worktree...".to_string()));
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
                    // Deleting via the action menu targets one worktree only —
                    // drop any checkbox selection so the sidebar reflects that.
                    self.view.branch_selected.clear();
                    self.overlays.confirm_dialog = Some(PendingConfirm {
                        dialog: ConfirmDialog::new(
                            "Delete Worktree",
                            format!("Remove worktree at {path}?"),
                        ),
                        on_confirm: Command::DeleteWorktree(path),
                    });
                }
            }
            ActionItem::CreateBranch => {
                self.overlays.branch_create_input = Some(BranchCreateInput {
                    source: entry.name.clone(),
                    name: String::new(),
                    cursor: 0,
                });
            }
            ActionItem::DeleteBranch => {
                let name = entry.name.clone();
                let is_unmerged = !entry.is_merged() && !entry.pr_is_merged();
                let msg = if is_unmerged {
                    format!("Delete branch {name}? (unmerged — will force delete)")
                } else {
                    format!("Delete branch {name}?")
                };
                self.overlays.confirm_dialog = Some(PendingConfirm {
                    dialog: ConfirmDialog::new("Delete Branch", msg),
                    on_confirm: Command::DeleteBranches(vec![name]),
                });
            }
            ActionItem::OpenPrInBrowser => {
                if let Some(pr) = &entry.pull_request {
                    self.push_command(Command::OpenPrInBrowser(entry.repo_id.clone(), pr.number));
                }
            }
            ActionItem::CopyBranchName => {
                self.push_command(Command::CopyBranchName(entry.name.clone()));
                self.overlays.notification =
                    Some(Notification::success(format!("Copied: {}", entry.name)));
            }
        }
    }

    fn handle_branch_create_input_key(&mut self, code: KeyCode) {
        let input = match &mut self.overlays.branch_create_input {
            Some(i) => i,
            None => return,
        };
        let char_len = input.name.chars().count();
        input.cursor = input.cursor.min(char_len);
        match code {
            KeyCode::Esc => {
                self.overlays.branch_create_input = None;
            }
            KeyCode::Enter if !input.name.is_empty() => {
                let source = input.source.clone();
                let name = input.name.clone();
                self.overlays.branch_create_input = None;
                self.push_command(Command::CreateBranch { source, name });
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
                if let Some(pending) = self.overlays.confirm_dialog.take() {
                    self.push_command(pending.on_confirm);
                }
            }
            KeyCode::Char('n') | KeyCode::Esc => {
                if let Some(pending) = self.overlays.confirm_dialog.take()
                    && let Command::ForceDeleteWorktree(path) = pending.on_confirm
                {
                    // Declining force-delete ends the op — release the path so
                    // its action items reappear.
                    self.inflight.worktrees.remove(&path);
                }
            }
            _ => {}
        }
    }

    pub fn is_protected_branch(&self, name: &str) -> bool {
        self.config.protected_branches.iter().any(|b| b == name)
    }

    pub fn known_hosts(&self) -> Vec<Option<String>> {
        self.cross_repo.known_hosts()
    }

    /// Remember a background-operation error for the error list shown in
    /// `--verbose` mode. Deduplicated, and recorded regardless of verbosity
    /// so the details are already there when the user relaunches with
    /// `--verbose` after seeing an error toast.
    pub fn record_error(&mut self, error: String) {
        if !self.verbose_errors.contains(&error) {
            self.verbose_errors.push(error);
        }
    }

    pub fn resolve_repo_config(&self, repo_root: &std::path::Path) -> crate::config::Config {
        self.cross_repo.resolve_repo_config(repo_root)
    }

    pub fn resolve_local_path(&mut self, id: &crate::git::types::RepoId) {
        self.cross_repo.resolve_local_path(id)
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
        assert!(app.progress.quit_pressed);

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
        assert!(app.progress.quit_pressed);

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
            body: "A".into(),
            additions: 0,
            deletions: 0,
        };
        let detail_b = PrDetail {
            number: 1,
            body: "B".into(),
            additions: 0,
            deletions: 0,
        };
        app.prs.detail.insert((id_a.clone(), 1), detail_a.clone());
        app.prs.detail.insert((id_b.clone(), 1), detail_b.clone());
        assert_eq!(app.prs.detail.len(), 2);
        assert_eq!(app.prs.detail.get(&(id_a, 1)).unwrap().body, "A");
        assert_eq!(app.prs.detail.get(&(id_b, 1)).unwrap().body, "B");
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
            app.cross_repo.repos.insert(
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
            app.cross_repo.repos.insert(
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
        app.cross_repo.clone_root = Some(tmp.path().to_path_buf());
        let id = RepoId {
            host: None,
            owner: "owner".into(),
            name: "name".into(),
        };
        app.cross_repo.repos.insert(
            id.clone(),
            crate::git::types::RepoMeta {
                local_path: None,
                local_path_resolved: false,
            },
        );
        app.resolve_local_path(&id);
        let meta = app.cross_repo.repos.get(&id).unwrap();
        assert!(meta.local_path_resolved);
        assert_eq!(meta.local_path.as_ref().unwrap(), &host_dir);
    }

    #[test]
    fn resolve_local_path_misses_when_dir_absent() {
        use crate::config::Config;
        use crate::git::types::RepoId;
        let tmp = tempfile::tempdir().unwrap();
        let mut app = App::new(Config::default());
        app.cross_repo.clone_root = Some(tmp.path().to_path_buf());
        let id = RepoId {
            host: None,
            owner: "x".into(),
            name: "y".into(),
        };
        app.cross_repo.repos.insert(
            id.clone(),
            crate::git::types::RepoMeta {
                local_path: None,
                local_path_resolved: false,
            },
        );
        app.resolve_local_path(&id);
        let meta = app.cross_repo.repos.get(&id).unwrap();
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
        use crate::git::types::{BranchEntry, PrState, PullRequest, RepoId};
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
        app.cross_repo.active_repo = Some(active.clone());
        app.view.main_filter = MainFilter::ReviewRequested;
        let make_pr = |num: u64, head: &str, repo: &RepoId| PullRequest {
            number: num,
            title: "t".into(),
            author: "u".into(),
            state: PrState::Open,
            head_ref: head.into(),
            updated_at: "2024".into(),
            is_draft: false,
            review_requests: vec![],
            latest_reviews: vec![],
            review_status: None,
            repo_id: repo.clone(),
        };
        app.view.entries = vec![
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
        assert_eq!(app.view.sidebar_scroll, 1);

        // j → should jump from index 1 to index 3 (skip Header at index 2)
        app.handle_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE));
        let rows = app.sidebar_rows();
        assert_eq!(app.view.sidebar_scroll, 3);
        assert!(matches!(
            rows[app.view.sidebar_scroll],
            SidebarRow::Entry(_)
        ));

        // k → should jump back from 3 to 1
        app.handle_key(KeyEvent::new(KeyCode::Char('k'), KeyModifiers::NONE));
        let rows = app.sidebar_rows();
        assert_eq!(app.view.sidebar_scroll, 1);
        assert!(matches!(
            rows[app.view.sidebar_scroll],
            SidebarRow::Entry(_)
        ));
    }
}

#[cfg(test)]
mod sidebar_rows_tests {
    use super::*;

    #[test]
    fn render_rows_groups_when_multi_repo() {
        use crate::app::SidebarRow;
        use crate::config::Config;
        use crate::git::types::{BranchEntry, PrState, PullRequest, RepoId};
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
        app.cross_repo.active_repo = Some(active.clone());
        app.view.main_filter = MainFilter::ReviewRequested;
        let make_pr = |num: u64, head: &str, repo: &RepoId| PullRequest {
            number: num,
            title: "x".into(),
            author: "u".into(),
            state: PrState::Open,
            head_ref: head.into(),
            updated_at: "2024".into(),
            is_draft: false,
            review_requests: vec![],
            latest_reviews: vec![],
            review_status: None,
            repo_id: repo.clone(),
        };
        app.view.entries = vec![
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
        use crate::git::types::{BranchEntry, PrState, PullRequest, RepoId};
        let mut app = App::new(Config::default());
        let active = RepoId {
            host: None,
            owner: "a".into(),
            name: "r1".into(),
        };
        app.cross_repo.active_repo = Some(active.clone());
        app.view.main_filter = MainFilter::ReviewRequested;
        let make_pr = |num: u64, head: &str| PullRequest {
            number: num,
            title: "x".into(),
            author: "u".into(),
            state: PrState::Open,
            head_ref: head.into(),
            updated_at: "2024".into(),
            is_draft: false,
            review_requests: vec![],
            latest_reviews: vec![],
            review_status: None,
            repo_id: active.clone(),
        };
        app.view.entries = vec![
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
        app.cross_repo.active_repo = Some(active.clone());
        app.view.main_filter = MainFilter::Local;
        // Local mode: filtered_entries requires has_local. Give entries a local branch.
        let make_branch = |name: &str| crate::git::types::Branch {
            name: name.into(),
            is_current: false,
            upstream: None,
            is_merged: false,
        };
        app.view.entries = vec![BranchEntry {
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
        use crate::git::types::{BranchEntry, PrState, PullRequest, RepoId, RepoMeta};
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
        app.cross_repo.active_repo = Some(active.clone());
        app.cross_repo.repos.insert(
            other.clone(),
            RepoMeta {
                local_path: None,
                local_path_resolved: true,
            },
        );
        app.view.main_filter = MainFilter::ReviewRequested;
        app.view.entries = vec![BranchEntry {
            name: "feat".into(),
            repo_id: other.clone(),
            local_branch: None,
            worktree: None,
            pull_request: Some(PullRequest {
                number: 1,
                title: "x".into(),
                author: "u".into(),
                state: PrState::Open,
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
        let menu = app.overlays.action_menu.as_ref().unwrap();
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
        use crate::git::types::{BranchEntry, PrState, PullRequest, RepoId, RepoMeta};
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
        app.cross_repo.active_repo = Some(active.clone());
        app.cross_repo.repos.insert(
            other.clone(),
            RepoMeta {
                local_path: Some(clone_path),
                local_path_resolved: true,
            },
        );
        app.view.main_filter = MainFilter::ReviewRequested;
        app.view.entries = vec![BranchEntry {
            name: "feat".into(),
            repo_id: other.clone(),
            local_branch: None,
            worktree: None,
            pull_request: Some(PullRequest {
                number: 1,
                title: "x".into(),
                author: "u".into(),
                state: PrState::Open,
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
        let menu = app.overlays.action_menu.as_ref().unwrap();
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
        app.cross_repo.active_repo = Some(active);
        app.view.entries = vec![BranchEntry {
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
        use crate::git::types::{Branch, BranchEntry, PrState, PullRequest, RepoId, Worktree};
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
        app.cross_repo.active_repo = Some(active.clone());

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
                state: PrState::Open,
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
        app.view.entries = vec![active_entry, cross_entry];

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
        use crate::git::types::{BranchEntry, PrState, PullRequest, RepoId, RepoMeta};
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
        app.cross_repo.active_repo = Some(active);
        app.cross_repo.clone_root = Some(tmp.path().to_path_buf());
        app.cross_repo.repos.insert(
            other.clone(),
            RepoMeta {
                local_path: None,
                local_path_resolved: false,
            },
        );
        app.view.main_filter = MainFilter::ReviewRequested;
        app.view.entries = vec![BranchEntry {
            name: "feat".into(),
            repo_id: other.clone(),
            local_branch: None,
            worktree: None,
            pull_request: Some(PullRequest {
                number: 1,
                title: "x".into(),
                author: "u".into(),
                state: PrState::Open,
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
        app.tick(); // fetches are debounced to the next tick
        assert!(
            app.commands
                .contains(&Command::LoadWorktreeList(other.clone()))
        );
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
        app.cross_repo.active_repo = Some(active.clone());
        let make_branch = |name: &str| crate::git::types::Branch {
            name: name.into(),
            is_current: false,
            upstream: None,
            is_merged: false,
        };
        app.view.entries = vec![BranchEntry {
            name: "feat".into(),
            repo_id: active.clone(),
            local_branch: Some(make_branch("feat")),
            worktree: None,
            pull_request: None,
            git_status: None,
        }];
        app.snap_scroll_to_entry();
        app.request_details_for_selection();
        app.tick();
        assert!(
            !app.commands
                .iter()
                .any(|c| matches!(c, Command::LoadWorktreeList(_)))
        );
    }
}

#[cfg(test)]
mod command_queue_tests {
    use super::*;
    use crate::config::Config;
    use crossterm::event::KeyModifiers;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn push_command_coalesces_duplicates_and_keeps_fifo_order() {
        let mut app = App::new(Config::default());
        app.push_command(Command::ReloadBranches);
        app.push_command(Command::ReloadCommits);
        app.push_command(Command::ReloadBranches); // duplicate — coalesced
        assert_eq!(app.commands.len(), 2);
        let drained: Vec<Command> = app.take_commands().into_iter().collect();
        assert_eq!(
            drained,
            vec![Command::ReloadBranches, Command::ReloadCommits]
        );
        assert!(app.commands.is_empty());
    }

    #[test]
    fn refresh_key_queues_branch_reload_and_pr_fetch() {
        let mut app = App::new(Config::default());
        app.handle_key(key(KeyCode::Char('r')));
        let drained: Vec<Command> = app.take_commands().into_iter().collect();
        assert_eq!(
            drained,
            vec![
                Command::ReloadBranches,
                Command::FetchPrs(MainFilter::Local)
            ]
        );
    }

    #[test]
    fn filter_switch_queues_pr_fetch_only_when_not_loaded() {
        let mut app = App::new(Config::default());
        app.handle_key(key(KeyCode::Char('2')));
        assert!(app.commands.contains(&Command::FetchPrs(MainFilter::MyPr)));

        let mut loaded = App::new(Config::default());
        loaded.prs.my_loaded = true;
        loaded.handle_key(key(KeyCode::Char('2')));
        assert!(
            !loaded
                .commands
                .contains(&Command::FetchPrs(MainFilter::MyPr))
        );
    }

    fn pr_entry(name: &str, number: u64) -> BranchEntry {
        use crate::git::types::{PrState, RepoId};
        let repo = RepoId {
            host: None,
            owner: "o".into(),
            name: "r".into(),
        };
        BranchEntry {
            name: name.into(),
            repo_id: repo.clone(),
            local_branch: Some(crate::git::types::Branch {
                name: name.into(),
                is_current: false,
                upstream: None,
                is_merged: false,
            }),
            worktree: None,
            pull_request: Some(PullRequest {
                number,
                title: "t".into(),
                author: "u".into(),
                state: PrState::Open,
                head_ref: name.into(),
                updated_at: "2024".into(),
                is_draft: false,
                review_requests: vec![],
                latest_reviews: vec![],
                review_status: None,
                repo_id: repo,
            }),
            git_status: None,
        }
    }

    fn app_with_pr_entries() -> App {
        use crate::git::types::RepoId;
        let mut app = App::new(Config::default());
        app.cross_repo.active_repo = Some(RepoId {
            host: None,
            owner: "o".into(),
            name: "r".into(),
        });
        app.view.entries = vec![pr_entry("a", 1), pr_entry("b", 2)];
        app
    }

    #[test]
    fn selection_detail_fetch_is_debounced_to_next_tick() {
        let mut app = app_with_pr_entries();
        app.request_details_for_selection();
        assert!(app.commands.is_empty(), "no fetch before the debounce tick");
        app.tick();
        assert!(
            app.commands
                .iter()
                .any(|c| matches!(c, Command::FetchPrDetail(_, 1)))
        );
    }

    #[test]
    fn rapid_selection_moves_coalesce_to_latest_target() {
        let mut app = app_with_pr_entries();
        // Two selection changes before any tick — like holding `j`.
        app.request_details_for_selection();
        app.view.sidebar_scroll = 1;
        app.request_details_for_selection();
        app.tick();
        let fetches: Vec<u64> = app
            .commands
            .iter()
            .filter_map(|c| match c {
                Command::FetchPrDetail(_, n) => Some(*n),
                _ => None,
            })
            .collect();
        assert_eq!(fetches, vec![2], "only the final selection is fetched");
    }

    #[test]
    fn cached_detail_queues_no_fetch_after_tick() {
        use crate::git::types::PrDetail;
        let mut app = app_with_pr_entries();
        let repo = app.view.entries[0].repo_id.clone();
        app.prs.detail.insert(
            (repo, 1),
            PrDetail {
                number: 1,
                body: String::new(),
                additions: 0,
                deletions: 0,
            },
        );
        app.request_details_for_selection();
        app.tick();
        assert!(app.commands.is_empty());
    }

    fn pending(on_confirm: Command) -> PendingConfirm {
        PendingConfirm {
            dialog: ConfirmDialog::new("title", "message"),
            on_confirm,
        }
    }

    #[test]
    fn confirm_y_pushes_on_confirm_and_closes_dialog() {
        let mut app = App::new(Config::default());
        app.overlays.confirm_dialog = Some(pending(Command::DeleteWorktree("/wt/p".into())));
        app.handle_key(key(KeyCode::Char('y')));
        assert!(app.overlays.confirm_dialog.is_none());
        assert!(
            app.commands
                .contains(&Command::DeleteWorktree("/wt/p".into()))
        );
    }

    #[test]
    fn confirm_decline_pushes_nothing() {
        let mut app = App::new(Config::default());
        app.overlays.confirm_dialog = Some(pending(Command::DeleteWorktree("/wt/p".into())));
        app.handle_key(key(KeyCode::Esc));
        assert!(app.overlays.confirm_dialog.is_none());
        assert!(app.commands.is_empty());
    }

    #[test]
    fn confirm_decline_force_delete_releases_inflight_path() {
        let mut app = App::new(Config::default());
        app.inflight.worktrees.insert("/wt/p".to_string());
        app.overlays.confirm_dialog = Some(pending(Command::ForceDeleteWorktree("/wt/p".into())));
        app.handle_key(key(KeyCode::Char('n')));
        assert!(app.overlays.confirm_dialog.is_none());
        assert!(app.commands.is_empty());
        assert!(!app.inflight.worktrees.contains("/wt/p"));
    }
}
