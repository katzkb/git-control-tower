use ratatui::{
    Frame,
    layout::{Constraint, Flex, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState},
};

use crate::app::App;
use crate::ui::{confirm_dialog, detail_pane, sidebar};

pub fn draw(frame: &mut Frame, area: Rect, app: &mut App) {
    let chunks =
        Layout::horizontal([Constraint::Percentage(35), Constraint::Percentage(65)]).split(area);

    sidebar::draw(frame, chunks[0], app);
    detail_pane::draw(frame, chunks[1], app);

    // Confirm dialog overlay
    if let Some(dialog) = &app.confirm_dialog {
        confirm_dialog::draw(frame, dialog);
    }

    // Action menu overlay
    if let Some(menu) = &app.action_menu {
        draw_action_menu(frame, menu);
    }
}

fn draw_action_menu(frame: &mut Frame, menu: &crate::app::ActionMenu) {
    let height = (menu.items.len() as u16) + 2; // items + border
    let area = centered_rect(40, height, frame.area());
    frame.render_widget(Clear, area);

    let items: Vec<ListItem> = menu
        .items
        .iter()
        .map(|item| {
            ListItem::new(Line::from(Span::styled(
                format!("  {}", item.label()),
                Style::default().fg(Color::White),
            )))
        })
        .collect();

    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Actions (j/k:navigate  Enter:select  Esc:cancel) ")
        .border_style(Style::default().fg(Color::Cyan));

    let list = List::new(items)
        .block(block)
        .highlight_style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▸ ");

    let mut state = ListState::default();
    state.select(Some(menu.scroll));
    frame.render_stateful_widget(list, area, &mut state);
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
