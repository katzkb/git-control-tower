use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};

/// Convert markdown text to styled ratatui Lines.
/// Supports: headings (#), bold (**), inline code (`), unordered lists (- ), code blocks (```).
pub fn render_markdown(text: &str) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    let mut in_code_block = false;

    for raw_line in text.lines() {
        if raw_line.starts_with("```") {
            in_code_block = !in_code_block;
            continue;
        }

        if in_code_block {
            lines.push(Line::from(Span::styled(
                format!("  {raw_line}"),
                Style::default().fg(Color::Green),
            )));
            continue;
        }

        if let Some(heading) = raw_line.strip_prefix("### ") {
            lines.push(Line::from(Span::styled(
                format!("   {heading}"),
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            )));
        } else if let Some(heading) = raw_line.strip_prefix("## ") {
            lines.push(Line::from(Span::styled(
                format!("  {heading}"),
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            )));
        } else if let Some(heading) = raw_line.strip_prefix("# ") {
            lines.push(Line::from(Span::styled(
                heading.to_string(),
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            )));
        } else if let Some(item) = raw_line.strip_prefix("- ") {
            lines.push(Line::from(vec![
                Span::styled("  • ", Style::default().fg(Color::Yellow)),
                Span::raw(render_inline(item)),
            ]));
        } else if let Some(item) = raw_line.strip_prefix("* ") {
            lines.push(Line::from(vec![
                Span::styled("  • ", Style::default().fg(Color::Yellow)),
                Span::raw(render_inline(item)),
            ]));
        } else if raw_line.is_empty() {
            lines.push(Line::from(""));
        } else {
            lines.push(Line::from(render_inline(raw_line)));
        }
    }

    lines
}

/// Simple inline rendering: strip markdown syntax for plain text display.
/// A full inline parser with mixed spans would be more complex; this is pragmatic.
fn render_inline(text: &str) -> String {
    text.replace("**", "").replace('`', "")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_heading() {
        let lines = render_markdown("# Title\n## Subtitle\n### Section");
        assert_eq!(lines.len(), 3);
    }

    #[test]
    fn test_list() {
        let lines = render_markdown("- item one\n- item two");
        assert_eq!(lines.len(), 2);
    }

    #[test]
    fn test_code_block() {
        let lines = render_markdown("```\nlet x = 1;\n```");
        assert_eq!(lines.len(), 1); // only the code line, ``` markers are skipped
    }
}
