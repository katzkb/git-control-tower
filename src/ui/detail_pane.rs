use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
};

use crate::app::App;
use crate::git::types::{BranchEntry, PrDetail};
use crate::ui::markdown;

pub fn draw(frame: &mut Frame, area: Rect, app: &App) {
    let entry = match app.selected_entry() {
        Some(e) => e,
        None => {
            let block = Block::default()
                .borders(Borders::ALL)
                .title(" Detail ")
                .border_style(Style::default().fg(Color::DarkGray));
            let p = Paragraph::new("No branch selected")
                .style(Style::default().fg(Color::DarkGray))
                .block(block);
            frame.render_widget(p, area);
            return;
        }
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .title(format!(" {} ", entry.name))
        .border_style(Style::default().fg(Color::DarkGray));

    let mut lines: Vec<Line> = Vec::new();

    // Git Status section
    draw_git_status_section(&mut lines, entry);

    // Worktree section
    draw_worktree_section(&mut lines, entry);

    // PR section
    draw_pr_section(&mut lines, entry, app.selected_pr_detail());

    if lines.is_empty() {
        lines.push(Line::from(Span::styled(
            " No additional information",
            Style::default().fg(Color::DarkGray),
        )));
    }

    let paragraph = Paragraph::new(lines)
        .block(block)
        .wrap(Wrap { trim: false })
        .scroll((app.pr_detail_scroll as u16, 0));
    frame.render_widget(paragraph, area);
}

fn section_header(title: &str) -> Line<'static> {
    Line::from(Span::styled(
        format!("── {title} ──────────────────────"),
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    ))
}

fn draw_git_status_section(lines: &mut Vec<Line<'static>>, entry: &BranchEntry) {
    lines.push(section_header("Git Status"));

    if entry.worktree.is_none() {
        lines.push(Line::from(Span::styled(
            "  —",
            Style::default().fg(Color::DarkGray),
        )));
        lines.push(Line::from(""));
        return;
    }

    let status = match &entry.git_status {
        Some(s) => s,
        None => {
            lines.push(Line::from(Span::styled(
                "  Loading...",
                Style::default().fg(Color::DarkGray),
            )));
            lines.push(Line::from(""));
            return;
        }
    };

    // Staged changes
    for file in &status.staged {
        lines.push(Line::from(Span::styled(
            format!("  {file}"),
            Style::default().fg(Color::Green),
        )));
    }

    // Unstaged changes
    for file in &status.unstaged {
        lines.push(Line::from(Span::styled(
            format!("  {file}"),
            Style::default().fg(Color::Red),
        )));
    }

    // Untracked files
    for file in &status.untracked {
        lines.push(Line::from(Span::styled(
            format!("  ?? {file}"),
            Style::default().fg(Color::DarkGray),
        )));
    }

    // Ahead/behind
    if status.ahead > 0 || status.behind > 0 {
        let mut ab_spans = vec![Span::raw("  ")];
        if status.ahead > 0 {
            ab_spans.push(Span::styled(
                format!("↑{}", status.ahead),
                Style::default().fg(Color::Green),
            ));
            ab_spans.push(Span::raw(" "));
        }
        if status.behind > 0 {
            ab_spans.push(Span::styled(
                format!("↓{}", status.behind),
                Style::default().fg(Color::Red),
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
            Style::default().fg(Color::DarkGray),
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
                Style::default().fg(Color::White),
            )));
        }
        None => {
            lines.push(Line::from(Span::styled(
                "  —",
                Style::default().fg(Color::DarkGray),
            )));
        }
    }
    lines.push(Line::from(""));
}

fn draw_pr_section(
    lines: &mut Vec<Line<'static>>,
    entry: &BranchEntry,
    pr_detail: Option<&PrDetail>,
) {
    let pr = match &entry.pull_request {
        Some(p) => p,
        None => {
            lines.push(section_header("PR"));
            lines.push(Line::from(Span::styled(
                "  —",
                Style::default().fg(Color::DarkGray),
            )));
            lines.push(Line::from(""));
            return;
        }
    };

    lines.push(section_header(&format!("PR #{}", pr.number)));

    // Title
    lines.push(Line::from(Span::styled(
        format!("  {}", pr.title),
        Style::default().add_modifier(Modifier::BOLD),
    )));

    // Author + State
    let state_color = match pr.state.as_str() {
        "OPEN" => Color::Green,
        "CLOSED" => Color::Red,
        "MERGED" => Color::Magenta,
        _ => Color::White,
    };
    lines.push(Line::from(vec![
        Span::styled("  Author: ", Style::default().fg(Color::DarkGray)),
        Span::styled(pr.author.clone(), Style::default().fg(Color::White)),
        Span::styled("  State: ", Style::default().fg(Color::DarkGray)),
        Span::styled(pr.state.clone(), Style::default().fg(state_color)),
    ]));

    // PR detail body (if loaded and matches)
    if let Some(detail) = pr_detail {
        if detail.number == pr.number {
            // Additions / Deletions
            lines.push(Line::from(vec![
                Span::styled(
                    format!("  +{}", detail.additions),
                    Style::default().fg(Color::Green),
                ),
                Span::styled(" / ", Style::default().fg(Color::DarkGray)),
                Span::styled(
                    format!("-{}", detail.deletions),
                    Style::default().fg(Color::Red),
                ),
            ]));

            lines.push(Line::from(""));

            // Markdown body
            if detail.body.trim().is_empty() {
                lines.push(Line::from(Span::styled(
                    "  (no description)",
                    Style::default().fg(Color::DarkGray),
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
            "  Loading...",
            Style::default().fg(Color::DarkGray),
        )));
    }

    lines.push(Line::from(""));
}
