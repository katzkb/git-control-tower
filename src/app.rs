use crossterm::event::{KeyCode, KeyEvent};

use crate::git::types::{Commit, PullRequest};

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
        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => self.should_quit = true,
            KeyCode::Char('1') => self.active_view = ActiveView::Log,
            KeyCode::Char('2') => self.active_view = ActiveView::Pr,
            KeyCode::Char('3') => self.active_view = ActiveView::Branch,
            KeyCode::Char('4') => self.active_view = ActiveView::Worktree,
            _ => match self.active_view {
                ActiveView::Log => self.handle_log_key(key.code),
                ActiveView::Pr => self.handle_pr_key(key.code),
                _ => {}
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
        let filtered_len = self.filtered_prs().len();
        match code {
            KeyCode::Char('j') | KeyCode::Down => {
                if filtered_len > 0 && self.pr_scroll + 1 < filtered_len {
                    self.pr_scroll += 1;
                }
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.pr_scroll = self.pr_scroll.saturating_sub(1);
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
}
