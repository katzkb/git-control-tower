use ratatui::{
    Frame,
    layout::{Constraint, Flex, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
};

use crate::app::BranchCreateInput;

pub fn draw(frame: &mut Frame, input: &BranchCreateInput) {
    let area = centered_rect(60, 8, frame.area());
    frame.render_widget(Clear, area);

    // Available width for the value portion of each row: modal inner width
    // minus the "  Name: " prefix (8) minus 1 column for the cursor on Name.
    // When the value is wider than this, we keep the trailing portion
    // visible (the typing position) and mark the hidden head with "…".
    let inner = area.width.saturating_sub(2) as usize;
    let from_prefix = "  From: ".len();
    let name_prefix = "  Name: ".len();
    let from_max = inner.saturating_sub(from_prefix);
    let name_max = inner.saturating_sub(name_prefix + 1);

    let source_display = truncate_head(&input.source, from_max);
    let name_display = truncate_head(&input.name, name_max);

    let lines = vec![
        Line::from(""),
        Line::from(vec![
            Span::styled("  From: ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                source_display,
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![
            Span::styled("  Name: ", Style::default().fg(Color::DarkGray)),
            Span::styled(name_display, Style::default().fg(Color::White)),
            Span::styled("_", Style::default().fg(Color::Cyan)),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled(
                " Enter ",
                Style::default()
                    .fg(Color::White)
                    .bg(Color::DarkGray)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" Create  "),
            Span::styled(
                " Esc ",
                Style::default()
                    .fg(Color::White)
                    .bg(Color::DarkGray)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" Cancel"),
        ]),
    ];

    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Create Branch ")
        .border_style(Style::default().fg(Color::Cyan));

    let paragraph = Paragraph::new(lines).block(block);
    frame.render_widget(paragraph, area);
}

fn truncate_head(s: &str, max: usize) -> String {
    if max == 0 {
        return String::new();
    }
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= max {
        return s.to_string();
    }
    // Reserve 1 char for the leading ellipsis.
    let start = chars.len() - (max - 1);
    let mut out = String::from("…");
    out.extend(&chars[start..]);
    out
}

fn centered_rect(percent_x: u16, height: u16, area: Rect) -> Rect {
    let vertical = Layout::vertical([Constraint::Length(height)])
        .flex(Flex::Center)
        .split(area);
    let horizontal = Layout::horizontal([Constraint::Percentage(percent_x)])
        .flex(Flex::Center)
        .split(vertical[0]);
    horizontal[0]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_head_fits() {
        assert_eq!(truncate_head("main", 10), "main");
    }

    #[test]
    fn truncate_head_cuts() {
        assert_eq!(truncate_head("feat/abcdefghij", 6), "…fghij");
    }

    #[test]
    fn truncate_head_zero_max() {
        assert_eq!(truncate_head("anything", 0), "");
    }

    #[test]
    fn truncate_head_unicode() {
        assert_eq!(truncate_head("αβγδε", 4), "…γδε");
    }
}
