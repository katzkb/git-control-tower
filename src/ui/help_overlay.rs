use ratatui::{
    Frame,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
};

use crate::ui::layout::centered_rect;
use crate::ui::theme;

pub fn draw(frame: &mut Frame) {
    let lines = vec![
        Line::from(Span::styled(
            "Git Control Tower — Keyboard Shortcuts",
            Style::default()
                .fg(theme::ACCENT)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        section("Global"),
        key_line("1", "Filter: Local"),
        key_line("2", "Filter: My PR"),
        key_line("3", "Filter: Review"),
        key_line("l", "Log View"),
        key_line("h", "History View"),
        key_line("r", "Refresh current view"),
        key_line("?", "Toggle this help"),
        key_line("q", "Quit"),
        Line::from(""),
        section("Main View"),
        key_line("j/k ↑/↓", "Navigate sidebar"),
        key_line("→/←", "Focus PR detail / back"),
        key_line("J/K", "Scroll PR detail (j/k when focused)"),
        key_line("Space", "Toggle selection"),
        key_line("a", "Select all merged"),
        key_line("d", "Delete branch/worktree"),
        key_line("w", "Create worktree from PR"),
        key_line("m", "Toggle merged PRs (My PR / Review)"),
        key_line("t", "Toggle team reviews (Review)"),
        key_line("Enter", "Action menu"),
        key_line("/", "Search branches"),
        key_line("Esc", "Quit"),
        Line::from(""),
        section("Log View"),
        key_line("j/k ↑/↓", "Navigate commits"),
        key_line("Esc", "Back to Main"),
        Line::from(""),
        section("History View"),
        key_line("j/k ↑/↓", "Navigate commands"),
        key_line("Esc", "Back to Main"),
        Line::from(""),
        Line::from(Span::styled(
            "Press ? or Esc to close",
            Style::default().fg(theme::TEXT_DIM),
        )),
    ];

    // Content height + 2 border rows — computed so the overlay never drifts
    // out of sync when shortcuts are added or removed above.
    let height = lines.len() as u16 + 2;
    let area = centered_rect(60, height, frame.area());
    frame.render_widget(Clear, area);

    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Help ")
        .border_style(Style::default().fg(theme::ACCENT));

    let paragraph = Paragraph::new(lines).block(block);
    frame.render_widget(paragraph, area);
}

fn section(name: &str) -> Line<'static> {
    Line::from(Span::styled(
        format!("  {name}"),
        Style::default()
            .fg(theme::WARNING)
            .add_modifier(Modifier::BOLD),
    ))
}

fn key_line(key: &str, desc: &str) -> Line<'static> {
    Line::from(vec![
        Span::styled(
            format!("    {key:<14}"),
            Style::default().fg(theme::SUCCESS),
        ),
        Span::raw(desc.to_string()),
    ])
}
