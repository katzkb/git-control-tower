use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState},
};

use crate::app::App;
use crate::git::types::BranchEntry;

pub fn draw(frame: &mut Frame, area: Rect, app: &App) {
    let chunks = Layout::vertical([Constraint::Length(1), Constraint::Min(1)]).split(area);

    draw_filter_bar(frame, chunks[0], app);
    draw_entry_list(frame, chunks[1], app);
}

fn draw_filter_bar(frame: &mut Frame, area: Rect, app: &App) {
    let label = app.main_filter.label();
    let bar = Line::from(vec![
        Span::styled(" Filter: ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            label,
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
    ]);
    frame.render_widget(
        ratatui::widgets::Paragraph::new(bar).style(Style::default().bg(Color::Black)),
        area,
    );
}

fn draw_entry_list(frame: &mut Frame, area: Rect, app: &App) {
    let filtered = app.filtered_entries();

    let items: Vec<ListItem> = filtered
        .iter()
        .map(|entry| ListItem::new(format_entry_line(entry)))
        .collect();

    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Branches ")
        .border_style(Style::default().fg(Color::DarkGray));

    let list = List::new(items)
        .block(block)
        .highlight_style(Style::default().add_modifier(Modifier::REVERSED));

    let mut state = ListState::default();
    state.select(Some(app.sidebar_scroll));
    frame.render_stateful_widget(list, area, &mut state);
}

fn format_entry_line(entry: &BranchEntry) -> Line<'static> {
    let mut spans: Vec<Span> = Vec::new();

    // Branch name
    let name_style = if entry.is_current() {
        Style::default()
            .fg(Color::Green)
            .add_modifier(Modifier::BOLD)
    } else if entry.is_merged() {
        Style::default().fg(Color::Yellow)
    } else {
        Style::default().fg(Color::White)
    };
    spans.push(Span::styled(format!(" {}", entry.name), name_style));

    // Current marker
    if entry.is_current() {
        spans.push(Span::styled(
            " *",
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        ));
    }

    // PR number
    if let Some(pr) = &entry.pull_request {
        spans.push(Span::styled(
            format!(" #{}", pr.number),
            Style::default().fg(Color::Yellow),
        ));
    }

    // Worktree indicator
    if entry.worktree.is_some() && !entry.is_current() {
        spans.push(Span::styled(" wt", Style::default().fg(Color::Cyan)));
    }

    // Merged tag
    if entry.is_merged() && !entry.is_current() {
        spans.push(Span::styled(
            " [merged]",
            Style::default().fg(Color::Yellow),
        ));
    }

    Line::from(spans)
}
