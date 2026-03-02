use pulldown_cmark::{Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use ratatui::prelude::*;

/// Convert markdown text into styled ratatui Lines.
pub(crate) fn to_lines(input: &str) -> Vec<Line<'static>> {
    let options = Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TABLES;
    let parser = Parser::new_ext(input, options);

    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut current_spans: Vec<Span<'static>> = Vec::new();
    let mut style_stack: Vec<Style> = vec![Style::default()];
    let mut list_depth: usize = 0;
    let mut in_code_block = false;
    let mut in_blockquote = false;
    let mut pending_link_url: Option<String> = None;

    for event in parser {
        match event {
            Event::Start(tag) => match tag {
                Tag::Heading { level, .. } => {
                    let style = match level {
                        HeadingLevel::H1 => Style::new().bold().fg(Color::Cyan),
                        HeadingLevel::H2 => Style::new().bold().fg(Color::Green),
                        HeadingLevel::H3 => Style::new().bold().fg(Color::Yellow),
                        _ => Style::new().bold(),
                    };
                    style_stack.push(style);
                }
                Tag::Emphasis => {
                    let base = current_style(&style_stack);
                    style_stack.push(base.italic());
                }
                Tag::Strong => {
                    let base = current_style(&style_stack);
                    style_stack.push(base.bold());
                }
                Tag::Strikethrough => {
                    let base = current_style(&style_stack);
                    style_stack.push(base.crossed_out());
                }
                Tag::CodeBlock(_) => {
                    in_code_block = true;
                    flush_line(&mut current_spans, &mut lines);
                }
                Tag::List(_) => {
                    list_depth += 1;
                }
                Tag::Item => {
                    let indent = "  ".repeat(list_depth.saturating_sub(1));
                    current_spans.push(Span::styled(
                        format!("{indent}• "),
                        Style::new().fg(Color::DarkGray),
                    ));
                }
                Tag::Link { dest_url, .. } => {
                    let base = current_style(&style_stack);
                    style_stack.push(base.fg(Color::Cyan).underlined());
                    pending_link_url = Some(dest_url.to_string());
                }
                Tag::BlockQuote(_) => {
                    let base = current_style(&style_stack);
                    style_stack.push(base.fg(Color::DarkGray).italic());
                    in_blockquote = true;
                    current_spans.push(Span::styled("│ ", Style::new().fg(Color::DarkGray)));
                }
                Tag::Paragraph => {}
                _ => {}
            },
            Event::End(tag_end) => match tag_end {
                TagEnd::Heading(_) => {
                    style_stack.pop();
                    flush_line(&mut current_spans, &mut lines);
                }
                TagEnd::Paragraph => {
                    flush_line(&mut current_spans, &mut lines);
                    lines.push(Line::raw(""));
                }
                TagEnd::Emphasis | TagEnd::Strong | TagEnd::Strikethrough => {
                    style_stack.pop();
                }
                TagEnd::Link => {
                    style_stack.pop();
                    if let Some(url) = pending_link_url.take() {
                        current_spans.push(Span::styled(
                            format!(" ({url})"),
                            Style::new().fg(Color::DarkGray),
                        ));
                    }
                }
                TagEnd::CodeBlock => {
                    in_code_block = false;
                }
                TagEnd::List(_) => {
                    list_depth = list_depth.saturating_sub(1);
                    if list_depth == 0 {
                        lines.push(Line::raw(""));
                    }
                }
                TagEnd::Item => {
                    flush_line(&mut current_spans, &mut lines);
                }
                TagEnd::BlockQuote(_) => {
                    style_stack.pop();
                    in_blockquote = false;
                    flush_line(&mut current_spans, &mut lines);
                }
                _ => {}
            },
            Event::Text(text) => {
                if in_code_block {
                    for line in text.as_ref().lines() {
                        current_spans.push(Span::styled(
                            format!("  {line}"),
                            Style::new().fg(Color::Gray),
                        ));
                        flush_line(&mut current_spans, &mut lines);
                    }
                } else {
                    let style = current_style(&style_stack);
                    current_spans.push(Span::styled(text.to_string(), style));
                }
            }
            Event::Code(code) => {
                current_spans.push(Span::styled(
                    format!("`{code}`"),
                    Style::new().fg(Color::Yellow),
                ));
            }
            Event::SoftBreak | Event::HardBreak => {
                flush_line(&mut current_spans, &mut lines);
                if in_blockquote {
                    current_spans.push(Span::styled("│ ", Style::new().fg(Color::DarkGray)));
                }
            }
            Event::Rule => {
                flush_line(&mut current_spans, &mut lines);
                lines.push(Line::styled(
                    "────────────────",
                    Style::new().fg(Color::DarkGray),
                ));
            }
            _ => {}
        }
    }

    flush_line(&mut current_spans, &mut lines);

    // Remove trailing empty line
    if lines.last().is_some_and(|l| l.spans.is_empty()) {
        lines.pop();
    }

    lines
}

fn current_style(stack: &[Style]) -> Style {
    stack.last().copied().unwrap_or_default()
}

fn flush_line(spans: &mut Vec<Span<'static>>, lines: &mut Vec<Line<'static>>) {
    if !spans.is_empty() {
        lines.push(Line::from(std::mem::take(spans)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heading_renders_styled() {
        let lines = to_lines("# Hello");
        assert!(!lines.is_empty());
        let text: String = lines[0].spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(text, "Hello");
    }

    #[test]
    fn plain_text_renders() {
        let lines = to_lines("just some text");
        assert!(!lines.is_empty());
        let text: String = lines[0].spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(text, "just some text");
    }

    #[test]
    fn list_renders_bullets() {
        let lines = to_lines("- item one\n- item two");
        let all_text: String = lines
            .iter()
            .flat_map(|l| l.spans.iter())
            .map(|s| s.content.as_ref())
            .collect();
        assert!(all_text.contains("•"));
        assert!(all_text.contains("item one"));
    }

    #[test]
    fn inline_code_renders() {
        let lines = to_lines("use `foo` here");
        let all_text: String = lines
            .iter()
            .flat_map(|l| l.spans.iter())
            .map(|s| s.content.as_ref())
            .collect();
        assert!(all_text.contains("`foo`"));
    }
}
