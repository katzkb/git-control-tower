//! Central color palette and shared status→color mappings.
//!
//! UI code refers to these semantic names instead of raw `ratatui`
//! colors, so restyling the app means editing this file only.

use ratatui::style::Color;

use crate::git::types::ReviewStatus;

/// Highlight color: titles, selection, key hints, worktree markers.
pub const ACCENT: Color = Color::Cyan;
/// Primary foreground text.
pub const TEXT: Color = Color::White;
/// De-emphasized text: labels, separators, hints, disabled items.
pub const TEXT_DIM: Color = Color::DarkGray;
/// Positive outcomes: success toasts, merged branches, additions.
pub const SUCCESS: Color = Color::Green;
/// Caution: unmerged deletes, selections pending action, section headers.
pub const WARNING: Color = Color::Yellow;
/// Failures and destructive states: error toasts, deletions, needs-review.
pub const ERROR: Color = Color::Red;
/// Merged pull requests.
pub const PR_MERGED: Color = Color::Magenta;
/// Background for the sidebar scroll indicator bar.
pub const BAR_BG: Color = Color::Black;

/// Display label (capitalized) and color for a PR review status.
/// Callers that want a lowercase tag (e.g. the sidebar's `[approved]`)
/// can `to_ascii_lowercase()` the label.
pub fn review_status(status: &ReviewStatus) -> (&'static str, Color) {
    match status {
        ReviewStatus::NeedsReview => ("Needs review", ERROR),
        ReviewStatus::Approved => ("Approved", SUCCESS),
        ReviewStatus::ChangesRequested => ("Changes requested", WARNING),
        ReviewStatus::Commented => ("Commented", ACCENT),
    }
}

/// Color for a PR state string as reported by `gh` (`OPEN`/`CLOSED`/`MERGED`).
pub fn pr_state_color(state: &str) -> Color {
    match state {
        "OPEN" => SUCCESS,
        "CLOSED" => ERROR,
        "MERGED" => PR_MERGED,
        _ => TEXT,
    }
}
