use crossterm::event::{KeyCode, KeyEvent};

use crate::git::types::Commit;

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

pub struct App {
    pub active_view: ActiveView,
    pub should_quit: bool,
    pub commits: Vec<Commit>,
    pub log_scroll: usize,
}

impl App {
    pub fn new() -> Self {
        Self {
            active_view: ActiveView::default(),
            should_quit: false,
            commits: Vec::new(),
            log_scroll: 0,
        }
    }

    pub fn handle_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => self.should_quit = true,
            KeyCode::Char('1') => self.active_view = ActiveView::Log,
            KeyCode::Char('2') => self.active_view = ActiveView::Pr,
            KeyCode::Char('3') => self.active_view = ActiveView::Branch,
            KeyCode::Char('4') => self.active_view = ActiveView::Worktree,
            _ => {
                if self.active_view == ActiveView::Log {
                    self.handle_log_key(key.code);
                }
            }
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
}
