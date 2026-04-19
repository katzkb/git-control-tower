use ratatui::{
    Frame,
    layout::{Constraint, Flex, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
};

use crate::app::BranchCreateInput;

pub fn draw(frame: &mut Frame, input: &BranchCreateInput) {
    let area = centered_rect(60, 8, frame.area());
    frame.render_widget(Clear, area);

    // Available width for the value portion of each row: modal inner width
    // minus the "  Name: " prefix (8) minus 1 column for the cursor on Name.
    let inner = area.width.saturating_sub(2) as usize;
    let from_prefix = "  From: ".len();
    let name_prefix = "  Name: ".len();
    let from_max = inner.saturating_sub(from_prefix);
    let name_max = inner.saturating_sub(name_prefix + 1);

    let source_display = truncate_head(&input.source, from_max);
    let window = window_around_cursor(&input.name, input.cursor, name_max);

    let white = Style::default().fg(Color::White);
    let gray = Style::default().fg(Color::DarkGray);
    let cyan = Style::default().fg(Color::Cyan);

    let mut name_spans: Vec<Span> = Vec::with_capacity(6);
    name_spans.push(Span::styled("  Name: ", gray));
    if window.left_ellipsis {
        name_spans.push(Span::styled("…", gray));
    }
    name_spans.push(Span::styled(window.before, white));
    name_spans.push(Span::styled("_", cyan));
    name_spans.push(Span::styled(window.after, white));
    if window.right_ellipsis {
        name_spans.push(Span::styled("…", gray));
    }

    let lines = vec![
        Line::from(""),
        Line::from(vec![
            Span::styled("  From: ", gray),
            Span::styled(source_display, white.add_modifier(Modifier::BOLD)),
        ]),
        Line::from(name_spans),
        Line::from(""),
        Line::from(vec![
            Span::styled(
                " Enter ",
                Style::default()
                    .fg(Color::White)
                    .bg(Color::DarkGray)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" Create  "),
            Span::styled(
                " Esc ",
                Style::default()
                    .fg(Color::White)
                    .bg(Color::DarkGray)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" Cancel"),
        ]),
    ];

    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Create Branch ")
        .border_style(Style::default().fg(Color::Cyan));

    let paragraph = Paragraph::new(lines).block(block);
    frame.render_widget(paragraph, area);
}

fn truncate_head(s: &str, max: usize) -> String {
    if max == 0 {
        return String::new();
    }
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= max {
        return s.to_string();
    }
    // Reserve 1 char for the leading ellipsis.
    let start = chars.len() - (max - 1);
    let mut out = String::from("…");
    out.extend(&chars[start..]);
    out
}

#[derive(Debug, PartialEq, Eq)]
struct CursorWindow {
    before: String,
    after: String,
    left_ellipsis: bool,
    right_ellipsis: bool,
}

/// Compute the visible portion of `s` around `cursor` that fits within `max` columns.
///
/// The returned `before` + cursor (1 col) + `after` spans at most `max` columns.
/// `left_ellipsis` / `right_ellipsis` indicate whether a `…` marker should be drawn
/// on the clipped side (ellipsis columns are not included in `max`).
fn window_around_cursor(s: &str, cursor: usize, max: usize) -> CursorWindow {
    let chars: Vec<char> = s.chars().collect();
    let len = chars.len();
    let cursor = cursor.min(len);

    // Content budget excludes the 1 column reserved for the cursor itself.
    let content_budget = max.saturating_sub(1);
    if content_budget == 0 || len == 0 {
        return CursorWindow {
            before: String::new(),
            after: String::new(),
            left_ellipsis: false,
            right_ellipsis: false,
        };
    }

    if len <= content_budget {
        return CursorWindow {
            before: chars[..cursor].iter().collect(),
            after: chars[cursor..].iter().collect(),
            left_ellipsis: false,
            right_ellipsis: false,
        };
    }

    // Text doesn't fit. Pick a window [start, end) of size `content_budget`
    // that contains the cursor and keeps it roughly centered.
    let half = content_budget / 2;
    let mut start = cursor.saturating_sub(half);
    let mut end = start + content_budget;
    if end > len {
        end = len;
        start = end - content_budget;
    }

    let left_ellipsis = start > 0;
    let right_ellipsis = end < len;
    let before: String = chars[start..cursor].iter().collect();
    let after: String = chars[cursor..end].iter().collect();

    CursorWindow {
        before,
        after,
        left_ellipsis,
        right_ellipsis,
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_head_fits() {
        assert_eq!(truncate_head("main", 10), "main");
    }

    #[test]
    fn truncate_head_cuts() {
        assert_eq!(truncate_head("feat/abcdefghij", 6), "…fghij");
    }

    #[test]
    fn truncate_head_zero_max() {
        assert_eq!(truncate_head("anything", 0), "");
    }

    #[test]
    fn truncate_head_unicode() {
        assert_eq!(truncate_head("αβγδε", 4), "…γδε");
    }

    fn w(before: &str, after: &str, l: bool, r: bool) -> CursorWindow {
        CursorWindow {
            before: before.to_string(),
            after: after.to_string(),
            left_ellipsis: l,
            right_ellipsis: r,
        }
    }

    #[test]
    fn window_fits_no_ellipsis() {
        assert_eq!(
            window_around_cursor("abc", 1, 10),
            w("a", "bc", false, false)
        );
    }

    #[test]
    fn window_cursor_at_start() {
        assert_eq!(
            window_around_cursor("abc", 0, 10),
            w("", "abc", false, false)
        );
    }

    #[test]
    fn window_cursor_at_end() {
        assert_eq!(
            window_around_cursor("abc", 3, 10),
            w("abc", "", false, false)
        );
    }

    #[test]
    fn window_clipped_right_when_cursor_near_start() {
        // "abcdefghij", cursor=1, max=5 -> content budget 4
        // half=2, start=max(0,1-2)=0, end=0+4=4 -> "abcd" with right ellipsis
        assert_eq!(
            window_around_cursor("abcdefghij", 1, 5),
            w("a", "bcd", false, true)
        );
    }

    #[test]
    fn window_clipped_left_when_cursor_near_end() {
        // "abcdefghij", cursor=10, max=5 -> content budget 4
        // half=2, start=8, end=12 -> clamp end=10, start=6 -> "ghij" with left ellipsis
        assert_eq!(
            window_around_cursor("abcdefghij", 10, 5),
            w("ghij", "", true, false)
        );
    }

    #[test]
    fn window_clipped_both_when_cursor_middle() {
        // "abcdefghij", cursor=5, max=5 -> content budget 4
        // half=2, start=3, end=7 -> "de" + "fg", both ellipses
        assert_eq!(
            window_around_cursor("abcdefghij", 5, 5),
            w("de", "fg", true, true)
        );
    }

    #[test]
    fn window_unicode() {
        // 5 multi-byte chars, cursor middle, max=4 -> content budget 3
        // half=1, start=1, end=4 -> "β" + "γδ", both ellipses
        assert_eq!(
            window_around_cursor("αβγδε", 2, 4),
            w("β", "γδ", true, true)
        );
    }

    #[test]
    fn window_zero_max() {
        assert_eq!(window_around_cursor("abc", 1, 0), w("", "", false, false));
    }

    #[test]
    fn window_empty_string() {
        assert_eq!(window_around_cursor("", 0, 10), w("", "", false, false));
    }
}
