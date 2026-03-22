use ratatui::{
    Frame,
    layout::Rect,
    widgets::{Block, Borders, Paragraph},
};

pub fn draw(frame: &mut Frame, area: Rect) {
    let block = Block::default().borders(Borders::ALL).title(" PR ");
    let content = Paragraph::new("PR View").block(block);
    frame.render_widget(content, area);
}
