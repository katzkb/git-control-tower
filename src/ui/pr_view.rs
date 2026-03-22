use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph},
};

use crate::app::App;

pub fn draw(frame: &mut Frame, area: Rect, app: &App) {
    let chunks = Layout::vertical([Constraint::Length(1), Constraint::Min(1)]).split(area);

    // Filter bar
    let filter_text = format!(
        " Filter: [{}]  a:Authored  r:Review Requested  j/k:Navigate",
        app.pr_filter.label()
    );
    let filter_bar =
        Paragraph::new(filter_text).style(Style::default().fg(Color::Cyan).bg(Color::Black));
    frame.render_widget(filter_bar, chunks[0]);

    let block = Block::default().borders(Borders::ALL).title(" PR ");

    if !app.prs_loaded {
        let loading = List::new(vec![ListItem::new("Loading...")]).block(block);
        frame.render_widget(loading, chunks[1]);
        return;
    }

    let filtered = app.filtered_prs();

    if filtered.is_empty() {
        let empty = List::new(vec![ListItem::new("No PRs found")]).block(block);
        frame.render_widget(empty, chunks[1]);
        return;
    }

    let items: Vec<ListItem> = filtered
        .iter()
        .map(|pr| {
            let state_color = match pr.state.as_str() {
                "OPEN" => Color::Green,
                "CLOSED" => Color::Red,
                "MERGED" => Color::Magenta,
                _ => Color::White,
            };

            let line = Line::from(vec![
                Span::styled(
                    format!("#{:<5} ", pr.number),
                    Style::default().fg(Color::Yellow),
                ),
                Span::styled(
                    format!("{:<8} ", pr.state),
                    Style::default().fg(state_color),
                ),
                Span::raw(&pr.title),
                Span::styled(
                    format!("  ({})", pr.author),
                    Style::default().fg(Color::DarkGray),
                ),
            ]);
            ListItem::new(line)
        })
        .collect();

    let list = List::new(items)
        .block(block)
        .highlight_style(Style::default().add_modifier(Modifier::REVERSED));

    let mut state = ListState::default();
    state.select(Some(app.pr_scroll));

    frame.render_stateful_widget(list, chunks[1], &mut state);
}
