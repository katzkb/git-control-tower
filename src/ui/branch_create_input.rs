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
    // minus the prefix. `window_around_cursor` / `truncate_head` each treat their
    // `max` as the full budget (content + cursor + ellipses), so no extra reservation.
    let inner = area.width.saturating_sub(2) as usize;
    let from_prefix = "  From: ".len();
    let name_prefix = "  Name: ".len();
    let from_max = inner.saturating_sub(from_prefix);
    let name_max = inner.saturating_sub(name_prefix);

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
/// `max` is the total budget for the rendered cell, covering the 1-column cursor
/// indicator, any visible content, and up to two `…` markers. The returned
/// `before` + cursor + `after` + ellipses together span at most `max` columns.
fn window_around_cursor(s: &str, cursor: usize, max: usize) -> CursorWindow {
    let empty = CursorWindow {
        before: String::new(),
        after: String::new(),
        left_ellipsis: false,
        right_ellipsis: false,
    };
    let chars: Vec<char> = s.chars().collect();
    let len = chars.len();
    let cursor = cursor.min(len);

    if max == 0 || len == 0 {
        return empty;
    }

    // 1 col goes to the cursor indicator; the rest is for content plus ellipses.
    let visible = max - 1;
    if visible == 0 {
        // Only room for the cursor indicator itself — hide content and ellipses.
        return empty;
    }

    // Fast path: the whole string fits alongside the cursor; no ellipses needed.
    if len <= visible {
        return CursorWindow {
            before: chars[..cursor].iter().collect(),
            after: chars[cursor..].iter().collect(),
            left_ellipsis: false,
            right_ellipsis: false,
        };
    }

    // String is wider than the visible area — at least one ellipsis is required.
    // A one-sided clip uses a content window of size `visible - 1`.
    let one_ell = visible.saturating_sub(1);

    // Cursor close to the start: clip only on the right.
    if cursor <= one_ell {
        let end = one_ell;
        return CursorWindow {
            before: chars[..cursor].iter().collect(),
            after: chars[cursor..end].iter().collect(),
            left_ellipsis: false,
            right_ellipsis: true,
        };
    }

    // Cursor close to the end: clip only on the left.
    if cursor >= len - one_ell {
        let start = len - one_ell;
        return CursorWindow {
            before: chars[start..cursor].iter().collect(),
            after: chars[cursor..].iter().collect(),
            left_ellipsis: true,
            right_ellipsis: false,
        };
    }

    // Cursor in the middle: clip on both sides. Budget shrinks by 2 for ellipses.
    if visible < 2 {
        return empty;
    }
    let window_size = visible - 2;
    let half = window_size / 2;
    let start = cursor - half;
    let end = start + window_size;
    CursorWindow {
        before: chars[start..cursor].iter().collect(),
        after: chars[cursor..end].iter().collect(),
        left_ellipsis: true,
        right_ellipsis: true,
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
        assert_eq!(
            window_around_cursor("abcdefghij", 1, 5),
            w("a", "bc", false, true)
        );
    }

    #[test]
    fn window_clipped_left_when_cursor_near_end() {
        assert_eq!(
            window_around_cursor("abcdefghij", 10, 5),
            w("hij", "", true, false)
        );
    }

    #[test]
    fn window_clipped_both_when_cursor_middle() {
        assert_eq!(
            window_around_cursor("abcdefghij", 5, 5),
            w("e", "f", true, true)
        );
    }

    #[test]
    fn window_unicode() {
        assert_eq!(
            window_around_cursor("αβγδε", 2, 4),
            w("αβ", "", false, true)
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

    /// Rendered cell width must never exceed `max` regardless of cursor position.
    #[test]
    fn window_never_exceeds_max() {
        let s = "abcdefghij";
        for cursor in 0..=s.chars().count() {
            for max in 0..=12 {
                let out = window_around_cursor(s, cursor, max);
                let width = out.before.chars().count()
                    + out.after.chars().count()
                    + 1 // cursor indicator
                    + usize::from(out.left_ellipsis)
                    + usize::from(out.right_ellipsis);
                assert!(
                    width <= max.max(1),
                    "cursor={cursor} max={max} width={width} out={out:?}"
                );
            }
        }
    }
}
