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
        .title(" Branch (Space:select  a:select-merged  d:delete  j/k:navigate) ");

    if !app.branches_loaded {
        let loading = List::new(vec![ListItem::new("Loading...")]).block(block);
        frame.render_widget(loading, area);
        return;
    }

    if app.branches.is_empty() {
        let empty = List::new(vec![ListItem::new("No branches found")]).block(block);
        frame.render_widget(empty, area);
        return;
    }

    let items: Vec<ListItem> = app
        .branches
        .iter()
        .map(|branch| {
            let checkbox = if app.branch_selected.contains(&branch.name) {
                "[x] "
            } else {
                "[ ] "
            };

            let current_marker = if branch.is_current { "* " } else { "  " };

            let name_style = if branch.is_merged && !branch.is_current {
                Style::default().fg(Color::Yellow)
            } else if branch.is_current {
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White)
            };

            let merged_tag = if branch.is_merged {
                Span::styled(" [merged]", Style::default().fg(Color::Yellow))
            } else {
                Span::raw("")
            };

            let upstream_info = branch
                .upstream
                .as_ref()
                .map(|u| Span::styled(format!(" → {u}"), Style::default().fg(Color::DarkGray)))
                .unwrap_or_else(|| Span::raw(""));

            let line = Line::from(vec![
                Span::styled(checkbox, Style::default().fg(Color::Cyan)),
                Span::raw(current_marker),
                Span::styled(&branch.name, name_style),
                merged_tag,
                upstream_info,
            ]);
            ListItem::new(line)
        })
        .collect();

    let list = List::new(items)
        .block(block)
        .highlight_style(Style::default().add_modifier(Modifier::REVERSED));

    let mut state = ListState::default();
    state.select(Some(app.branch_scroll));

    frame.render_stateful_widget(list, area, &mut state);

    // Draw confirm dialog on top if active
    if let Some(dialog) = &app.confirm_dialog {
        confirm_dialog::draw(frame, dialog);
    }
}
