use std::time::Duration;

use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState},
};

use crate::app::App;
use crate::git::command::{CommandRecord, command_history_snapshot, session_elapsed_at};

pub fn draw(frame: &mut Frame, area: Rect, app: &App) {
    let block = Block::default().borders(Borders::ALL).title(" History ");

    let records = command_history_snapshot();

    if records.is_empty() {
        let placeholder = List::new(vec![ListItem::new(Span::styled(
            "No commands recorded yet.",
            Style::default().fg(Color::DarkGray),
        ))])
        .block(block);
        frame.render_widget(placeholder, area);
        return;
    }

    let items: Vec<ListItem> = records.iter().map(format_record).collect();

    let list = List::new(items)
        .block(block)
        .highlight_style(Style::default().add_modifier(Modifier::REVERSED));

    let mut state = ListState::default();
    if app.history_scroll < records.len() {
        state.select(Some(app.history_scroll));
    } else if !records.is_empty() {
        state.select(Some(records.len() - 1));
    }

    frame.render_stateful_widget(list, area, &mut state);
}

fn format_record(record: &CommandRecord) -> ListItem<'static> {
    let offset_style = Style::default().fg(Color::DarkGray);
    let exec_style = Style::default().fg(Color::Cyan);
    let status_style = if record.success {
        Style::default().fg(Color::Green)
    } else {
        Style::default().fg(Color::Red)
    };
    let meta_style = Style::default().fg(Color::DarkGray);

    let status = if record.success { "OK " } else { "ERR" };
    let duration = format_duration(record.duration);
    let size = format_bytes(record.output_bytes);
    let offset = format!(
        "+{}",
        format_duration(session_elapsed_at(record.started_at))
    );

    let mut spans = vec![
        Span::styled(format!("{offset:>8}"), offset_style),
        Span::raw("  "),
        Span::styled(format!("{} ", record.executable), exec_style),
        Span::raw(record.args.join(" ")),
        Span::raw("  "),
        Span::styled(status.to_string(), status_style),
        Span::styled(format!("  {size:>7}  {duration:>6}"), meta_style),
    ];

    if let Some(err) = &record.error {
        spans.push(Span::raw("  "));
        spans.push(Span::styled(
            err.lines().next().unwrap_or("").to_string(),
            Style::default().fg(Color::Red),
        ));
    }

    ListItem::new(Line::from(spans))
}

fn format_duration(d: Duration) -> String {
    let ms = d.as_millis();
    if ms < 1000 {
        format!("{ms}ms")
    } else {
        format!("{:.2}s", d.as_secs_f64())
    }
}

fn format_bytes(n: usize) -> String {
    if n < 1024 {
        format!("{n}B")
    } else if n < 1024 * 1024 {
        format!("{:.1}KB", n as f64 / 1024.0)
    } else {
        format!("{:.1}MB", n as f64 / (1024.0 * 1024.0))
    }
}
