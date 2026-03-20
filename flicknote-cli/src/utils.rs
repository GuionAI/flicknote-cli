/// Extract the title from a *leading* H1 heading and return both the title
/// and the content with the H1 line stripped (plus any immediately following blank lines).
/// Only treats the H1 as the title if all lines before it are blank (leading H1 convention).
/// If no H1 is found, returns `(None, original_content)`. Trailing content is preserved as-is.
pub(crate) fn extract_title_and_strip(content: &str) -> (Option<String>, String) {
    let mut title: Option<String> = None;
    let mut h1_line_idx: Option<usize> = None;

    for (idx, line) in content.lines().enumerate() {
        let trimmed = line.trim();
        // Any non-blank content before an H1 means this is not a leading H1 — stop looking
        if !trimmed.is_empty() && !trimmed.starts_with("# ") {
            break;
        }
        if let Some(t) = trimmed.strip_prefix("# ") {
            let t = t.trim();
            if !t.is_empty() {
                title = Some(t.to_string());
                h1_line_idx = Some(idx);
                break;
            }
        }
    }

    let Some(h1_idx) = h1_line_idx else {
        return (None, content.to_string());
    };

    // Collect lines, skip the H1 line and any immediately following blank lines
    let lines: Vec<&str> = content.lines().collect();
    let mut rest_start = h1_idx + 1;
    while rest_start < lines.len() && lines[rest_start].trim().is_empty() {
        rest_start += 1;
    }

    // Preserve trailing newline if original had one
    let stripped = lines[rest_start..].join("\n");
    let trailing = if content.ends_with('\n') && !stripped.is_empty() {
        format!("{stripped}\n")
    } else {
        stripped
    };
    (title, trailing)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_title_strips_h1() {
        let content = "# My Title\n\nBody text here.";
        let (title, stripped) = extract_title_and_strip(content);
        assert_eq!(title, Some("My Title".to_string()));
        assert_eq!(stripped, "Body text here.");
    }

    #[test]
    fn test_extract_title_strips_h1_with_leading_whitespace() {
        let content = "  # My Title\n\nBody text here.";
        let (title, stripped) = extract_title_and_strip(content);
        assert_eq!(title, Some("My Title".to_string()));
        assert_eq!(stripped, "Body text here.");
    }

    #[test]
    fn test_extract_title_no_h1_returns_none_and_original() {
        let content = "No heading here.\n\nJust body.";
        let (title, stripped) = extract_title_and_strip(content);
        assert_eq!(title, None);
        assert_eq!(stripped, "No heading here.\n\nJust body.");
    }

    #[test]
    fn test_extract_title_h1_only_no_body() {
        let content = "# Just a Title";
        let (title, stripped) = extract_title_and_strip(content);
        assert_eq!(title, Some("Just a Title".to_string()));
        assert_eq!(stripped, "");
    }

    #[test]
    fn test_extract_title_h1_with_blank_lines_before_body() {
        let content = "# Title\n\n\nBody after two blank lines.";
        let (title, stripped) = extract_title_and_strip(content);
        assert_eq!(title, Some("Title".to_string()));
        assert_eq!(stripped, "Body after two blank lines.");
    }

    #[test]
    fn test_extract_title_preserves_h2_and_below() {
        let content = "# Main Title\n\n## Section One\n\nContent.\n\n## Section Two\n\nMore.";
        let (title, stripped) = extract_title_and_strip(content);
        assert_eq!(title, Some("Main Title".to_string()));
        assert_eq!(
            stripped,
            "## Section One\n\nContent.\n\n## Section Two\n\nMore."
        );
    }

    #[test]
    fn test_extract_title_empty_content() {
        let content = "";
        let (title, stripped) = extract_title_and_strip(content);
        assert_eq!(title, None);
        assert_eq!(stripped, "");
    }

    #[test]
    fn test_extract_title_non_leading_h1_ignored() {
        let content = "Some intro.\n\n# Heading\n\nBody.";
        let (title, stripped) = extract_title_and_strip(content);
        assert_eq!(title, None);
        assert_eq!(stripped, "Some intro.\n\n# Heading\n\nBody.");
    }

    #[test]
    fn test_extract_title_trailing_newline_trimmed() {
        let content = "# Title\n\nBody.\n";
        let (title, stripped) = extract_title_and_strip(content);
        assert_eq!(title, Some("Title".to_string()));
        // Trailing newline is preserved — consistent with how add.rs only trims stdin with trim_end()
        assert_eq!(stripped, "Body.\n");
    }
}
