use ratatui::{
    Frame,
    layout::{Constraint, Flex, Layout, Rect},
    style::{Color, Style},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
};

#[derive(Debug, Clone)]
pub struct Notification {
    pub message: String,
    pub is_error: bool,
}

impl Notification {
    #[allow(dead_code)]
    pub fn success(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            is_error: false,
        }
    }

    #[allow(dead_code)]
    pub fn error(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            is_error: true,
        }
    }
}

pub fn draw(frame: &mut Frame, notification: &Notification) {
    let area = bottom_rect(80, 3, frame.area());

    frame.render_widget(Clear, area);

    let color = if notification.is_error {
        Color::Red
    } else {
        Color::Green
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(color));

    let paragraph = Paragraph::new(notification.message.as_str())
        .block(block)
        .style(Style::default().fg(color))
        .wrap(Wrap { trim: false });

    frame.render_widget(paragraph, area);
}

fn bottom_rect(percent_x: u16, height: u16, area: Rect) -> Rect {
    let vertical = Layout::vertical([Constraint::Min(0), Constraint::Length(height)]).split(area);
    let horizontal = Layout::horizontal([Constraint::Percentage(percent_x)])
        .flex(Flex::Center)
        .split(vertical[1]);
    horizontal[0]
}
