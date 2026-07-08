use ratatui::{
    style::{Modifier, Style},
    text::{Line, Span},
};

use crate::ui::theme;

/// Convert markdown text to styled ratatui Lines.
///
/// Block constructs: ATX headings (`#`–`####`), unordered lists (`-`/`*`,
/// nested by indentation), ordered lists (`1.`/`1)`), task lists (`- [x]`),
/// blockquotes (`>`), horizontal rules, and fenced code blocks (the fence
/// line itself, including any language tag, is dropped).
///
/// Inline constructs: `**bold**`, `*italic*`/`_italic_`, `` `code` ``, and
/// `[text](url)` links. Unmatched markers render literally.
///
/// Out of scope: tables and images (no width/graphics available here — the
/// detail pane soft-wraps via `Paragraph::wrap`), setext headings, and
/// nested emphasis. Unrecognized lines pass through as plain paragraphs.
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
                Style::default().fg(theme::SUCCESS),
            )));
            continue;
        }

        // Horizontal rule — checked before lists so `- - -` isn't a bullet.
        if is_horizontal_rule(raw_line) {
            lines.push(Line::from(Span::styled(
                "─".repeat(16),
                Style::default().fg(theme::TEXT_DIM),
            )));
            continue;
        }

        if let Some(line) = try_heading(raw_line) {
            lines.push(line);
        } else if let Some(line) = try_blockquote(raw_line) {
            lines.push(line);
        } else if let Some(line) = try_list_item(raw_line) {
            lines.push(line);
        } else if raw_line.is_empty() {
            lines.push(Line::from(""));
        } else {
            lines.push(Line::from(render_inline(raw_line)));
        }
    }

    lines
}

/// A trimmed line of three or more `-`, `*`, or `_` (spaces allowed between).
fn is_horizontal_rule(line: &str) -> bool {
    let compact: String = line.chars().filter(|c| !c.is_whitespace()).collect();
    compact.len() >= 3
        && ["-", "*", "_"]
            .iter()
            .any(|m| compact.chars().all(|c| c.to_string() == *m))
}

/// ATX headings `#`–`####` (deeper levels render like `####`).
fn try_heading(line: &str) -> Option<Line<'static>> {
    let hashes = line.chars().take_while(|&c| c == '#').count();
    if hashes == 0 || !line[hashes..].starts_with(' ') {
        return None;
    }
    let text = line[hashes + 1..].to_string();
    // Indent grows with depth so the hierarchy stays visible without color changes.
    let indent = " ".repeat(hashes.saturating_sub(1).min(3));
    let base = Style::default()
        .fg(theme::ACCENT)
        .add_modifier(Modifier::BOLD);
    let mut spans = vec![Span::styled(indent, base)];
    spans.extend(patch_spans(render_inline(&text), base));
    Some(Line::from(spans))
}

/// Blockquote: one `│ ` gutter per `>` nesting level, content dimmed.
fn try_blockquote(line: &str) -> Option<Line<'static>> {
    let mut rest = line.trim_start();
    let mut depth = 0;
    while let Some(stripped) = rest.strip_prefix('>') {
        depth += 1;
        rest = stripped.strip_prefix(' ').unwrap_or(stripped);
    }
    if depth == 0 {
        return None;
    }
    let base = Style::default().fg(theme::TEXT_DIM);
    let mut spans = vec![Span::styled("│ ".repeat(depth), base)];
    spans.extend(patch_spans(render_inline(rest), base));
    Some(Line::from(spans))
}

/// Unordered (`- `/`* `), task (`- [x] `), and ordered (`1. `/`1) `) list
/// items, with leading indentation preserved for nesting.
fn try_list_item(line: &str) -> Option<Line<'static>> {
    let trimmed = line.trim_start();
    let indent = " ".repeat(line.len() - trimmed.len());

    // Task list — matched before the plain bullet so `[x]` isn't left raw.
    for (marker, symbol, color) in [
        ("- [x] ", "☑ ", theme::SUCCESS),
        ("- [X] ", "☑ ", theme::SUCCESS),
        ("- [ ] ", "☐ ", theme::TEXT_DIM),
    ] {
        if let Some(item) = trimmed.strip_prefix(marker) {
            let mut spans = vec![Span::styled(
                format!("  {indent}{symbol}"),
                Style::default().fg(color),
            )];
            spans.extend(render_inline(item));
            return Some(Line::from(spans));
        }
    }

    // Unordered bullet.
    for marker in ["- ", "* "] {
        if let Some(item) = trimmed.strip_prefix(marker) {
            let mut spans = vec![Span::styled(
                format!("  {indent}• "),
                Style::default().fg(theme::WARNING),
            )];
            spans.extend(render_inline(item));
            return Some(Line::from(spans));
        }
    }

    // Ordered: digits then `.` or `)` then a space.
    let digits = trimmed.chars().take_while(|c| c.is_ascii_digit()).count();
    if digits > 0
        && let Some(sep) = trimmed[digits..].strip_prefix(['.', ')'])
        && let Some(item) = sep.strip_prefix(' ')
    {
        let number = &trimmed[..digits + 1];
        let mut spans = vec![Span::styled(
            format!("  {indent}{number} "),
            Style::default().fg(theme::WARNING),
        )];
        spans.extend(render_inline(item));
        return Some(Line::from(spans));
    }

    None
}

/// Re-base each span's style on `base` (span-specific fg/modifiers win).
fn patch_spans(spans: Vec<Span<'static>>, base: Style) -> Vec<Span<'static>> {
    spans
        .into_iter()
        .map(|s| {
            let style = base.patch(s.style);
            Span::styled(s.content, style)
        })
        .collect()
}

/// Single-pass inline tokenizer: `` `code` ``, `**bold**`, `*italic*` /
/// `_italic_`, and `[text](url)` links become styled spans; unmatched
/// markers render literally. Emphasis does not nest (pragmatic — PR bodies
/// rarely need it, and the pane stays readable either way).
fn render_inline(text: &str) -> Vec<Span<'static>> {
    let chars: Vec<char> = text.chars().collect();
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut plain = String::new();
    let mut i = 0;

    let flush = |plain: &mut String, spans: &mut Vec<Span<'static>>| {
        if !plain.is_empty() {
            spans.push(Span::raw(std::mem::take(plain)));
        }
    };

    while i < chars.len() {
        match chars[i] {
            '`' => {
                if let Some(end) = find_char(&chars, i + 1, '`') {
                    flush(&mut plain, &mut spans);
                    let code: String = chars[i + 1..end].iter().collect();
                    spans.push(Span::styled(code, Style::default().fg(theme::SUCCESS)));
                    i = end + 1;
                } else {
                    plain.push('`');
                    i += 1;
                }
            }
            '*' if chars.get(i + 1) == Some(&'*') => {
                if let Some(end) = find_pair(&chars, i + 2, '*')
                    && end > i + 2
                {
                    flush(&mut plain, &mut spans);
                    let inner: String = chars[i + 2..end].iter().collect();
                    spans.push(Span::styled(
                        inner,
                        Style::default().add_modifier(Modifier::BOLD),
                    ));
                    i = end + 2;
                } else {
                    plain.push_str("**");
                    i += 2;
                }
            }
            '*' => {
                if chars.get(i + 1).is_some_and(|c| !c.is_whitespace())
                    && let Some(end) = find_char(&chars, i + 1, '*')
                    && end > i + 1
                {
                    flush(&mut plain, &mut spans);
                    let inner: String = chars[i + 1..end].iter().collect();
                    spans.push(Span::styled(
                        inner,
                        Style::default().add_modifier(Modifier::ITALIC),
                    ));
                    i = end + 1;
                } else {
                    plain.push('*');
                    i += 1;
                }
            }
            '_' => {
                // Word-boundary check keeps snake_case identifiers intact.
                let at_boundary = i == 0 || !chars[i - 1].is_alphanumeric();
                let closer = find_char(&chars, i + 1, '_').filter(|&end| {
                    end > i + 1 && chars.get(end + 1).is_none_or(|c| !c.is_alphanumeric())
                });
                if at_boundary
                    && chars.get(i + 1).is_some_and(|c| !c.is_whitespace())
                    && let Some(end) = closer
                {
                    flush(&mut plain, &mut spans);
                    let inner: String = chars[i + 1..end].iter().collect();
                    spans.push(Span::styled(
                        inner,
                        Style::default().add_modifier(Modifier::ITALIC),
                    ));
                    i = end + 1;
                } else {
                    plain.push('_');
                    i += 1;
                }
            }
            '[' => {
                if let Some((label, url, next)) = parse_link(&chars, i) {
                    flush(&mut plain, &mut spans);
                    spans.push(Span::styled(
                        label,
                        Style::default()
                            .fg(theme::ACCENT)
                            .add_modifier(Modifier::UNDERLINED),
                    ));
                    spans.push(Span::styled(
                        format!(" ({url})"),
                        Style::default().fg(theme::TEXT_DIM),
                    ));
                    i = next;
                } else {
                    plain.push('[');
                    i += 1;
                }
            }
            c => {
                plain.push(c);
                i += 1;
            }
        }
    }

    flush(&mut plain, &mut spans);
    spans
}

fn find_char(chars: &[char], from: usize, target: char) -> Option<usize> {
    chars[from..]
        .iter()
        .position(|&c| c == target)
        .map(|p| from + p)
}

/// Position of the next `marker` pair (e.g. the closing `**`).
fn find_pair(chars: &[char], from: usize, marker: char) -> Option<usize> {
    let mut i = from;
    while i + 1 < chars.len() {
        if chars[i] == marker && chars[i + 1] == marker {
            return Some(i);
        }
        i += 1;
    }
    None
}

/// Parse `[label](url)` starting at `chars[open] == '['`.
/// Returns (label, url, index after the closing parenthesis).
fn parse_link(chars: &[char], open: usize) -> Option<(String, String, usize)> {
    let close_bracket = find_char(chars, open + 1, ']')?;
    if chars.get(close_bracket + 1) != Some(&'(') {
        return None;
    }
    let close_paren = find_char(chars, close_bracket + 2, ')')?;
    let label: String = chars[open + 1..close_bracket].iter().collect();
    let url: String = chars[close_bracket + 2..close_paren].iter().collect();
    if label.is_empty() || url.is_empty() {
        return None;
    }
    Some((label, url, close_paren + 1))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text_of(line: &Line) -> String {
        line.spans.iter().map(|s| s.content.as_ref()).collect()
    }

    #[test]
    fn test_heading() {
        let lines = render_markdown("# Title\n## Subtitle\n### Section\n#### Deep");
        assert_eq!(lines.len(), 4);
        assert_eq!(text_of(&lines[0]), "Title");
        assert_eq!(text_of(&lines[3]), "   Deep");
    }

    #[test]
    fn heading_renders_inline_code() {
        let lines = render_markdown("## Use `gct`");
        let code = &lines[0].spans.last().unwrap();
        assert_eq!(code.content, "gct");
        assert_eq!(code.style.fg, Some(theme::SUCCESS));
    }

    #[test]
    fn test_list() {
        let lines = render_markdown("- item one\n- item two");
        assert_eq!(lines.len(), 2);
        assert_eq!(text_of(&lines[0]), "  • item one");
    }

    #[test]
    fn nested_list_preserves_indent() {
        let lines = render_markdown("- top\n  - sub");
        assert_eq!(text_of(&lines[1]), "    • sub");
    }

    #[test]
    fn ordered_list_keeps_numbers() {
        let lines = render_markdown("1. first\n2) second\n  3. nested");
        assert_eq!(text_of(&lines[0]), "  1. first");
        assert_eq!(text_of(&lines[1]), "  2) second");
        assert_eq!(text_of(&lines[2]), "    3. nested");
    }

    #[test]
    fn task_list_renders_checkboxes() {
        let lines = render_markdown("- [x] done\n- [ ] todo");
        assert_eq!(text_of(&lines[0]), "  ☑ done");
        assert_eq!(lines[0].spans[0].style.fg, Some(theme::SUCCESS));
        assert_eq!(text_of(&lines[1]), "  ☐ todo");
        assert_eq!(lines[1].spans[0].style.fg, Some(theme::TEXT_DIM));
    }

    #[test]
    fn blockquote_gets_gutter_per_depth() {
        let lines = render_markdown("> quoted\n> > deeper");
        assert_eq!(text_of(&lines[0]), "│ quoted");
        assert_eq!(text_of(&lines[1]), "│ │ deeper");
        assert_eq!(lines[0].spans[0].style.fg, Some(theme::TEXT_DIM));
    }

    #[test]
    fn horizontal_rule_renders_line() {
        let lines = render_markdown("---\n* * *");
        assert!(text_of(&lines[0]).starts_with('─'));
        assert!(text_of(&lines[1]).starts_with('─'));
    }

    #[test]
    fn test_code_block() {
        let lines = render_markdown("```rust\nlet x = 1;\n```");
        assert_eq!(lines.len(), 1); // fence lines (incl. language tag) dropped
        assert_eq!(text_of(&lines[0]), "  let x = 1;");
    }

    #[test]
    fn inline_bold_and_italic_are_styled() {
        let spans = render_inline("a **bold** and *slanted* word");
        let bold = spans.iter().find(|s| s.content == "bold").unwrap();
        assert!(bold.style.add_modifier.contains(Modifier::BOLD));
        let italic = spans.iter().find(|s| s.content == "slanted").unwrap();
        assert!(italic.style.add_modifier.contains(Modifier::ITALIC));
        let joined: String = spans.iter().map(|s| s.content.as_ref()).collect::<String>();
        assert_eq!(joined, "a bold and slanted word");
    }

    #[test]
    fn unmatched_markers_render_literally() {
        let joined = |s: &str| {
            render_inline(s)
                .iter()
                .map(|sp| sp.content.to_string())
                .collect::<String>()
        };
        assert_eq!(joined("2 ** 3 is unmatched"), "2 ** 3 is unmatched");
        assert_eq!(joined("odd `backtick"), "odd `backtick");
        assert_eq!(joined("a * b"), "a * b");
    }

    #[test]
    fn bold_marker_inside_code_span_is_untouched() {
        let spans = render_inline("`a ** b`");
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].content, "a ** b");
        assert_eq!(spans[0].style.fg, Some(theme::SUCCESS));
    }

    #[test]
    fn snake_case_is_not_italicized() {
        let spans = render_inline("call snake_case_name here");
        let joined: String = spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(joined, "call snake_case_name here");
    }

    #[test]
    fn links_show_label_and_dim_url() {
        let spans = render_inline("see [docs](https://example.com) now");
        let label = spans.iter().find(|s| s.content == "docs").unwrap();
        assert!(label.style.add_modifier.contains(Modifier::UNDERLINED));
        let url = spans
            .iter()
            .find(|s| s.content == " (https://example.com)")
            .unwrap();
        assert_eq!(url.style.fg, Some(theme::TEXT_DIM));
    }

    #[test]
    fn malformed_link_renders_literally() {
        let joined: String = render_inline("[not a link] plain")
            .iter()
            .map(|s| s.content.as_ref())
            .collect();
        assert_eq!(joined, "[not a link] plain");
    }
}
