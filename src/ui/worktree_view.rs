use ratatui::{
    Frame,
    layout::Rect,
    widgets::{Block, Borders, Paragraph},
};

pub fn draw(frame: &mut Frame, area: Rect) {
    let block = Block::default().borders(Borders::ALL).title(" Worktree ");
    let content = Paragraph::new("Worktree View").block(block);
    frame.render_widget(content, area);
}
