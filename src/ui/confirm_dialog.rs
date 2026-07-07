use ratatui::{
    Frame,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
};

use crate::ui::layout::centered_rect;
use crate::ui::theme;

#[derive(Debug, Clone)]
pub struct ConfirmDialog {
    pub title: String,
    pub message: String,
}

impl ConfirmDialog {
    pub fn new(title: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            message: message.into(),
        }
    }
}

pub fn draw(frame: &mut Frame, dialog: &ConfirmDialog) {
    let message_lines: Vec<&str> = dialog.message.lines().collect();
    let height = (message_lines.len() as u16) + 5; // padding + buttons + borders
    let area = centered_rect(50, height, frame.area());

    frame.render_widget(Clear, area);

    let mut lines = vec![Line::from("")];
    for msg_line in &message_lines {
        lines.push(Line::from(Span::raw(*msg_line)));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::styled(
            " y ",
            Style::default()
                .fg(theme::TEXT)
                .bg(theme::ERROR)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" Yes  "),
        Span::styled(
            " n ",
            Style::default()
                .fg(theme::TEXT)
                .bg(theme::TEXT_DIM)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" No"),
    ]));

    let block = Block::default()
        .borders(Borders::ALL)
        .title(format!(" {} ", dialog.title))
        .border_style(Style::default().fg(theme::WARNING));

    let paragraph = Paragraph::new(lines).block(block);
    frame.render_widget(paragraph, area);
}
