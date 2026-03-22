use ratatui::{
    Frame,
    layout::{Constraint, Flex, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
};

pub fn draw(frame: &mut Frame) {
    let area = centered_rect(60, 24, frame.area());
    frame.render_widget(Clear, area);

    let lines = vec![
        Line::from(Span::styled(
            "Git Control Tower — Keyboard Shortcuts",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        section("Global"),
        key_line("1", "Log View"),
        key_line("2", "PR View"),
        key_line("3", "Branch View"),
        key_line("4", "Worktree View"),
        key_line("?", "Toggle this help"),
        key_line("q / Esc", "Quit"),
        Line::from(""),
        section("Log View"),
        key_line("j/k ↑/↓", "Navigate commits"),
        Line::from(""),
        section("PR View"),
        key_line("j/k ↑/↓", "Navigate PRs"),
        key_line("a", "Filter: authored by me"),
        key_line("r", "Filter: review requested"),
        key_line("Enter", "View PR detail"),
        key_line("w", "Create worktree (in detail)"),
        key_line("Esc", "Back to list (in detail)"),
        Line::from(""),
        section("Branch View"),
        key_line("j/k ↑/↓", "Navigate branches"),
        key_line("Space", "Toggle selection"),
        key_line("a", "Select all merged"),
        key_line("d", "Delete selected"),
        Line::from(""),
        section("Worktree View"),
        key_line("j/k ↑/↓", "Navigate worktrees"),
        key_line("d", "Delete worktree"),
        Line::from(""),
        Line::from(Span::styled(
            "Press ? or Esc to close",
            Style::default().fg(Color::DarkGray),
        )),
    ];

    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Help ")
        .border_style(Style::default().fg(Color::Cyan));

    let paragraph = Paragraph::new(lines).block(block);
    frame.render_widget(paragraph, area);
}

fn section(name: &str) -> Line<'static> {
    Line::from(Span::styled(
        format!("  {name}"),
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
    ))
}

fn key_line(key: &str, desc: &str) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("    {key:<14}"), Style::default().fg(Color::Green)),
        Span::raw(desc.to_string()),
    ])
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
