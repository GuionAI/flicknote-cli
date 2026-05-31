//! Frontmatter parsing and rendering for editable Markdown documents.
//!
//! The editable-note contract:
//!
//! Full-note Markdown uses Obsidian-style YAML frontmatter at byte 0 when frontmatter
//! exists, followed by the leading H1 title convention:
//!
//! ```markdown
//! ---
//! topics:
//!   - rust
//! entities:
//!   - PowerSync
//! custom: keep me
//! ---
//! # Note title
//!
//! Body...
//! ```
//!
//! Rules:
//! - `notes.title` is represented only as the leading H1, not as a frontmatter key.
//! - Managed extraction frontmatter keys are `topics` and `entities`.
//! - Existing user frontmatter keys must round-trip transparently.
//! - Read paths merge DB extraction rows into displayed frontmatter.
//! - Write paths split the document back into `notes.title`, stored body content,
//!   unmanaged stored frontmatter, and `note_extractions`.
//! - JSON output stays structured and must not use synthetic Markdown.
/// Result of parsing a full editable Markdown document.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct EditableDoc {
    /// Title extracted from leading H1 after optional frontmatter.
    pub title: Option<String>,
    /// Body content without the synthetic H1 line.
    pub body: String,
    /// Managed extraction values: `topics` and `entities`.
    pub topics: Vec<String>,
    pub entities: Vec<String>,
    /// User-owned frontmatter keys that round-trip transparently.
    /// Raw YAML string (including `---` delimiters) or None if no user frontmatter.
    pub unmanaged_frontmatter: Option<String>,
}
/// Error returned when a full-note editable document is missing a required H1 title.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct MissingTitleError {
    pub message: String,
}
impl std::fmt::Display for MissingTitleError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}
impl std::error::Error for MissingTitleError {}
/// Validate that a full-note editable document has a non-empty H1 title.
///
/// Full-note writes must contain a leading `# Title` after optional frontmatter.
/// Missing or empty H1 is an error — do not silently preserve the old title.
pub(crate) fn validate_title_required(doc: &EditableDoc) -> Result<(), MissingTitleError> {
    match &doc.title {
        None => Err(MissingTitleError {
            message: "Full-note write requires a leading H1 title (e.g. `# My Title`) after optional frontmatter. \
                     Missing H1 is not allowed — add a title to the document."
                .into(),
        }),
        Some(t) if t.trim().is_empty() => Err(MissingTitleError {
            message: "Full-note write requires a non-empty H1 title. \
                     An empty `# ` heading is not a valid title."
                .into(),
        }),
        _ => Ok(()),
    }
}
/// Detect and parse leading YAML frontmatter.
///
/// Returns `(frontmatter_body, rest_of_doc)` when document starts with `---`
/// at byte 0 and is closed by a subsequent `---` delimiter on its own line.
/// Returns `(None, original)` if no valid frontmatter block is found.
pub(crate) fn split_frontmatter(content: &str) -> (Option<&str>, &str) {
    let trimmed = content;
    if !trimmed.starts_with("---\n") && trimmed != "---" {
        return (None, content);
    }
    // Find the closing `---`
    let after_open = &trimmed[3..]; // skip opening ---
    if let Some(end_idx) = after_open.find("\n---") {
        let fm_end = 3 + end_idx + 4; // position after closing ---\n (the \n is included in find)
        let fm = &trimmed[..fm_end];
        let rest = &trimmed[fm_end..];
        let rest = rest.trim_start_matches('\n');
        (Some(fm), rest)
    } else if after_open == "---\n" || after_open == "---" {
        // Empty frontmatter: ---\n---
        (Some(trimmed), "")
    } else {
        // No closing delimiter found — treat as body
        (None, content)
    }
}
/// Parse managed list values from a YAML frontmatter body.
///
/// This is a minimal parser that handles the simple YAML list shape:
/// ```yaml
/// key:
///   - value1
///   - value2
/// ```
/// Also supports inline empty lists: `key: []`
fn parse_frontmatter_list(fm_body: &str, key: &str) -> Option<Vec<String>> {
    let fm_lines: Vec<&str> = fm_body.lines().collect();
    // Find the key line
    let key_idx = fm_lines.iter().position(|line| {
        let trimmed = line.trim();
        trimmed == format!("{key}:") || trimmed == format!("{key}: []")
    })?;
    // Check for inline empty list
    if fm_lines[key_idx].trim() == format!("{key}: []") {
        return Some(Vec::new());
    }
    // Collect list items
    let mut values = Vec::new();
    for line in &fm_lines[key_idx + 1..] {
        let trimmed = line.trim();
        if let Some(stripped) = trimmed.strip_prefix("- ") {
            values.push(stripped.to_string());
        } else if trimmed.starts_with("-") {
            // No space after dash — not a valid list item
            break;
        } else {
            // Not a list item — end of this list
            break;
        }
    }
    if values.is_empty() && fm_lines[key_idx].trim() == format!("{key}:") {
        // empty list with no items
        return Some(Vec::new());
    }
    if values.is_empty() {
        return None;
    }
    Some(values)
}
/// Extract managed keys from a frontmatter body, returning the body with those keys removed.
///
/// Returns `(remaining_frontmatter, topics, entities)`.
/// If after removing managed keys the frontmatter is empty, returns None for remaining.
fn extract_managed_from_frontmatter(fm_body: &str) -> (Option<String>, Vec<String>, Vec<String>) {
    let topics = parse_frontmatter_list(fm_body, "topics").unwrap_or_default();
    let entities = parse_frontmatter_list(fm_body, "entities").unwrap_or_default();
    // Remove managed keys from the frontmatter body
    let managed_keys = ["topics:", "entities:"];
    let lines: Vec<&str> = fm_body.lines().collect();
    let mut remaining_lines: Vec<&str> = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        let trimmed = line.trim();
        // Check if this is a managed key
        let is_managed = managed_keys
            .iter()
            .any(|k| trimmed == *k || trimmed.starts_with(&format!("{k} []")));
        if is_managed {
            // Skip this line and all subsequent indented list items
            i += 1;
            while i < lines.len() {
                let next_trimmed = lines[i].trim();
                if next_trimmed.starts_with("- ") || next_trimmed == "-" {
                    i += 1;
                } else if next_trimmed.starts_with('-') {
                    // No space — might be invalid, stop skipping
                    break;
                } else {
                    break;
                }
            }
            continue;
        }
        remaining_lines.push(line);
        i += 1;
    }
    // If the remaining frontmatter only has `---` delimiters, treat as empty
    let remaining: Vec<&str> = remaining_lines
        .iter()
        .filter(|l| !l.trim().is_empty() && l.trim() != "---")
        .copied()
        .collect();
    if remaining.is_empty() {
        (None, topics, entities)
    } else {
        (Some(remaining_lines.join("\n")), topics, entities)
    }
}
/// Render a YAML frontmatter block from managed extractions and optional user frontmatter.
///
/// Merges managed `topics`/`entities` into the user frontmatter (or creates frontmatter
/// if only managed values exist). Returns the full frontmatter block including `---`
/// delimiters, or None if there is nothing to render.
pub(crate) fn render_frontmatter(
    topics: &[String],
    entities: &[String],
    user_frontmatter: Option<&str>,
) -> Option<String> {
    let has_managed = !topics.is_empty() || !entities.is_empty();
    let has_user = user_frontmatter.is_some_and(|fm| {
        let body = fm.trim();
        !body.is_empty() && body != "---"
    });
    if !has_managed && !has_user {
        return None;
    }
    let mut lines: Vec<String> = vec!["---".to_string()];
    // Add managed keys first
    if !topics.is_empty() {
        lines.push("topics:".to_string());
        for t in topics {
            lines.push(format!("  - {t}"));
        }
    } else if has_managed {
        // Render empty managed lists explicitly
    }
    if !entities.is_empty() {
        lines.push("entities:".to_string());
        for e in entities {
            lines.push(format!("  - {e}"));
        }
    } else if has_managed {
        // Render empty managed lists explicitly
    }
    // Add user frontmatter (preserving as much of the original formatting as practical)
    if let Some(fm) = user_frontmatter {
        let fm_body = fm.trim();
        // Strip the opening/closing `---` if present
        let fm_body = fm_body
            .strip_prefix("---")
            .unwrap_or(fm_body)
            .strip_suffix("---")
            .unwrap_or(fm_body);
        let fm_body = fm_body.trim();
        if !fm_body.is_empty() {
            // Remove managed keys from user's frontmatter before including
            let (remaining, _, _) = extract_managed_from_frontmatter(fm_body);
            if let Some(ref r) = remaining {
                // Already stripped ---, extract managed keys again
                let (clean, _, _) = extract_managed_from_frontmatter(r);
                if let Some(c) = clean {
                    let c = c.trim();
                    // Strip leading/trailing --- that might remain
                    let c = c.strip_prefix("---").unwrap_or(c);
                    let c = c.strip_suffix("---").unwrap_or(c);
                    let c = c.trim();
                    if !c.is_empty() {
                        lines.push(c.to_string());
                    }
                }
            }
        }
    }
    lines.push("---".to_string());
    Some(lines.join("\n"))
}
/// Parse a full editable Markdown document into its components.
///
/// Extracts:
/// - title from leading H1 after optional frontmatter
/// - body without the synthetic H1
/// - managed extraction values for `topics` and `entities`
/// - unmanaged frontmatter preserved in stored content when unknown keys remain
pub(crate) fn parse_editable_doc(content: &str) -> EditableDoc {
    let (fm_opt, after_fm) = split_frontmatter(content);
    let (unmanaged_fm, topics, entities) = if let Some(fm) = fm_opt {
        let fm_body = fm
            .strip_prefix("---")
            .unwrap_or(fm)
            .strip_suffix("---")
            .unwrap_or(fm);
        extract_managed_from_frontmatter(fm_body)
    } else {
        (None, Vec::new(), Vec::new())
    };
    // Extract title from leading H1
    let (title, body) = crate::utils::extract_title_and_strip(after_fm);
    EditableDoc {
        title,
        body,
        topics,
        entities,
        unmanaged_frontmatter: unmanaged_fm,
    }
}
/// Build the full editable document content for reading.
///
/// Takes a title, body content, DB extraction rows, and optional stored unmanaged
/// frontmatter. Returns the full Markdown with frontmatter + H1 + body.
pub(crate) fn build_editable_content(
    title: Option<&str>,
    body: &str,
    topics: &[String],
    entities: &[String],
    stored_frontmatter: Option<&str>,
) -> String {
    let mut parts = Vec::new();
    // Frontmatter
    let fm = render_frontmatter(topics, entities, stored_frontmatter);
    if let Some(fm) = fm {
        parts.push(fm);
    }
    // Leading H1
    if let Some(t) = title {
        parts.push(format!("# {t}"));
    }
    // Body (with blank line separator after H1)
    let body = body.trim_end();
    if !body.is_empty() {
        if title.is_some() {
            parts.push(String::new()); // blank line after H1
        }
        parts.push(body.to_string());
    } else if title.is_some() {
        // Just a title, no body — no trailing blank line
    }
    let result = parts.join("\n");
    // Ensure trailing newline
    if result.is_empty() {
        result
    } else {
        format!("{result}\n")
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_split_frontmatter_simple() {
        let input = "---\ntopics:\n  - rust\n---\n# Title\n\nBody.\n";
        let (fm, rest) = split_frontmatter(input);
        assert!(fm.is_some());
        assert_eq!(fm.unwrap(), "---\ntopics:\n  - rust\n---");
        assert_eq!(rest, "# Title\n\nBody.\n");
    }
    #[test]
    fn test_split_frontmatter_none() {
        let input = "# Just a heading\n\nBody.\n";
        let (fm, rest) = split_frontmatter(input);
        assert!(fm.is_none());
        assert_eq!(rest, input);
    }
    #[test]
    fn test_split_frontmatter_empty() {
        let input = "---\n---\n# Title\n\nBody.\n";
        let (fm, rest) = split_frontmatter(input);
        assert!(fm.is_some());
        assert_eq!(rest, "# Title\n\nBody.\n");
    }
    #[test]
    fn test_split_frontmatter_malformed_no_close() {
        // Malformed: no closing ---, treat as body
        let input = "---\ntopics:\n  - rust\n\n# Title\n\nBody.\n";
        let (fm, rest) = split_frontmatter(input);
        assert!(fm.is_none());
        assert_eq!(rest, input);
    }
    #[test]
    fn test_split_frontmatter_unterminated_preserved() {
        // Unterminated frontmatter is treated as normal body text
        let input = "---\nunclosed: true\n# Title\n\nBody.\n";
        let (fm, rest) = split_frontmatter(input);
        assert!(fm.is_none());
        assert_eq!(rest, input);
    }
    #[test]
    fn test_parse_frontmatter_list_simple() {
        let fm = "topics:\n  - rust\n  - async\nentities:\n  - Tokio\n";
        let topics = parse_frontmatter_list(fm, "topics");
        assert_eq!(topics, Some(vec!["rust".to_string(), "async".to_string()]));
    }
    #[test]
    fn test_parse_frontmatter_list_empty_inline() {
        let fm = "topics: []\nentities:\n  - Tokio\n";
        let topics = parse_frontmatter_list(fm, "topics");
        assert_eq!(topics, Some(vec![]));
    }
    #[test]
    fn test_parse_editable_doc_full() {
        let input = "---\ntopics:\n  - rust\nentities:\n  - PowerSync\ncustom: keep\n---\n# My Title\n\nBody goes here.\n";
        let doc = parse_editable_doc(input);
        assert_eq!(doc.title, Some("My Title".to_string()));
        assert_eq!(doc.body, "Body goes here.\n");
        assert_eq!(doc.topics, vec!["rust".to_string()]);
        assert_eq!(doc.entities, vec!["PowerSync".to_string()]);
        assert!(doc.unmanaged_frontmatter.is_some());
    }
    #[test]
    fn test_parse_editable_doc_no_frontmatter() {
        let input = "# Just a Title\n\nBody.\n";
        let doc = parse_editable_doc(input);
        assert_eq!(doc.title, Some("Just a Title".to_string()));
        assert_eq!(doc.body, "Body.\n");
        assert!(doc.topics.is_empty());
        assert!(doc.entities.is_empty());
        assert!(doc.unmanaged_frontmatter.is_none());
    }
    #[test]
    fn test_parse_editable_doc_stored_custom_keys_only() {
        // Stored content has custom keys only; read output adds managed keys
        let input = "---\ncustom: keep me\npriority: high\n---\n# Title\n\nBody.\n";
        let doc = parse_editable_doc(input);
        assert_eq!(doc.title, Some("Title".to_string()));
        assert!(doc.topics.is_empty());
        assert!(doc.entities.is_empty());
        assert!(doc.unmanaged_frontmatter.is_some());
        let fm = doc.unmanaged_frontmatter.unwrap();
        assert!(fm.contains("custom: keep me"));
        assert!(fm.contains("priority: high"));
    }
    #[test]
    fn test_parse_editable_doc_stale_managed_keys() {
        // Stored content has stale managed keys; DB extraction rows are source of truth
        let input =
            "---\ntopics:\n  - old-topic\nentities:\n  - old-entity\n---\n# Title\n\nBody.\n";
        let doc = parse_editable_doc(input);
        // Managed keys are extracted and not in unmanaged
        assert_eq!(doc.topics, vec!["old-topic".to_string()]);
        assert_eq!(doc.entities, vec!["old-entity".to_string()]);
        // After extraction, unmanaged should NOT contain topics/entities
        if let Some(ref fm) = doc.unmanaged_frontmatter {
            assert!(!fm.contains("topics:"));
            assert!(!fm.contains("entities:"));
        }
    }
    #[test]
    fn test_parse_editable_doc_all_custom_removed() {
        // If all custom keys are removed and managed keys are parsed into DB rows,
        // stored content should not keep an empty frontmatter block
        let input = "---\ntopics:\n  - rust\n---\n# Title\n\nBody.\n";
        let doc = parse_editable_doc(input);
        assert_eq!(doc.topics, vec!["rust".to_string()]);
        assert!(doc.entities.is_empty());
        // No unmanaged frontmatter since the only key was managed
        assert!(doc.unmanaged_frontmatter.is_none());
    }
    #[test]
    fn test_render_frontmatter_managed_only() {
        let topics = vec!["rust".to_string()];
        let entities = vec!["PowerSync".to_string()];
        let fm = render_frontmatter(&topics, &entities, None);
        assert!(fm.is_some());
        let fm = fm.unwrap();
        assert!(fm.contains("topics:"));
        assert!(fm.contains("  - rust"));
        assert!(fm.contains("entities:"));
        assert!(fm.contains("  - PowerSync"));
    }
    #[test]
    fn test_render_frontmatter_none() {
        let fm = render_frontmatter(&[], &[], None);
        assert!(fm.is_none());
    }
    #[test]
    fn test_render_frontmatter_merge_custom() {
        // Merge managed extraction values with existing user frontmatter
        let topics = vec!["rust".to_string()];
        let entities = vec!["PowerSync".to_string()];
        let user_fm = "---\ncustom: keep\npriority: high\n---\n";
        let fm = render_frontmatter(&topics, &entities, Some(user_fm));
        assert!(fm.is_some());
        let fm = fm.unwrap();
        assert!(fm.contains("topics:"));
        assert!(fm.contains("entities:"));
        assert!(fm.contains("custom: keep"));
        assert!(fm.contains("priority: high"));
    }
    #[test]
    fn test_build_editable_content_full() {
        let content = build_editable_content(
            Some("My Title"),
            "Body text.",
            &["rust".to_string()],
            &["PowerSync".to_string()],
            None,
        );
        assert!(content.starts_with("---\n"));
        assert!(content.contains("# My Title"));
        assert!(content.contains("Body text."));
    }
    #[test]
    fn test_build_editable_content_no_extractions() {
        let content = build_editable_content(Some("Title"), "Body.", &[], &[], None);
        assert!(!content.starts_with("---"));
        assert!(content.starts_with("# Title"));
    }
    #[test]
    fn test_build_editable_content_with_stored_frontmatter() {
        // Note with stored custom frontmatter + DB extractions merges into one block
        let stored_fm = "---\ncustom: keep\n---\n\nbody content";
        let (fm_opt, _) = split_frontmatter(stored_fm);
        let content =
            build_editable_content(Some("Title"), stored_fm, &["rust".to_string()], &[], fm_opt);
        assert!(content.starts_with("---\n"));
        assert!(content.contains("topics:"));
        assert!(content.contains("custom: keep"));
        assert!(content.contains("# Title"));
    }
    #[test]
    fn test_build_editable_content_no_title() {
        let content = build_editable_content(None, "Just body.", &["rust".to_string()], &[], None);
        assert!(content.starts_with("---\n"));
        assert!(!content.contains("#"));
        assert!(content.contains("Just body."));
    }
    #[test]
    fn test_extract_managed_from_frontmatter_clean() {
        let fm_body = "topics:\n  - a\nentities:\n  - b\ncustom: y\n";
        let (remaining, topics, entities) = extract_managed_from_frontmatter(fm_body);
        assert_eq!(topics, vec!["a".to_string()]);
        assert_eq!(entities, vec!["b".to_string()]);
        assert!(remaining.is_some());
        let remaining = remaining.unwrap();
        assert!(!remaining.contains("topics:"));
        assert!(!remaining.contains("entities:"));
        assert!(remaining.contains("custom: y"));
    }
    #[test]
    fn test_extract_managed_from_frontmatter_only_managed() {
        let fm_body = "topics:\n  - a\nentities:\n  - b\n";
        let (remaining, topics, entities) = extract_managed_from_frontmatter(fm_body);
        assert_eq!(topics, vec!["a".to_string()]);
        assert_eq!(entities, vec!["b".to_string()]);
        assert!(remaining.is_none());
    }
    // ─── Title guard tests ────────────────────────────────────────────────
    #[test]
    fn test_validate_title_required_some() {
        let doc = EditableDoc {
            title: Some("My Title".to_string()),
            body: "Body.".to_string(),
            topics: vec![],
            entities: vec![],
            unmanaged_frontmatter: None,
        };
        assert!(validate_title_required(&doc).is_ok());
    }
    #[test]
    fn test_validate_title_required_none_rejected() {
        let doc = EditableDoc {
            title: None,
            body: "Body.".to_string(),
            topics: vec![],
            entities: vec![],
            unmanaged_frontmatter: None,
        };
        let err = validate_title_required(&doc);
        assert!(err.is_err());
        let msg = err.unwrap_err().message;
        assert!(msg.contains("requires a leading H1 title"));
    }
    #[test]
    fn test_validate_title_required_empty_rejected() {
        let doc = EditableDoc {
            title: Some("".to_string()),
            body: "Body.".to_string(),
            topics: vec![],
            entities: vec![],
            unmanaged_frontmatter: None,
        };
        let err = validate_title_required(&doc);
        assert!(err.is_err());
        assert!(err.unwrap_err().message.contains("non-empty H1"));
    }
    #[test]
    fn test_validate_title_required_whitespace_only_rejected() {
        let doc = EditableDoc {
            title: Some("  ".to_string()),
            body: "Body.".to_string(),
            topics: vec![],
            entities: vec![],
            unmanaged_frontmatter: None,
        };
        let err = validate_title_required(&doc);
        assert!(err.is_err());
    }
    // ─── Full-note write edge case tests ──────────────────────────────────
    #[test]
    fn test_parse_editable_doc_h1_after_frontmatter() {
        // H1 after frontmatter is detected as title
        let input = "---\ntopics:\n  - rust\n---\n# After FM\n\nBody.\n";
        let doc = parse_editable_doc(input);
        assert_eq!(doc.title, Some("After FM".to_string()));
        assert_eq!(doc.topics, vec!["rust".to_string()]);
    }
    #[test]
    fn test_parse_editable_doc_frontmatter_only_no_h1() {
        // Frontmatter only, no H1 — title is None
        let input = "---\ntopics:\n  - rust\n---\n\nBody without title.\n";
        let doc = parse_editable_doc(input);
        assert_eq!(doc.title, None);
        assert_eq!(doc.body, "Body without title.\n");
    }
    #[test]
    fn test_parse_editable_doc_deleting_topics_clears_them() {
        // When topics key is removed from frontmatter, parsed doc has empty topics
        let input = "---\nentities:\n  - PowerSync\n---\n# Title\n\nBody.\n";
        let doc = parse_editable_doc(input);
        assert!(doc.topics.is_empty());
        assert_eq!(doc.entities, vec!["PowerSync".to_string()]);
    }
    #[test]
    fn test_parse_editable_doc_deleting_entities_clears_them() {
        // When entities key is removed from frontmatter, parsed doc has empty entities
        let input = "---\ntopics:\n  - rust\n---\n# Title\n\nBody.\n";
        let doc = parse_editable_doc(input);
        assert_eq!(doc.topics, vec!["rust".to_string()]);
        assert!(doc.entities.is_empty());
    }
    #[test]
    fn test_parse_editable_doc_managed_keys_not_duplicated() {
        // When both topics and entities are managed, they don't leak into unmanaged
        let input =
            "---\ntopics:\n  - rust\nentities:\n  - Tokio\ncustom: kept\n---\n# Title\n\nBody.\n";
        let doc = parse_editable_doc(input);
        // Managed keys extracted
        assert_eq!(doc.topics, vec!["rust".to_string()]);
        assert_eq!(doc.entities, vec!["Tokio".to_string()]);
        // Unmanaged should contain custom: kept but NOT topics/entities
        assert!(doc.unmanaged_frontmatter.is_some());
        let fm = doc.unmanaged_frontmatter.unwrap();
        assert!(fm.contains("custom: kept"));
        assert!(!fm.contains("topics:"));
        assert!(!fm.contains("entities:"));
    }
    #[test]
    fn test_parse_editable_doc_no_frontmatter_clears_extractions() {
        // No frontmatter at all → both topics and entities are empty
        let input = "# Title\n\nBody.\n";
        let doc = parse_editable_doc(input);
        assert!(
            doc.topics.is_empty(),
            "topics should be empty when no frontmatter"
        );
        assert!(
            doc.entities.is_empty(),
            "entities should be empty when no frontmatter"
        );
        assert!(doc.unmanaged_frontmatter.is_none());
    }
    #[test]
    fn test_parse_editable_doc_topics_empty_list_clears() {
        // topics: [] in frontmatter → topics is empty vec
        let input = "---\ntopics: []\nentities:\n  - Tokio\n---\n# Title\n\nBody.\n";
        let doc = parse_editable_doc(input);
        assert!(
            doc.topics.is_empty(),
            "topics: [] should result in empty vec"
        );
        assert_eq!(doc.entities, vec!["Tokio".to_string()]);
    }
    #[test]
    fn test_parse_editable_doc_entities_empty_list_clears() {
        // entities: [] in frontmatter → entities is empty vec
        let input = "---\ntopics:\n  - rust\nentities: []\n---\n# Title\n\nBody.\n";
        let doc = parse_editable_doc(input);
        assert_eq!(doc.topics, vec!["rust".to_string()]);
        assert!(
            doc.entities.is_empty(),
            "entities: [] should result in empty vec"
        );
    }
    #[test]
    fn test_parse_editable_doc_absent_key_clears_that_type() {
        // Only topics present, entities absent → entities is empty
        let input = "---\ntopics:\n  - rust\n---\n# Title\n\nBody.\n";
        let doc = parse_editable_doc(input);
        assert_eq!(doc.topics, vec!["rust".to_string()]);
        assert!(
            doc.entities.is_empty(),
            "absent entities key should result in empty vec"
        );
    }
    #[test]
    fn test_parse_editable_doc_both_keys_absent_clears_both() {
        // Neither topics nor entities in frontmatter → both empty
        let input = "---\ncustom: only\n---\n# Title\n\nBody.\n";
        let doc = parse_editable_doc(input);
        assert!(doc.topics.is_empty(), "absent topics should be empty");
        assert!(doc.entities.is_empty(), "absent entities should be empty");
        assert!(doc.unmanaged_frontmatter.is_some());
    }
}
