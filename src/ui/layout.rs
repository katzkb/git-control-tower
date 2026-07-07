//! Shared geometry helpers for overlay placement.

use ratatui::layout::{Constraint, Flex, Layout, Rect};

/// A rect centered both ways: `percent_x` of the width, fixed `height`.
pub fn centered_rect(percent_x: u16, height: u16, area: Rect) -> Rect {
    let vertical = Layout::vertical([Constraint::Length(height)])
        .flex(Flex::Center)
        .split(area);
    let horizontal = Layout::horizontal([Constraint::Percentage(percent_x)])
        .flex(Flex::Center)
        .split(vertical[0]);
    horizontal[0]
}

/// A rect pinned to the bottom edge: `percent_x` of the width (centered
/// horizontally), fixed `height`.
pub fn bottom_rect(percent_x: u16, height: u16, area: Rect) -> Rect {
    let vertical = Layout::vertical([Constraint::Min(0), Constraint::Length(height)]).split(area);
    let horizontal = Layout::horizontal([Constraint::Percentage(percent_x)])
        .flex(Flex::Center)
        .split(vertical[1]);
    horizontal[0]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn centered_rect_is_centered_with_requested_size() {
        let area = Rect::new(0, 0, 100, 40);
        let r = centered_rect(50, 10, area);
        assert_eq!((r.width, r.height), (50, 10));
        assert_eq!(r.x, 25);
        assert_eq!(r.y, 15);
    }

    #[test]
    fn bottom_rect_is_pinned_to_bottom_edge() {
        let area = Rect::new(0, 0, 100, 40);
        let r = bottom_rect(80, 3, area);
        assert_eq!((r.width, r.height), (80, 3));
        assert_eq!(r.x, 10);
        assert_eq!(r.y + r.height, area.height);
    }
}
