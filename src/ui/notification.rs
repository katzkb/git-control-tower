use ratatui::{
    Frame,
    style::Style,
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
};

use crate::ui::layout::bottom_rect;
use crate::ui::theme;

const SUCCESS_TICKS: u32 = 38; // ~3 seconds at 80ms tick
const ERROR_TICKS: u32 = 63; // ~5 seconds at 80ms tick

#[derive(Debug, Clone)]
pub struct Notification {
    pub message: String,
    pub is_error: bool,
    pub ticks_remaining: u32,
}

impl Notification {
    pub fn success(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            is_error: false,
            ticks_remaining: SUCCESS_TICKS,
        }
    }

    pub fn error(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            is_error: true,
            ticks_remaining: ERROR_TICKS,
        }
    }
}

pub fn draw(frame: &mut Frame, notification: &Notification) {
    let area = bottom_rect(80, 3, frame.area());

    frame.render_widget(Clear, area);

    let color = if notification.is_error {
        theme::ERROR
    } else {
        theme::SUCCESS
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
