mod branch_view;
pub mod confirm_dialog;
mod log_view;
pub mod markdown;
mod pr_detail;
mod pr_view;
mod worktree_view;

use ratatui::{
    Frame,
    layout::{Constraint, Layout},
    style::{Color, Style},
    widgets::Paragraph,
};

use crate::app::{ActiveView, App};

pub fn draw(frame: &mut Frame, app: &App) {
    let chunks = Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).split(frame.area());

    match app.active_view {
        ActiveView::Log => log_view::draw(frame, chunks[0], app),
        ActiveView::Pr => pr_view::draw(frame, chunks[0], app),
        ActiveView::Branch => branch_view::draw(frame, chunks[0]),
        ActiveView::Worktree => worktree_view::draw(frame, chunks[0], app),
    }

    let status = Paragraph::new(format!(
        " [{}]  1:Log  2:PR  3:Branch  4:Worktree  q:Quit",
        app.active_view.label()
    ))
    .style(Style::default().fg(Color::White).bg(Color::DarkGray));
    frame.render_widget(status, chunks[1]);
}
