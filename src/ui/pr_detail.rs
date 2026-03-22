use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
};

use crate::git::types::PrDetail;
use crate::ui::markdown::render_markdown;

pub fn draw(frame: &mut Frame, area: Rect, detail: &PrDetail, scroll: usize) {
    let chunks = Layout::vertical([Constraint::Length(4), Constraint::Min(1)]).split(area);

    // Header: metadata
    let header_lines = vec![
        Line::from(vec![
            Span::styled(
                format!("#{} ", detail.number),
                Style::default().fg(Color::Yellow),
            ),
            Span::styled(&detail.title, Style::default().add_modifier(Modifier::BOLD)),
        ]),
        Line::from(vec![
            Span::styled("Author: ", Style::default().fg(Color::DarkGray)),
            Span::raw(&detail.author),
            Span::raw("  "),
            Span::styled("State: ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                &detail.state,
                Style::default().fg(match detail.state.as_str() {
                    "OPEN" => Color::Green,
                    "CLOSED" => Color::Red,
                    "MERGED" => Color::Magenta,
                    _ => Color::White,
                }),
            ),
            Span::raw("  "),
            Span::styled("Branch: ", Style::default().fg(Color::DarkGray)),
            Span::raw(&detail.head_ref),
        ]),
        Line::from(vec![
            Span::styled(
                format!("+{}", detail.additions),
                Style::default().fg(Color::Green),
            ),
            Span::raw(" / "),
            Span::styled(
                format!("-{}", detail.deletions),
                Style::default().fg(Color::Red),
            ),
        ]),
    ];

    let header = Paragraph::new(header_lines).block(
        Block::default()
            .borders(Borders::BOTTOM)
            .title(" PR Detail "),
    );
    frame.render_widget(header, chunks[0]);

    // Body: markdown
    let body_lines = if detail.body.is_empty() {
        vec![Line::from(Span::styled(
            "(no description)",
            Style::default().fg(Color::DarkGray),
        ))]
    } else {
        render_markdown(&detail.body)
    };

    let body = Paragraph::new(body_lines)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Description (j/k:scroll  Esc:back) "),
        )
        .wrap(Wrap { trim: false })
        .scroll((scroll as u16, 0));
    frame.render_widget(body, chunks[1]);
}
