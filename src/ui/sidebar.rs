use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState},
};

use crate::app::{App, MainFilter};
use crate::git::types::BranchEntry;

use crate::ui::theme;

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
                    .fg(theme::ACCENT)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(app.search_query.clone(), Style::default().fg(theme::TEXT)),
            Span::styled("_", Style::default().fg(theme::ACCENT)),
        ])
    } else {
        let label = app.main_filter.label();
        let mut spans = vec![
            Span::styled(" Filter: ", Style::default().fg(theme::TEXT_DIM)),
            Span::styled(
                label,
                Style::default()
                    .fg(theme::ACCENT)
                    .add_modifier(Modifier::BOLD),
            ),
        ];
        // Show merged toggle indicator for My PR / Review views
        if matches!(
            app.main_filter,
            MainFilter::MyPr | MainFilter::ReviewRequested
        ) && app.prs.show_merged
        {
            spans.push(Span::styled(
                " [+merged]",
                Style::default().fg(theme::WARNING),
            ));
        }
        // Show active search filter
        if !app.search_query.is_empty() {
            spans.push(Span::styled(
                format!(" /{}", app.search_query),
                Style::default().fg(theme::ACCENT),
            ));
        }
        // Show team toggle indicator for Review view
        if app.main_filter == MainFilter::ReviewRequested {
            let team_label = if app.prs.include_team_reviews {
                " [+team]"
            } else {
                " [me]"
            };
            spans.push(Span::styled(team_label, Style::default().fg(theme::ACCENT)));
        }
        // Show repo/PR counter when multiple repos are present
        if matches!(
            app.main_filter,
            MainFilter::MyPr | MainFilter::ReviewRequested
        ) {
            let filtered = app.filtered_entries();
            let repo_count = filtered
                .iter()
                .map(|e| e.repo_id.clone())
                .collect::<std::collections::HashSet<_>>()
                .len();
            if repo_count > 1 {
                let pr_count = filtered.iter().filter(|e| e.pull_request.is_some()).count();
                spans.push(Span::styled(
                    format!("  {repo_count} repos · {pr_count} PRs"),
                    Style::default().fg(theme::TEXT_DIM),
                ));
            }
        }
        Line::from(spans)
    };
    frame.render_widget(
        ratatui::widgets::Paragraph::new(bar).style(Style::default().bg(theme::BAR_BG)),
        area,
    );
}

fn draw_entry_list(frame: &mut Frame, area: Rect, app: &mut App) {
    let show_checkboxes = !app.branch_selected.is_empty();
    let is_loading = app.is_current_view_loading();

    // Build items and capture count before mutably borrowing app
    let (items, item_count): (Vec<ListItem>, usize) = {
        let rows = app.sidebar_rows();
        let item_count = rows.len();
        let items = if rows.is_empty() && is_loading {
            let spinner = app.spinner_frame();
            vec![ListItem::new(Line::from(Span::styled(
                format!("  {spinner} Loading"),
                Style::default().fg(theme::TEXT_DIM),
            )))]
        } else if rows.is_empty() {
            let msg = if !app.search_query.is_empty() {
                "  No branches match the search"
            } else {
                match app.main_filter {
                    MainFilter::Local => "  No local branches",
                    MainFilter::MyPr => "  No branches with your PRs",
                    MainFilter::ReviewRequested => "  No branches awaiting review",
                }
            };
            vec![ListItem::new(Line::from(Span::styled(
                msg,
                Style::default().fg(theme::TEXT_DIM),
            )))]
        } else {
            let search_query = &app.search_query;
            rows.iter()
                .map(|row| match row {
                    crate::app::SidebarRow::Header { repo_id } => {
                        ListItem::new(Line::from(vec![Span::styled(
                            format!(" ▾ {repo_id} "),
                            Style::default()
                                .fg(theme::ACCENT)
                                .add_modifier(Modifier::DIM | Modifier::BOLD),
                        )]))
                    }
                    crate::app::SidebarRow::Entry(entry) => {
                        let is_selected = app.branch_selected.contains(&entry.name);
                        let is_protected = app.is_protected_branch(&entry.name);
                        ListItem::new(format_entry_line(
                            entry,
                            show_checkboxes,
                            is_selected,
                            is_protected,
                            search_query,
                        ))
                    }
                })
                .collect()
        };
        (items, item_count)
    };

    // Adjust viewport offset and clamp scroll/offset to valid ranges
    let visible_height = area.height.saturating_sub(2) as usize;
    app.adjust_sidebar_offset(visible_height, item_count);

    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Branches ")
        .border_style(Style::default().fg(theme::TEXT_DIM));

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
    is_protected: bool,
    search_query: &str,
) -> Line<'static> {
    let mut spans: Vec<Span> = Vec::new();

    // Checkbox (only shown when multi-select is active)
    if show_checkboxes {
        if is_selected {
            spans.push(Span::styled("[x] ", Style::default().fg(theme::ACCENT)));
        } else {
            spans.push(Span::styled("[ ] ", Style::default().fg(theme::TEXT_DIM)));
        }
    }

    // Branch name (with search highlight)
    let name_style = if entry.is_current() {
        Style::default()
            .fg(theme::SUCCESS)
            .add_modifier(Modifier::BOLD)
    } else if (entry.is_merged() || entry.pr_is_merged()) && !is_protected {
        Style::default().fg(theme::WARNING)
    } else {
        Style::default().fg(theme::TEXT)
    };
    spans.extend(format_branch_name(&entry.name, search_query, name_style));

    // Current marker
    if entry.is_current() {
        spans.push(Span::styled(
            " *",
            Style::default()
                .fg(theme::SUCCESS)
                .add_modifier(Modifier::BOLD),
        ));
    }

    // PR number
    if let Some(pr) = &entry.pull_request {
        spans.push(Span::styled(
            format!(" #{}", pr.number),
            Style::default().fg(theme::WARNING),
        ));
    }

    // Draft tag
    if let Some(pr) = &entry.pull_request
        && pr.is_draft
    {
        spans.push(Span::styled(
            " [draft]",
            Style::default().fg(theme::TEXT_DIM),
        ));
    }

    // Review status tag
    if let Some(pr) = &entry.pull_request
        && let Some(status) = &pr.review_status
    {
        let (label, color) = theme::review_status(status);
        spans.push(Span::styled(
            format!(" [{}]", label.to_ascii_lowercase()),
            Style::default().fg(color),
        ));
    }

    // Worktree indicator
    if entry.worktree.is_some() && !entry.is_current() {
        spans.push(Span::styled(" wt", Style::default().fg(theme::ACCENT)));
    }

    // Merged tag
    if (entry.is_merged() || entry.pr_is_merged()) && !entry.is_current() && !is_protected {
        spans.push(Span::styled(
            " [merged]",
            Style::default().fg(theme::WARNING),
        ));
    }

    Line::from(spans)
}

fn format_branch_name(name: &str, search_query: &str, base_style: Style) -> Vec<Span<'static>> {
    if search_query.is_empty() {
        return vec![Span::styled(format!(" {name}"), base_style)];
    }
    // Match on Vec<char> throughout so slicing is always char-boundary safe,
    // even when a char's lowercase form changes byte length (e.g. Turkish İ).
    let name_chars: Vec<char> = name.chars().collect();
    let lower_name_chars: Vec<char> = name_chars
        .iter()
        .map(|c| c.to_lowercase().next().unwrap_or(*c))
        .collect();
    let lower_query_chars: Vec<char> = search_query.to_lowercase().chars().collect();
    if let Some(start) = lower_name_chars
        .windows(lower_query_chars.len())
        .position(|w| w == lower_query_chars.as_slice())
    {
        let end = start + lower_query_chars.len();
        let highlight_style = base_style
            .add_modifier(Modifier::UNDERLINED)
            .fg(theme::ACCENT);
        vec![
            Span::styled(
                format!(" {}", name_chars[..start].iter().collect::<String>()),
                base_style,
            ),
            Span::styled(
                name_chars[start..end].iter().collect::<String>(),
                highlight_style,
            ),
            Span::styled(name_chars[end..].iter().collect::<String>(), base_style),
        ]
    } else {
        vec![Span::styled(format!(" {name}"), base_style)]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain_style() -> Style {
        Style::default()
    }

    fn spans_to_string(spans: &[Span<'static>]) -> String {
        spans.iter().map(|s| s.content.as_ref()).collect()
    }

    #[test]
    fn no_query_returns_plain_name() {
        let spans = format_branch_name("feature/login", "", plain_style());
        assert_eq!(spans_to_string(&spans), " feature/login");
    }

    #[test]
    fn ascii_match_highlights_substring() {
        let spans = format_branch_name("feature/login", "log", plain_style());
        assert_eq!(spans.len(), 3);
        assert_eq!(spans_to_string(&spans), " feature/login");
        assert_eq!(spans[1].content.as_ref(), "log");
    }

    #[test]
    fn case_insensitive_match() {
        let spans = format_branch_name("Feature/LOGIN", "log", plain_style());
        assert_eq!(spans.len(), 3);
        assert_eq!(spans[1].content.as_ref(), "LOG");
    }

    #[test]
    fn no_match_returns_plain_name() {
        let spans = format_branch_name("feature/login", "zzz", plain_style());
        assert_eq!(spans.len(), 1);
        assert_eq!(spans_to_string(&spans), " feature/login");
    }

    #[test]
    fn multibyte_lowercase_expansion_does_not_panic() {
        // 'İ' (U+0130) lowercases to two chars ("i" + combining dot above),
        // which previously caused byte-offset/char-boundary mismatches.
        let spans = format_branch_name("featureİ/login", "login", plain_style());
        assert_eq!(spans.len(), 3);
        assert_eq!(spans_to_string(&spans), " featureİ/login");
        assert_eq!(spans[1].content.as_ref(), "login");
    }

    #[test]
    fn other_multibyte_branch_names_do_not_panic() {
        let spans = format_branch_name("feature/ログイン-test", "test", plain_style());
        assert_eq!(spans.len(), 3);
        assert_eq!(spans_to_string(&spans), " feature/ログイン-test");
        assert_eq!(spans[1].content.as_ref(), "test");

        let spans = format_branch_name("αβγδε", "γδ", plain_style());
        assert_eq!(spans.len(), 3);
        assert_eq!(spans_to_string(&spans), " αβγδε");
        assert_eq!(spans[1].content.as_ref(), "γδ");
    }
}
