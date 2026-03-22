use crossterm::event::{KeyCode, KeyEvent};

use std::collections::HashSet;

use crate::git::types::{Branch, Commit, PrDetail, PullRequest, Worktree};
use crate::ui::confirm_dialog::ConfirmDialog;
use crate::ui::notification::Notification;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum ActiveView {
    #[default]
    Log,
    Pr,
    Branch,
    Worktree,
}

impl ActiveView {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Log => "Log",
            Self::Pr => "PR",
            Self::Branch => "Branch",
            Self::Worktree => "Worktree",
        }
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum PrFilter {
    #[default]
    All,
    AuthoredByMe,
    ReviewRequested,
}

impl PrFilter {
    pub fn label(&self) -> &'static str {
        match self {
            Self::All => "All",
            Self::AuthoredByMe => "Authored",
            Self::ReviewRequested => "Review Requested",
        }
    }
}

pub struct App {
    pub active_view: ActiveView,
    pub should_quit: bool,
    // Log View
    pub commits: Vec<Commit>,
    pub log_scroll: usize,
    // PR View
    pub pull_requests: Vec<PullRequest>,
    pub pr_scroll: usize,
    pub pr_filter: PrFilter,
    pub gh_user: String,
    pub prs_loaded: bool,
    // PR Detail
    pub pr_detail: Option<PrDetail>,
    pub pr_detail_scroll: usize,
    pub pr_detail_requested: Option<u64>,
    // Worktree View
    pub worktrees: Vec<Worktree>,
    pub wt_scroll: usize,
    pub wt_loaded: bool,
    pub confirm_dialog: Option<ConfirmDialog>,
    pub wt_delete_requested: Option<String>,
    // Worktree creation from PR
    pub wt_create_requested: Option<(String, u64)>, // (head_ref, pr_number)
    // Branch View
    pub branches: Vec<Branch>,
    pub branch_scroll: usize,
    pub branch_selected: HashSet<String>,
    pub branches_loaded: bool,
    pub branch_delete_requested: bool,
    // Notification
    pub notification: Option<Notification>,
    // Help
    pub show_help: bool,
}

impl App {
    pub fn new() -> Self {
        Self {
            active_view: ActiveView::default(),
            should_quit: false,
            commits: Vec::new(),
            log_scroll: 0,
            pull_requests: Vec::new(),
            pr_scroll: 0,
            pr_filter: PrFilter::default(),
            gh_user: String::new(),
            prs_loaded: false,
            pr_detail: None,
            pr_detail_scroll: 0,
            pr_detail_requested: None,
            worktrees: Vec::new(),
            wt_scroll: 0,
            wt_loaded: false,
            confirm_dialog: None,
            wt_delete_requested: None,
            wt_create_requested: None,
            branches: Vec::new(),
            branch_scroll: 0,
            branch_selected: HashSet::new(),
            branches_loaded: false,
            branch_delete_requested: false,
            notification: None,
            show_help: false,
        }
    }

    pub fn filtered_prs(&self) -> Vec<&PullRequest> {
        self.pull_requests
            .iter()
            .filter(|pr| match self.pr_filter {
                PrFilter::All => true,
                PrFilter::AuthoredByMe => pr.author == self.gh_user,
                PrFilter::ReviewRequested => {
                    pr.review_requests.iter().any(|r| r.login == self.gh_user)
                }
            })
            .collect()
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

        // PR detail: Esc/Backspace goes back to list instead of quitting
        if self.active_view == ActiveView::Pr && self.pr_detail.is_some() {
            self.handle_pr_detail_key(key.code);
            return;
        }

        match key.code {
            KeyCode::Char('?') => self.show_help = true,
            KeyCode::Char('q') | KeyCode::Esc => self.should_quit = true,
            KeyCode::Char('1') => self.active_view = ActiveView::Log,
            KeyCode::Char('2') => self.active_view = ActiveView::Pr,
            KeyCode::Char('3') => self.active_view = ActiveView::Branch,
            KeyCode::Char('4') => self.active_view = ActiveView::Worktree,
            _ => match self.active_view {
                ActiveView::Log => self.handle_log_key(key.code),
                ActiveView::Pr => self.handle_pr_key(key.code),
                ActiveView::Branch => self.handle_branch_key(key.code),
                ActiveView::Worktree => self.handle_wt_key(key.code),
            },
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

    fn handle_pr_key(&mut self, code: KeyCode) {
        let filtered = self.filtered_prs();
        let filtered_len = filtered.len();
        match code {
            KeyCode::Char('j') | KeyCode::Down => {
                if filtered_len > 0 && self.pr_scroll + 1 < filtered_len {
                    self.pr_scroll += 1;
                }
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.pr_scroll = self.pr_scroll.saturating_sub(1);
            }
            KeyCode::Enter => {
                if let Some(pr) = filtered.get(self.pr_scroll) {
                    self.pr_detail_requested = Some(pr.number);
                    self.pr_detail_scroll = 0;
                }
            }
            KeyCode::Char('a') => {
                self.pr_filter = if self.pr_filter == PrFilter::AuthoredByMe {
                    PrFilter::All
                } else {
                    PrFilter::AuthoredByMe
                };
                self.pr_scroll = 0;
            }
            KeyCode::Char('r') => {
                self.pr_filter = if self.pr_filter == PrFilter::ReviewRequested {
                    PrFilter::All
                } else {
                    PrFilter::ReviewRequested
                };
                self.pr_scroll = 0;
            }
            _ => {}
        }
    }

    fn handle_pr_detail_key(&mut self, code: KeyCode) {
        // Clear notification on any key press
        self.notification = None;

        match code {
            KeyCode::Esc | KeyCode::Backspace | KeyCode::Char('q') => {
                self.pr_detail = None;
                self.pr_detail_scroll = 0;
            }
            KeyCode::Char('j') | KeyCode::Down => {
                self.pr_detail_scroll += 1;
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.pr_detail_scroll = self.pr_detail_scroll.saturating_sub(1);
            }
            KeyCode::Char('w') => {
                if let Some(detail) = &self.pr_detail {
                    self.wt_create_requested = Some((detail.head_ref.clone(), detail.number));
                }
            }
            _ => {}
        }
    }

    fn handle_wt_key(&mut self, code: KeyCode) {
        let len = self.worktrees.len();
        match code {
            KeyCode::Char('j') | KeyCode::Down => {
                if len > 0 && self.wt_scroll + 1 < len {
                    self.wt_scroll += 1;
                }
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.wt_scroll = self.wt_scroll.saturating_sub(1);
            }
            KeyCode::Char('d') => {
                if let Some(wt) = self.worktrees.get(self.wt_scroll) {
                    // Don't allow deleting the main worktree (first one)
                    if self.wt_scroll == 0 {
                        return;
                    }
                    self.confirm_dialog = Some(ConfirmDialog::new(
                        "Delete Worktree",
                        format!("Remove worktree at {}?", wt.path),
                    ));
                }
            }
            _ => {}
        }
    }

    fn handle_branch_key(&mut self, code: KeyCode) {
        let len = self.branches.len();
        match code {
            KeyCode::Char('j') | KeyCode::Down => {
                if len > 0 && self.branch_scroll + 1 < len {
                    self.branch_scroll += 1;
                }
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.branch_scroll = self.branch_scroll.saturating_sub(1);
            }
            KeyCode::Char(' ') => {
                if let Some(branch) = self.branches.get(self.branch_scroll) {
                    // Don't allow selecting current branch or main/master
                    if branch.is_current || Self::is_protected_branch(&branch.name) {
                        return;
                    }
                    let name = branch.name.clone();
                    if self.branch_selected.contains(&name) {
                        self.branch_selected.remove(&name);
                    } else {
                        self.branch_selected.insert(name);
                    }
                }
            }
            KeyCode::Char('a') => {
                // Select all merged branches (except current and protected)
                for branch in &self.branches {
                    if branch.is_merged
                        && !branch.is_current
                        && !Self::is_protected_branch(&branch.name)
                    {
                        self.branch_selected.insert(branch.name.clone());
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
                }
            }
            _ => {}
        }
    }

    fn handle_confirm_key(&mut self, code: KeyCode) {
        match code {
            KeyCode::Char('y') => {
                match self.active_view {
                    ActiveView::Worktree => {
                        if let Some(wt) = self.worktrees.get(self.wt_scroll) {
                            self.wt_delete_requested = Some(wt.path.clone());
                        }
                    }
                    ActiveView::Branch => {
                        self.branch_delete_requested = true;
                    }
                    _ => {}
                }
                self.confirm_dialog = None;
            }
            KeyCode::Char('n') | KeyCode::Esc => {
                self.confirm_dialog = None;
            }
            _ => {}
        }
    }

    fn is_protected_branch(name: &str) -> bool {
        matches!(name, "main" | "master")
    }
}
