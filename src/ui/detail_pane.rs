use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
};

use crate::app::{PaneFocus, PrCaches, ViewState};
use crate::git::types::{BranchEntry, PrDetail, RepoId};
use crate::ui::markdown;

use crate::ui::theme;

/// Read-only state the detail pane needs from outside `ViewState`.
pub struct DetailPaneContext<'a> {
    /// Current spinner animation frame for loading indicators.
    pub spinner: &'static str,
    /// PR caches, for the detail-body lookup of the selected entry.
    pub prs: &'a PrCaches,
    /// Active repo, to label cross-repo entries.
    pub active_repo: Option<&'a RepoId>,
    /// Errors to render at the bottom; empty unless verbose mode is on.
    pub verbose_errors: &'a [String],
}

pub fn draw(frame: &mut Frame, area: Rect, view: &mut ViewState, ctx: &DetailPaneContext<'_>) {
    // Build owned title + lines in a scoped block so the selected-entry
    // borrow of `view` ends before the scroll state is touched below.
    let (title, lines): (String, Vec<Line<'static>>) = {
        let entry = view.selected_entry();
        let title = match &entry {
            Some(e) => format!(" {} ", e.name),
            None => " Detail ".to_string(),
        };

        let mut lines: Vec<Line> = Vec::new();
        if let Some(entry) = &entry {
            draw_git_status_section(&mut lines, entry, ctx.spinner);
            draw_worktree_section(&mut lines, entry);
            draw_pr_section(
                &mut lines,
                entry,
                ctx.prs.detail_for(entry),
                ctx.spinner,
                ctx.active_repo,
            );
        } else {
            lines.push(Line::from(Span::styled(
                " No branch selected",
                Style::default().fg(theme::TEXT_DIM),
            )));
            lines.push(Line::from(""));
        }

        // Errors section — always drawn, even with no entry (the slice is
        // empty unless verbose mode is on).
        if !ctx.verbose_errors.is_empty() {
            draw_errors_section(&mut lines, ctx.verbose_errors);
        }

        if lines.is_empty() {
            lines.push(Line::from(Span::styled(
                " No additional information",
                Style::default().fg(theme::TEXT_DIM),
            )));
        }
        (title, lines)
    };

    // Highlight the border while the pane has key focus (issue #269).
    let border_color = if view.pane_focus == PaneFocus::Detail {
        theme::ACCENT
    } else {
        theme::TEXT_DIM
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .border_style(Style::default().fg(border_color));

    let paragraph = Paragraph::new(lines)
        .block(block)
        .wrap(Wrap { trim: false });

    // Clamp to the wrapped line count so the last page stays reachable —
    // the render is the only place viewport height and line count are known
    // (same pattern as adjust_sidebar_offset in the sidebar).
    let total_lines = paragraph.line_count(area.width.saturating_sub(2));
    let visible_height = area.height.saturating_sub(2) as usize;
    view.clamp_pr_detail_scroll(visible_height, total_lines);

    let paragraph = paragraph.scroll((view.pr_detail_scroll.min(u16::MAX as usize) as u16, 0));
    frame.render_widget(paragraph, area);
}

fn section_header(title: &str) -> Line<'static> {
    section_header_with_color(title, theme::ACCENT)
}

fn section_header_with_color(title: &str, color: Color) -> Line<'static> {
    Line::from(Span::styled(
        format!("── {title} ──────────────────────"),
        Style::default().fg(color).add_modifier(Modifier::BOLD),
    ))
}

fn draw_git_status_section(lines: &mut Vec<Line<'static>>, entry: &BranchEntry, spinner: &str) {
    lines.push(section_header("Git Status"));

    if entry.worktree.is_none() {
        lines.push(Line::from(Span::styled(
            "  —",
            Style::default().fg(theme::TEXT_DIM),
        )));
        lines.push(Line::from(""));
        return;
    }

    let status = match &entry.git_status {
        Some(s) => s,
        None => {
            lines.push(Line::from(Span::styled(
                format!("  {spinner} Loading"),
                Style::default().fg(theme::TEXT_DIM),
            )));
            lines.push(Line::from(""));
            return;
        }
    };

    // Staged changes
    for file in &status.staged {
        lines.push(Line::from(Span::styled(
            format!("  {file}"),
            Style::default().fg(theme::SUCCESS),
        )));
    }

    // Unstaged changes
    for file in &status.unstaged {
        lines.push(Line::from(Span::styled(
            format!("  {file}"),
            Style::default().fg(theme::ERROR),
        )));
    }

    // Untracked files
    for file in &status.untracked {
        lines.push(Line::from(Span::styled(
            format!("  ?? {file}"),
            Style::default().fg(theme::TEXT_DIM),
        )));
    }

    // Ahead/behind
    if status.ahead > 0 || status.behind > 0 {
        let mut ab_spans = vec![Span::raw("  ")];
        if status.ahead > 0 {
            ab_spans.push(Span::styled(
                format!("↑{}", status.ahead),
                Style::default().fg(theme::SUCCESS),
            ));
            ab_spans.push(Span::raw(" "));
        }
        if status.behind > 0 {
            ab_spans.push(Span::styled(
                format!("↓{}", status.behind),
                Style::default().fg(theme::ERROR),
            ));
        }
        lines.push(Line::from(ab_spans));
    }

    // Empty line if there were no changes but we still have the section
    if status.staged.is_empty()
        && status.unstaged.is_empty()
        && status.untracked.is_empty()
        && status.ahead == 0
        && status.behind == 0
    {
        lines.push(Line::from(Span::styled(
            "  (clean)",
            Style::default().fg(theme::TEXT_DIM),
        )));
    }

    lines.push(Line::from(""));
}

fn draw_worktree_section(lines: &mut Vec<Line<'static>>, entry: &BranchEntry) {
    lines.push(section_header("Worktree"));

    match &entry.worktree {
        Some(wt) => {
            lines.push(Line::from(Span::styled(
                format!("  {}", wt.path),
                Style::default().fg(theme::TEXT),
            )));
        }
        None => {
            lines.push(Line::from(Span::styled(
                "  —",
                Style::default().fg(theme::TEXT_DIM),
            )));
        }
    }
    lines.push(Line::from(""));
}

fn draw_pr_section(
    lines: &mut Vec<Line<'static>>,
    entry: &BranchEntry,
    pr_detail: Option<&PrDetail>,
    spinner: &str,
    active_repo: Option<&RepoId>,
) {
    let pr = match &entry.pull_request {
        Some(p) => p,
        None => {
            lines.push(section_header("PR"));
            lines.push(Line::from(Span::styled(
                "  —",
                Style::default().fg(theme::TEXT_DIM),
            )));
            lines.push(Line::from(""));
            return;
        }
    };

    lines.push(section_header(&format!("PR #{}", pr.number)));

    // Cross-repo: show repo identifier above title only when we know the active
    // repo and the entry belongs to a different one. When active_repo is None
    // (e.g. no `origin` remote, detached HEAD) we suppress the label — every
    // entry would otherwise show it, which is more confusing than helpful.
    if let Some(active) = active_repo
        && active != &entry.repo_id
    {
        lines.push(Line::from(vec![
            Span::styled("  repo: ", Style::default().fg(theme::TEXT_DIM)),
            Span::styled(
                entry.repo_id.to_string(),
                Style::default()
                    .fg(theme::ACCENT)
                    .add_modifier(Modifier::BOLD),
            ),
        ]));
    }

    // Title
    lines.push(Line::from(Span::styled(
        format!("  {}", pr.title),
        Style::default().add_modifier(Modifier::BOLD),
    )));

    // Author + State
    let (state_str, state_color) = if pr.is_draft {
        ("DRAFT", theme::TEXT_DIM)
    } else {
        theme::pr_state(pr.state)
    };
    lines.push(Line::from(vec![
        Span::styled("  Author: ", Style::default().fg(theme::TEXT_DIM)),
        Span::styled(pr.author.clone(), Style::default().fg(theme::TEXT)),
        Span::styled("  State: ", Style::default().fg(theme::TEXT_DIM)),
        Span::styled(state_str, Style::default().fg(state_color)),
    ]));

    // Review status
    if let Some(status) = &pr.review_status {
        let (label, color) = theme::review_status(status);
        lines.push(Line::from(vec![
            Span::styled("  Review: ", Style::default().fg(theme::TEXT_DIM)),
            Span::styled(label, Style::default().fg(color)),
        ]));
    }

    // PR detail body (if loaded and matches)
    if let Some(detail) = pr_detail {
        if detail.number == pr.number {
            // Additions / Deletions
            lines.push(Line::from(vec![
                Span::styled(
                    format!("  +{}", detail.additions),
                    Style::default().fg(theme::SUCCESS),
                ),
                Span::styled(" / ", Style::default().fg(theme::TEXT_DIM)),
                Span::styled(
                    format!("-{}", detail.deletions),
                    Style::default().fg(theme::ERROR),
                ),
            ]));

            lines.push(Line::from(""));

            // Markdown body
            if detail.body.trim().is_empty() {
                lines.push(Line::from(Span::styled(
                    "  (no description)",
                    Style::default().fg(theme::TEXT_DIM),
                )));
            } else {
                let md_lines = markdown::render_markdown(&detail.body);
                for line in md_lines {
                    lines.push(line);
                }
            }
        }
    } else {
        lines.push(Line::from(Span::styled(
            format!("  {spinner} Loading"),
            Style::default().fg(theme::TEXT_DIM),
        )));
    }

    lines.push(Line::from(""));
}

fn draw_errors_section(lines: &mut Vec<Line<'static>>, errors: &[String]) {
    lines.push(section_header_with_color("Errors", theme::ERROR));
    for err in errors {
        lines.push(Line::from(Span::styled(
            format!("  {err}"),
            Style::default().fg(theme::ERROR),
        )));
    }
    lines.push(Line::from(""));
}
