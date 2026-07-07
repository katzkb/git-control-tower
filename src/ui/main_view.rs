use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap},
};

use crate::app::App;
use crate::ui::{confirm_dialog, detail_pane, sidebar};

use crate::ui::layout::centered_rect;
use crate::ui::theme;

pub fn draw(frame: &mut Frame, area: Rect, app: &mut App) {
    let chunks =
        Layout::horizontal([Constraint::Percentage(35), Constraint::Percentage(65)]).split(area);

    sidebar::draw(frame, chunks[0], app);
    detail_pane::draw(frame, chunks[1], app);

    // Confirm dialog overlay
    if let Some(pending) = &app.overlays.confirm_dialog {
        confirm_dialog::draw(frame, &pending.dialog);
    }

    // Action menu overlay
    if let Some(menu) = &app.overlays.action_menu {
        draw_action_menu(frame, menu);
    }
}

/// A display row of the action menu: a selectable item or a group separator.
/// Separators exist only at render time; selection state (`menu.scroll`)
/// keeps indexing items alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MenuRow {
    Item(crate::app::ActionItem),
    Separator(&'static str),
}

/// Insert a labeled separator before each item whose group differs from the
/// previous item's group. Empty groups never appear, so no trailing or
/// leading separators are produced (the top section is ungrouped).
fn build_menu_rows(items: &[crate::app::ActionItem]) -> Vec<MenuRow> {
    let mut rows = Vec::with_capacity(items.len());
    let mut prev_group = None;
    for item in items {
        let group = item.group_label();
        if group != prev_group
            && let Some(label) = group
        {
            rows.push(MenuRow::Separator(label));
        }
        prev_group = group;
        rows.push(MenuRow::Item(*item));
    }
    rows
}

/// Map an index into `menu.items` to its display-row index (items plus
/// separators), so the highlight lands on the right row.
fn display_index(rows: &[MenuRow], item_index: usize) -> usize {
    let mut seen_items = 0;
    for (row_idx, row) in rows.iter().enumerate() {
        if matches!(row, MenuRow::Item(_)) {
            if seen_items == item_index {
                return row_idx;
            }
            seen_items += 1;
        }
    }
    rows.len().saturating_sub(1)
}

fn draw_action_menu(frame: &mut Frame, menu: &crate::app::ActionMenu) {
    let rows = build_menu_rows(&menu.items);
    let footer_height: u16 = if menu.footer.is_some() { 1 } else { 0 };
    let list_height = (rows.len() as u16) + 2; // rows + border
    let total_height = list_height + footer_height;
    let area = centered_rect(40, total_height, frame.area());
    frame.render_widget(Clear, area);

    // Split the area into list portion and optional footer.
    let [list_area, footer_area] = Layout::vertical([
        Constraint::Length(list_height),
        Constraint::Length(footer_height),
    ])
    .areas(area);

    // Width inside the borders, minus the 2-char highlight symbol column.
    let inner_width = list_area.width.saturating_sub(2 + 2) as usize;
    let items: Vec<ListItem> = rows
        .iter()
        .map(|row| match row {
            MenuRow::Item(item) => ListItem::new(Line::from(Span::styled(
                format!("  {}", item.label()),
                Style::default().fg(theme::TEXT),
            ))),
            MenuRow::Separator(label) => {
                let head = format!("── {label} ");
                let fill = "─".repeat(inner_width.saturating_sub(head.chars().count()));
                ListItem::new(Line::from(Span::styled(
                    format!("{head}{fill}"),
                    Style::default().fg(theme::TEXT_DIM),
                )))
            }
        })
        .collect();

    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Actions (j/k:navigate  Enter:select  Esc:cancel) ")
        .border_style(Style::default().fg(theme::ACCENT));

    let list = List::new(items)
        .block(block)
        .highlight_style(
            Style::default()
                .fg(theme::ACCENT)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▸ ");

    let mut state = ListState::default();
    state.select(Some(display_index(&rows, menu.scroll)));
    frame.render_stateful_widget(list, list_area, &mut state);

    if let Some(ref footer) = menu.footer {
        let footer_line = Line::from(Span::styled(
            format!("─ {footer}"),
            Style::default().fg(theme::TEXT_DIM),
        ));
        let paragraph = Paragraph::new(footer_line).wrap(Wrap { trim: true });
        frame.render_widget(paragraph, footer_area);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::ActionItem;

    #[test]
    fn build_menu_rows_inserts_separators_at_group_boundaries() {
        let items = vec![
            ActionItem::OpenPrInBrowser,
            ActionItem::CopyBranchName,
            ActionItem::CdIntoWorktree,
            ActionItem::DeleteWorktree,
            ActionItem::CreateBranch,
            ActionItem::DeleteBranch,
        ];
        let rows = build_menu_rows(&items);
        assert_eq!(
            rows,
            vec![
                MenuRow::Item(ActionItem::OpenPrInBrowser),
                MenuRow::Item(ActionItem::CopyBranchName),
                MenuRow::Separator("Worktree"),
                MenuRow::Item(ActionItem::CdIntoWorktree),
                MenuRow::Item(ActionItem::DeleteWorktree),
                MenuRow::Separator("Branch"),
                MenuRow::Item(ActionItem::CreateBranch),
                MenuRow::Item(ActionItem::DeleteBranch),
            ]
        );
    }

    #[test]
    fn build_menu_rows_skips_separator_for_absent_group() {
        // No worktree items: the "Worktree" separator must not appear.
        let items = vec![ActionItem::CopyBranchName, ActionItem::CreateBranch];
        let rows = build_menu_rows(&items);
        assert_eq!(
            rows,
            vec![
                MenuRow::Item(ActionItem::CopyBranchName),
                MenuRow::Separator("Branch"),
                MenuRow::Item(ActionItem::CreateBranch),
            ]
        );
    }

    #[test]
    fn display_index_accounts_for_separators() {
        let items = vec![
            ActionItem::CopyBranchName,
            ActionItem::CdIntoWorktree,
            ActionItem::CreateBranch,
        ];
        // Rows: Copy, ──Worktree──, cd, ──Branch──, Create
        let rows = build_menu_rows(&items);
        assert_eq!(display_index(&rows, 0), 0);
        assert_eq!(display_index(&rows, 1), 2);
        assert_eq!(display_index(&rows, 2), 4);
    }
}
