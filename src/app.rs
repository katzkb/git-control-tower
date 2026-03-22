use crossterm::event::{KeyCode, KeyEvent};

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
}

impl App {
    pub fn new() -> Self {
        Self {
            active_view: ActiveView::default(),
            should_quit: false,
        }
    }

    pub fn handle_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => self.should_quit = true,
            KeyCode::Char('1') => self.active_view = ActiveView::Log,
            KeyCode::Char('2') => self.active_view = ActiveView::Pr,
            KeyCode::Char('3') => self.active_view = ActiveView::Branch,
            KeyCode::Char('4') => self.active_view = ActiveView::Worktree,
            _ => {}
        }
    }
}
