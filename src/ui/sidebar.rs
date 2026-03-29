use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState},
};

use crate::app::{App, MainFilter};
use crate::git::types::{BranchEntry, ReviewStatus};

pub fn draw(frame: &mut Frame, area: Rect, app: &mut App) {
    let chunks = Layout::vertical([Constraint::Length(1), Constraint::Min(1)]).split(area);

    draw_filter_bar(frame, chunks[0], app);
    draw_entry_list(frame, chunks[1], app);
}

fn draw_filter_bar(frame: &mut Frame, area: Rect, app: &App) {
    let bar = if app.search_active {
        Line::from(vec![
            Span::styled(
                " /",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(app.search_query.clone(), Style::default().fg(Color::White)),
            Span::styled("_", Style::default().fg(Color::Cyan)),
        ])
    } else {
        let label = app.main_filter.label();
        let mut spans = vec![
            Span::styled(" Filter: ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                label,
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
        ];
        // Show merged toggle indicator for My PR / Review views
        if matches!(
            app.main_filter,
            MainFilter::MyPr | MainFilter::ReviewRequested
        ) && app.show_merged
        {
            spans.push(Span::styled(
                " [+merged]",
                Style::default().fg(Color::Yellow),
            ));
        }
        // Show active search filter
        if !app.search_query.is_empty() {
            spans.push(Span::styled(
                format!(" /{}", app.search_query),
                Style::default().fg(Color::Cyan),
            ));
        }
        // Show team toggle indicator for Review view
        if app.main_filter == MainFilter::ReviewRequested {
            let team_label = if app.include_team_reviews {
                " [+team]"
            } else {
                " [me]"
            };
            spans.push(Span::styled(team_label, Style::default().fg(Color::Cyan)));
        }
        Line::from(spans)
    };
    frame.render_widget(
        ratatui::widgets::Paragraph::new(bar).style(Style::default().bg(Color::Black)),
        area,
    );
}

fn draw_entry_list(frame: &mut Frame, area: Rect, app: &mut App) {
    let show_checkboxes = !app.branch_selected.is_empty();
    let is_loading = app.is_current_view_loading();

    // Build items and capture count before mutably borrowing app
    let (items, item_count): (Vec<ListItem>, usize) = {
        let filtered = app.filtered_entries();
        let count = filtered.len();
        let items = if filtered.is_empty() && is_loading {
            let spinner = app.spinner_frame();
            vec![ListItem::new(Line::from(Span::styled(
                format!("  {spinner} Loading"),
                Style::default().fg(Color::DarkGray),
            )))]
        } else {
            let search_query = &app.search_query;
            filtered
                .iter()
                .map(|entry| {
                    let is_selected = app.branch_selected.contains(&entry.name);
                    ListItem::new(format_entry_line(
                        entry,
                        show_checkboxes,
                        is_selected,
                        search_query,
                    ))
                })
                .collect()
        };
        (items, count)
    };

    // Adjust viewport offset and clamp scroll/offset to valid ranges
    let visible_height = area.height.saturating_sub(2) as usize;
    app.adjust_sidebar_offset(visible_height, item_count);

    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Branches ")
        .border_style(Style::default().fg(Color::DarkGray));

    let list = List::new(items)
        .block(block)
        .highlight_style(Style::default().add_modifier(Modifier::REVERSED));

    let mut state = ListState::default();
    if item_count > 0 {
        state.select(Some(app.sidebar_scroll));
    }
    *state.offset_mut() = app.sidebar_offset;
    frame.render_stateful_widget(list, area, &mut state);
}

fn format_entry_line(
    entry: &BranchEntry,
    show_checkboxes: bool,
    is_selected: bool,
    search_query: &str,
) -> Line<'static> {
    let mut spans: Vec<Span> = Vec::new();

    // Checkbox (only shown when multi-select is active)
    if show_checkboxes {
        if is_selected {
            spans.push(Span::styled("[x] ", Style::default().fg(Color::Cyan)));
        } else {
            spans.push(Span::styled("[ ] ", Style::default().fg(Color::DarkGray)));
        }
    }

    // Branch name (with search highlight)
    let name_style = if entry.is_current() {
        Style::default()
            .fg(Color::Green)
            .add_modifier(Modifier::BOLD)
    } else if entry.is_merged() || entry.pr_is_merged() {
        Style::default().fg(Color::Yellow)
    } else {
        Style::default().fg(Color::White)
    };
    spans.extend(format_branch_name(&entry.name, search_query, name_style));

    // Current marker
    if entry.is_current() {
        spans.push(Span::styled(
            " *",
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        ));
    }

    // PR number
    if let Some(pr) = &entry.pull_request {
        spans.push(Span::styled(
            format!(" #{}", pr.number),
            Style::default().fg(Color::Yellow),
        ));
    }

    // Review status tag
    if let Some(pr) = &entry.pull_request
        && let Some(status) = &pr.review_status
    {
        let (label, color) = match status {
            ReviewStatus::NeedsReview => ("needs review", Color::Red),
            ReviewStatus::Approved => ("approved", Color::Green),
            ReviewStatus::ChangesRequested => ("changes requested", Color::Yellow),
            ReviewStatus::Commented => ("commented", Color::Cyan),
        };
        spans.push(Span::styled(
            format!(" [{label}]"),
            Style::default().fg(color),
        ));
    }

    // Worktree indicator
    if entry.worktree.is_some() && !entry.is_current() {
        spans.push(Span::styled(" wt", Style::default().fg(Color::Cyan)));
    }

    // Merged tag
    if (entry.is_merged() || entry.pr_is_merged()) && !entry.is_current() {
        spans.push(Span::styled(
            " [merged]",
            Style::default().fg(Color::Yellow),
        ));
    }

    Line::from(spans)
}

fn format_branch_name(name: &str, search_query: &str, base_style: Style) -> Vec<Span<'static>> {
    if search_query.is_empty() {
        return vec![Span::styled(format!(" {name}"), base_style)];
    }
    let lower_name = name.to_lowercase();
    let lower_query = search_query.to_lowercase();
    if let Some(start) = lower_name.find(&lower_query) {
        let end = start + lower_query.len();
        let highlight_style = base_style
            .add_modifier(Modifier::UNDERLINED)
            .fg(Color::Cyan);
        vec![
            Span::styled(format!(" {}", &name[..start]), base_style),
            Span::styled(name[start..end].to_string(), highlight_style),
            Span::styled(name[end..].to_string(), base_style),
        ]
    } else {
        vec![Span::styled(format!(" {name}"), base_style)]
    }
}
