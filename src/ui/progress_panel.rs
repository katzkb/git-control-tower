use std::time::Instant;

use ratatui::{
    Frame,
    layout::{Constraint, Flex, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
};

use crate::app::{OpProgress, OpStep, ProgressTracker};

const MAX_VISIBLE_OPS: usize = 7;

pub fn draw(frame: &mut Frame, tracker: &ProgressTracker, quit_warning: bool) {
    let lines = build_lines(tracker, quit_warning, Instant::now());
    let height = (lines.len() as u16) + 2; // borders
    let area = bottom_rect(80, height, frame.area());
    frame.render_widget(Clear, area);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));

    let paragraph = Paragraph::new(lines).block(block);
    frame.render_widget(paragraph, area);
}

fn build_lines(tracker: &ProgressTracker, quit_warning: bool, now: Instant) -> Vec<Line<'static>> {
    let mut lines: Vec<Line<'static>> = Vec::new();

    if quit_warning {
        lines.push(Line::from(Span::styled(
            "Delete in progress. Press q again to quit anyway.".to_string(),
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )));
    }

    let total = tracker.total();
    let done = tracker.done_count();
    let elapsed = tracker
        .started_at
        .map(|s| now.saturating_duration_since(s).as_secs_f32())
        .unwrap_or(0.0);
    lines.push(Line::from(Span::styled(
        format!("Deleting ({done}/{total} done · {elapsed:.1}s)"),
        Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD),
    )));

    let mut ops_iter = tracker.ops.values();
    for _ in 0..MAX_VISIBLE_OPS.min(total) {
        if let Some(op) = ops_iter.next() {
            lines.push(format_op_line(op, now));
        }
    }
    let remaining = total.saturating_sub(MAX_VISIBLE_OPS);
    if remaining > 0 {
        lines.push(Line::from(Span::styled(
            format!("  +{remaining} more"),
            Style::default().fg(Color::DarkGray),
        )));
    }

    lines
}

fn format_op_line(op: &OpProgress, now: Instant) -> Line<'static> {
    let icon = match op.current_step {
        OpStep::Done { success: true } => "✓",
        OpStep::Done { success: false } => "✗",
        _ => "⏳",
    };
    let icon_style = match op.current_step {
        OpStep::Done { success: true } => Style::default().fg(Color::Green),
        OpStep::Done { success: false } => Style::default().fg(Color::Red),
        _ => Style::default().fg(Color::Yellow),
    };
    let label = format!(" {:<14}", trunc(&op.label, 14));
    let cmd = op
        .last_command
        .clone()
        .or_else(|| op.error.clone())
        .unwrap_or_else(|| "starting…".to_string());
    let elapsed = now
        .saturating_duration_since(op.op_started_at)
        .as_secs_f32();
    let row_style = if matches!(op.current_step, OpStep::Done { success: true }) {
        Style::default().fg(Color::DarkGray)
    } else if matches!(op.current_step, OpStep::Done { success: false }) {
        Style::default().fg(Color::Red)
    } else {
        Style::default().fg(Color::White)
    };

    Line::from(vec![
        Span::styled(format!("  {icon}"), icon_style),
        Span::styled(label, row_style),
        Span::styled(format!(" {cmd}"), row_style),
        Span::styled(
            format!("  {elapsed:.1}s"),
            Style::default().fg(Color::DarkGray),
        ),
    ])
}

// Counts unicode scalar values (chars), not display columns. CJK / emoji
// labels render wider than their char count, so the column may overflow
// for non-ASCII branch names. Acceptable for current usage; revisit with
// `unicode-width` if internationalized labels become common.
fn trunc(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(n.saturating_sub(1)).collect();
        out.push('…');
        out
    }
}

fn bottom_rect(percent_x: u16, height: u16, area: Rect) -> Rect {
    let vertical = Layout::vertical([Constraint::Min(0), Constraint::Length(height)]).split(area);
    let horizontal = Layout::horizontal([Constraint::Percentage(percent_x)])
        .flex(Flex::Center)
        .split(vertical[1]);
    horizontal[0]
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn fixed_now() -> Instant {
        // Anchor "now" for deterministic elapsed values.
        Instant::now()
    }

    #[test]
    fn build_lines_includes_header_when_active() {
        let mut t = ProgressTracker::default();
        let id = t.allocate_ids(1).start;
        t.insert(id, OpProgress::new("feat-a".into()));
        let now = fixed_now();
        let lines = build_lines(&t, false, now);
        assert!(lines.len() >= 2);
        let header_text: String = lines[0].spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(header_text.contains("Deleting"));
        assert!(header_text.contains("0/1"));
    }

    #[test]
    fn build_lines_overflow_shows_more_marker() {
        let mut t = ProgressTracker::default();
        let ids: Vec<u64> = t.allocate_ids(MAX_VISIBLE_OPS + 3).collect();
        for (i, id) in ids.iter().enumerate() {
            t.insert(*id, OpProgress::new(format!("op{i}")));
        }
        let lines = build_lines(&t, false, fixed_now());
        let last: String = lines
            .last()
            .unwrap()
            .spans
            .iter()
            .map(|s| s.content.as_ref())
            .collect();
        assert!(
            last.contains("+3 more"),
            "expected overflow marker, got: {last}"
        );
    }

    #[test]
    fn build_lines_quit_warning_prepended() {
        let mut t = ProgressTracker::default();
        let id = t.allocate_ids(1).start;
        t.insert(id, OpProgress::new("a".into()));
        let lines = build_lines(&t, true, fixed_now());
        let first: String = lines[0].spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(first.contains("Delete in progress"));
        assert!(first.contains("Press q again"));
    }

    #[test]
    fn format_op_line_done_success_uses_check_icon() {
        let mut op = OpProgress::new("a".into());
        op.current_step = OpStep::Done { success: true };
        op.last_command = Some("git worktree remove /wt/a".into());
        let line = format_op_line(&op, Instant::now() + Duration::from_secs(1));
        let txt: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(txt.contains("✓"));
        assert!(txt.contains("git worktree remove /wt/a"));
    }

    #[test]
    fn format_op_line_done_failure_uses_cross_icon() {
        let mut op = OpProgress::new("a".into());
        op.current_step = OpStep::Done { success: false };
        op.error = Some("perm denied".into());
        let line = format_op_line(&op, Instant::now());
        let txt: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(txt.contains("✗"));
        assert!(txt.contains("perm denied"));
    }
}
