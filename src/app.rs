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

pub struct App {
    pub active_view: ActiveView,
    pub should_quit: bool,

    // Log View
    pub commits: Vec<Commit>,
    pub log_scroll: usize,

    // Main View — unified entries
    pub entries: Vec<BranchEntry>,
    pub entries_loaded: bool,
    pub main_filter: MainFilter,
    pub sidebar_scroll: usize,

    // Raw data sources (for merge_entries and async reload)
    pub branches: Vec<Branch>,
    pub worktrees: Vec<Worktree>,
    pub pull_requests: Vec<PullRequest>,
    pub gh_user: String,

    // PR Detail (for detail pane, cached by PR number)
    pub pr_detail_cache: HashMap<u64, PrDetail>,
    pub pr_detail_scroll: usize,
    pub pr_detail_requested: Option<u64>,

    // Git status loading
    pub git_status_requested: Option<String>, // worktree path

    // Overlays
    pub confirm_dialog: Option<ConfirmDialog>,
    pub notification: Option<Notification>,
    pub show_help: bool,

    // Action requests
    pub wt_delete_requested: Option<String>,
    pub wt_create_requested: Option<(String, u64)>,
    pub branch_selected: HashSet<String>,
    pub branch_delete_requested: bool,
}

impl App {
    pub fn new() -> Self {
        Self {
            active_view: ActiveView::default(),
            should_quit: false,
            commits: Vec::new(),
            log_scroll: 0,
            entries: Vec::new(),
            entries_loaded: false,
            main_filter: MainFilter::default(),
            sidebar_scroll: 0,
            branches: Vec::new(),
            worktrees: Vec::new(),
            pull_requests: Vec::new(),
            gh_user: String::new(),
            pr_detail_cache: HashMap::new(),
            pr_detail_scroll: 0,
            pr_detail_requested: None,
            git_status_requested: None,
            confirm_dialog: None,
            notification: None,
            show_help: false,
            wt_delete_requested: None,
            wt_create_requested: None,
            branch_selected: HashSet::new(),
            branch_delete_requested: false,
        }
    }

    pub fn filtered_entries(&self) -> Vec<&BranchEntry> {
        self.entries
            .iter()
            .filter(|entry| match self.main_filter {
                MainFilter::Local => entry.has_local(),
                MainFilter::MyPr => entry
                    .pull_request
                    .as_ref()
                    .is_some_and(|pr| pr.author == self.gh_user),
                MainFilter::ReviewRequested => entry
                    .pull_request
                    .as_ref()
                    .is_some_and(|pr| pr.review_requests.iter().any(|r| r.login == self.gh_user)),
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

        match key.code {
            KeyCode::Char('?') => self.show_help = true,
            KeyCode::Char('q') => self.should_quit = true,
            KeyCode::Esc => {
                if self.active_view == ActiveView::Log {
                    self.active_view = ActiveView::Main;
                } else {
                    self.should_quit = true;
                }
            }
            KeyCode::Char('l') => self.active_view = ActiveView::Log,
            KeyCode::Char('1') => {
                self.main_filter = MainFilter::Local;
                self.active_view = ActiveView::Main;
                self.sidebar_scroll = 0;
                self.request_details_for_selection();
            }
            KeyCode::Char('2') => {
                self.main_filter = MainFilter::MyPr;
                self.active_view = ActiveView::Main;
                self.sidebar_scroll = 0;
                self.request_details_for_selection();
            }
            KeyCode::Char('3') => {
                self.main_filter = MainFilter::ReviewRequested;
                self.active_view = ActiveView::Main;
                self.sidebar_scroll = 0;
                self.request_details_for_selection();
            }
            _ => match self.active_view {
                ActiveView::Main => self.handle_main_key(key.code),
                ActiveView::Log => self.handle_log_key(key.code),
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
                if let Some(entry) = self.selected_entry() {
                    let name = entry.name.clone();
                    if !entry.is_current() && !Self::is_protected_branch(&name) {
                        if self.branch_selected.contains(&name) {
                            self.branch_selected.remove(&name);
                        } else {
                            self.branch_selected.insert(name);
                        }
                    }
                }
            }
            KeyCode::Char('a') => {
                for entry in &self.entries {
                    if entry.is_merged()
                        && !entry.is_current()
                        && !Self::is_protected_branch(&entry.name)
                    {
                        self.branch_selected.insert(entry.name.clone());
                    }
                }
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
                } else if let Some(entry) = self.selected_entry()
                    && let Some(wt_path) = entry.worktree_path()
                    && !entry.is_current()
                {
                    self.confirm_dialog = Some(ConfirmDialog::new(
                        "Delete Worktree",
                        format!("Remove worktree at {wt_path}?"),
                    ));
                }
            }
            KeyCode::Char('w') => {
                if let Some(entry) = self.selected_entry()
                    && entry.worktree.is_none()
                    && let Some(pr) = &entry.pull_request
                {
                    self.wt_create_requested = Some((entry.name.clone(), pr.number));
                }
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

    fn handle_confirm_key(&mut self, code: KeyCode) {
        match code {
            KeyCode::Char('y') => {
                if !self.branch_selected.is_empty() {
                    self.branch_delete_requested = true;
                } else if let Some(entry) = self.selected_entry()
                    && let Some(wt_path) = entry.worktree_path()
                {
                    self.wt_delete_requested = Some(wt_path.to_string());
                }
                self.confirm_dialog = None;
            }
            KeyCode::Char('n') | KeyCode::Esc => {
                self.confirm_dialog = None;
            }
            _ => {}
        }
    }

    pub fn is_protected_branch(name: &str) -> bool {
        matches!(name, "main" | "master")
    }
}
