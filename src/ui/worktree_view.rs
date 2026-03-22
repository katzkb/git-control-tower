use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState},
};

use crate::app::App;
use crate::ui::confirm_dialog;

pub fn draw(frame: &mut Frame, area: Rect, app: &App) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Worktree (d:delete  j/k:navigate) ");

    if !app.wt_loaded {
        let loading = List::new(vec![ListItem::new("Loading...")]).block(block);
        frame.render_widget(loading, area);
        return;
    }

    if app.worktrees.is_empty() {
        let empty = List::new(vec![ListItem::new("No worktrees found")]).block(block);
        frame.render_widget(empty, area);
        return;
    }

    let items: Vec<ListItem> = app
        .worktrees
        .iter()
        .map(|wt| {
            let branch_str =
                wt.branch
                    .as_deref()
                    .unwrap_or(if wt.is_bare { "(bare)" } else { "(detached)" });
            let line = Line::from(vec![
                Span::styled(
                    format!("{:<20} ", branch_str),
                    Style::default().fg(Color::Cyan),
                ),
                Span::styled(
                    format!("{} ", &wt.head[..wt.head.len().min(8)]),
                    Style::default().fg(Color::Yellow),
                ),
                Span::styled(&wt.path, Style::default().fg(Color::White)),
            ]);
            ListItem::new(line)
        })
        .collect();

    let list = List::new(items)
        .block(block)
        .highlight_style(Style::default().add_modifier(Modifier::REVERSED));

    let mut state = ListState::default();
    state.select(Some(app.wt_scroll));

    frame.render_stateful_widget(list, area, &mut state);

    // Draw confirm dialog on top if active
    if let Some(dialog) = &app.confirm_dialog {
        confirm_dialog::draw(frame, dialog);
    }
}
