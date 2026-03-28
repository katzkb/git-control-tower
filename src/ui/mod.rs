pub mod confirm_dialog;
mod detail_pane;
mod help_overlay;
mod log_view;
mod main_view;
pub mod markdown;
pub mod notification;
pub mod sidebar;

use ratatui::{
    Frame,
    layout::{Constraint, Layout},
    style::{Color, Style},
    widgets::Paragraph,
};

use crate::app::{ActiveView, App, MainFilter};

pub fn draw(frame: &mut Frame, app: &App) {
    let chunks = Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).split(frame.area());

    match app.active_view {
        ActiveView::Main => main_view::draw(frame, chunks[0], app),
        ActiveView::Log => log_view::draw(frame, chunks[0], app),
    }

    // Status bar
    let status_text = match app.active_view {
        ActiveView::Main => {
            let merged_hint = if matches!(
                app.main_filter,
                MainFilter::MyPr | MainFilter::ReviewRequested
            ) {
                "  m:Merged"
            } else {
                ""
            };
            let team_hint = if app.main_filter == MainFilter::ReviewRequested {
                "  t:Team"
            } else {
                ""
            };
            format!(
                " [{}]  1:Local  2:My PR  3:Review  Enter:Actions  /:Search{merged_hint}{team_hint}  l:Log  ?:Help  q:Quit",
                app.main_filter.label()
            )
        }
        ActiveView::Log => " [Log]  1:Local  2:My PR  3:Review  l:Log  ?:Help  q:Quit".to_string(),
    };
    let status =
        Paragraph::new(status_text).style(Style::default().fg(Color::White).bg(Color::DarkGray));
    frame.render_widget(status, chunks[1]);

    // Notification overlay
    if let Some(notif) = &app.notification {
        notification::draw(frame, notif);
    }

    // Help overlay
    if app.show_help {
        help_overlay::draw(frame);
    }
}
