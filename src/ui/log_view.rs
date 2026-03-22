use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState},
};

use crate::app::App;

pub fn draw(frame: &mut Frame, area: Rect, app: &App) {
    let block = Block::default().borders(Borders::ALL).title(" Log ");

    if app.commits.is_empty() {
        let loading = List::new(vec![ListItem::new("Loading...")]).block(block);
        frame.render_widget(loading, area);
        return;
    }

    let items: Vec<ListItem> = app
        .commits
        .iter()
        .map(|commit| {
            let line = Line::from(vec![
                Span::styled(
                    format!("{} ", commit.hash),
                    Style::default().fg(Color::Yellow),
                ),
                Span::raw(&commit.message),
                Span::styled(
                    format!("  ({}, {})", commit.author, commit.date),
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
    state.select(Some(app.log_scroll));

    frame.render_stateful_widget(list, area, &mut state);
}
