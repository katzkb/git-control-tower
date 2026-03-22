use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
};

use crate::app::App;
use crate::ui::{confirm_dialog, detail_pane, sidebar};

pub fn draw(frame: &mut Frame, area: Rect, app: &App) {
    let chunks =
        Layout::horizontal([Constraint::Percentage(35), Constraint::Percentage(65)]).split(area);

    sidebar::draw(frame, chunks[0], app);
    detail_pane::draw(frame, chunks[1], app);

    // Confirm dialog overlay (used by PR C actions)
    if let Some(dialog) = &app.confirm_dialog {
        confirm_dialog::draw(frame, dialog);
    }
}
