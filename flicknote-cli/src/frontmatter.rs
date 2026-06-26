//! Frontmatter parsing and rendering for editable Markdown documents.
//!
//! The editable-note contract:
//!
//! Full-note Markdown uses Obsidian-style YAML frontmatter at byte 0 for managed
//! note fields:
//!
//! ```markdown
//! ---
//! title: Note title
//! topics:
//!   - rust
//! entities:
//!   - PowerSync
//! custom: keep me
//! ---
//! Body...
//! ```
//!
//! Rules:
//! - `notes.title` is represented only as the `title` frontmatter key.
//! - Managed extraction frontmatter keys are `topics` and `entities`.
//! - Markdown headings are normal body content.
//! - Existing user frontmatter keys must round-trip transparently.
//! - Read paths merge DB extraction rows into displayed frontmatter.
//! - Write paths split the document back into `notes.title`, stored body content,
//!   unmanaged stored frontmatter, and `note_extractions`.
//! - JSON output stays structured and must not use synthetic Markdown.
/// Result of parsing a full editable Markdown document.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct EditableDoc {
    /// Title extracted from frontmatter.
    pub title: Option<String>,
    /// Body content without managed frontmatter.
    pub body: String,
    /// Managed extraction values: `topics` and `entities`.
    pub topics: Vec<String>,
    pub entities: Vec<String>,
    /// User-owned frontmatter keys that round-trip transparently.
    /// Raw YAML string (including `---` delimiters) or None if no user frontmatter.
    pub unmanaged_frontmatter: Option<String>,
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
/// Extract managed keys from a YAML body, returning the body with those keys removed.
///
/// Uses `yaml_serde` for proper YAML parsing so that all valid YAML keys
/// (including nested structures, quoted strings, and inline lists) round-trip
/// correctly. Returns `(remaining_yaml, title, topics, entities)`.
/// If after removing managed keys the mapping is empty, returns None for remaining.
fn extract_managed_from_frontmatter(
    fm_body: &str,
) -> (Option<String>, Option<String>, Vec<String>, Vec<String>) {
    // Split_frontmatter includes the closing --- in its returned slice.
    // Strip it so yaml_serde receives a clean YAML body.
    let fm_body = frontmatter_body(fm_body);
    let Ok(mut value) = yaml_serde::from_str::<yaml_serde::Value>(fm_body) else {
        return extract_managed_from_invalid_frontmatter(fm_body);
    };
    let title = take_yaml_string(&mut value, "title");
    let topics = take_yaml_list(&mut value, "topics");
    let entities = take_yaml_list(&mut value, "entities");
    let remaining = if let Some(mapping) = value.as_mapping() {
        if mapping.is_empty() {
            return (None, title, topics, entities);
        }
        let Some(remaining) =
            serialized_yaml_body(yaml_serde::to_string(&value), "unmanaged editable note")
        else {
            return (Some(fm_body.to_string()), title, topics, entities);
        };
        remaining
    } else {
        // Scalar value: re-serialize the original body
        fm_body.to_string()
    };
    let remaining = remaining.trim().to_string();
    if remaining.is_empty() {
        (None, title, topics, entities)
    } else {
        (Some(remaining), title, topics, entities)
    }
}

fn extract_managed_from_invalid_frontmatter(
    fm_body: &str,
) -> (Option<String>, Option<String>, Vec<String>, Vec<String>) {
    let mut title = None;
    let mut topics = Vec::new();
    let mut entities = Vec::new();
    let mut remaining = Vec::new();
    let lines: Vec<_> = fm_body.lines().collect();
    let mut index = 0;

    while index < lines.len() {
        let Some(key) = managed_key_line(lines[index]) else {
            remaining.push(lines[index]);
            index += 1;
            continue;
        };
        let start = index;
        index += 1;
        while index < lines.len() && !is_top_level_mapping_key(lines[index]) {
            index += 1;
        }

        let block = lines[start..index].join("\n");
        let Ok(mut value) = yaml_serde::from_str::<yaml_serde::Value>(&block) else {
            remaining.extend_from_slice(&lines[start..index]);
            continue;
        };
        match key {
            "title" => title = take_yaml_string(&mut value, key),
            "topics" => topics = take_yaml_list(&mut value, key),
            "entities" => entities = take_yaml_list(&mut value, key),
            _ => unreachable!("managed_key_line returned an unknown key"),
        }
    }

    let remaining = remaining.join("\n").trim().to_string();
    if remaining.is_empty() {
        (None, title, topics, entities)
    } else {
        (Some(remaining), title, topics, entities)
    }
}

fn managed_key_line(line: &str) -> Option<&'static str> {
    if line.starts_with("title:") {
        Some("title")
    } else if line.starts_with("topics:") {
        Some("topics")
    } else if line.starts_with("entities:") {
        Some("entities")
    } else {
        None
    }
}

fn is_top_level_mapping_key(line: &str) -> bool {
    let Some(first) = line.chars().next() else {
        return false;
    };
    if first.is_whitespace() || first == '-' {
        return false;
    }
    let Some((key, _)) = line.split_once(':') else {
        return false;
    };
    !key.trim().is_empty()
}

fn take_yaml_string(value: &mut yaml_serde::Value, key: &str) -> Option<String> {
    let mapping = value.as_mapping_mut()?;
    let k = yaml_serde::Value::String(key.to_string());
    match mapping.remove(&k) {
        Some(yaml_serde::Value::String(value)) => {
            let value = value.trim().to_string();
            if value.is_empty() { None } else { Some(value) }
        }
        Some(yaml_serde::Value::Sequence(seq)) => {
            if seq.len() > 1 {
                log::warn!(
                    "{key} frontmatter sequence has {} values; using the first non-empty string",
                    seq.len()
                );
            }
            seq.into_iter().find_map(|v| {
                v.as_str()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(String::from)
            })
        }
        _ => None,
    }
}

/// Remove and return the list value for `key` from a YAML mapping.
/// Returns an empty Vec if the key is missing or not a sequence.
fn take_yaml_list(value: &mut yaml_serde::Value, key: &str) -> Vec<String> {
    let mapping = match value.as_mapping_mut() {
        Some(m) => m,
        None => return Vec::new(),
    };
    let k = yaml_serde::Value::String(key.to_string());
    match mapping.remove(&k) {
        Some(yaml_serde::Value::Sequence(seq)) => seq
            .into_iter()
            .filter_map(|v| v.as_str().map(String::from))
            .collect(),
        _ => Vec::new(),
    }
}

/// Render a YAML frontmatter block from managed extractions and optional user frontmatter.
///
/// Merges managed `topics`/`entities` into the user frontmatter (or creates frontmatter
/// if only managed values exist). Returns the full frontmatter block including `---`
/// delimiters, or None if there is nothing to render.
pub(crate) fn render_frontmatter(
    title: Option<&str>,
    topics: &[String],
    entities: &[String],
    user_frontmatter: Option<&str>,
) -> Option<String> {
    let title = title.map(str::trim).filter(|title| !title.is_empty());
    let has_managed = title.is_some() || !topics.is_empty() || !entities.is_empty();
    let has_user = user_frontmatter.is_some_and(|fm| {
        let body = fm.trim();
        !body.is_empty() && body != "---"
    });
    if !has_managed && !has_user {
        return None;
    }
    // Build a combined YAML mapping: managed lists + user keys
    let mut combined = yaml_serde::Mapping::new();
    if let Some(title) = title {
        combined.insert(
            yaml_serde::Value::String("title".into()),
            yaml_serde::Value::String(title.to_string()),
        );
    }
    if !topics.is_empty() {
        let seq: yaml_serde::Sequence = topics
            .iter()
            .map(|t| yaml_serde::Value::String(t.clone()))
            .collect();
        combined.insert(
            yaml_serde::Value::String("topics".into()),
            yaml_serde::Value::Sequence(seq),
        );
    }
    if !entities.is_empty() {
        let seq: yaml_serde::Sequence = entities
            .iter()
            .map(|e| yaml_serde::Value::String(e.clone()))
            .collect();
        combined.insert(
            yaml_serde::Value::String("entities".into()),
            yaml_serde::Value::Sequence(seq),
        );
    }
    if let Some(fm) = user_frontmatter {
        let fm_body = frontmatter_body(fm);
        if let Ok(mut user_value) = yaml_serde::from_str::<yaml_serde::Value>(fm_body) {
            // Strip managed keys from user mapping (managed lists already in `combined`)
            if let Some(user_mapping) = user_value.as_mapping_mut() {
                user_mapping.remove(yaml_serde::Value::String("title".into()));
                user_mapping.remove(yaml_serde::Value::String("topics".into()));
                user_mapping.remove(yaml_serde::Value::String("entities".into()));
            }
            // Merge remaining user keys into the combined mapping
            if let Some(user_map) = user_value.as_mapping() {
                for (k, v) in user_map {
                    if !combined.contains_key(k) {
                        combined.insert(k.clone(), v.clone());
                    }
                }
            }
        } else {
            return render_invalid_user_frontmatter(&combined, fm);
        }
    }
    if combined.is_empty() {
        return None;
    }
    let inner = serialized_yaml_body(yaml_serde::to_string(&combined), "editable note")?;
    Some(format!("---\n{}\n---", inner))
}

fn serialized_yaml_body(
    serialized: Result<String, yaml_serde::Error>,
    context: &str,
) -> Option<String> {
    match serialized {
        Ok(body) => Some(body.trim().to_string()),
        Err(err) => {
            log::warn!("failed to serialize {context} frontmatter: {err}");
            None
        }
    }
}

fn render_invalid_user_frontmatter(
    combined: &yaml_serde::Mapping,
    user_frontmatter: &str,
) -> Option<String> {
    if combined.is_empty() {
        return Some(normalize_frontmatter_block(user_frontmatter));
    }

    let Some(managed) = serialized_yaml_body(yaml_serde::to_string(combined), "managed extraction")
    else {
        return Some(normalize_frontmatter_block(user_frontmatter));
    };
    let user_body = frontmatter_body(user_frontmatter);
    let body = if user_body.is_empty() {
        managed.to_string()
    } else {
        format!("{managed}\n{user_body}")
    };
    Some(format!("---\n{}\n---", body.trim()))
}

fn normalize_frontmatter_block(fm: &str) -> String {
    let fm = fm.trim();
    if fm.starts_with("---") && fm.ends_with("---") {
        fm.to_string()
    } else {
        format!("---\n{}\n---", fm)
    }
}

fn frontmatter_body(fm: &str) -> &str {
    let fm = fm.trim();
    let fm = fm.strip_prefix("---").unwrap_or(fm);
    let fm = fm.strip_suffix("---").unwrap_or(fm);
    fm.trim()
}
/// Parse a full editable Markdown document into its components.
///
/// Extracts:
/// - title from frontmatter
/// - body content
/// - managed extraction values for `topics` and `entities`
/// - unmanaged frontmatter preserved in stored content when unknown keys remain
pub(crate) fn parse_editable_doc(content: &str) -> EditableDoc {
    let (fm_opt, after_fm) = split_frontmatter(content);
    let (unmanaged_fm, title, topics, entities) = if let Some(fm) = fm_opt {
        let fm_body = fm
            .strip_prefix("---")
            .unwrap_or(fm)
            .strip_suffix("---")
            .unwrap_or(fm);
        let (remaining, title, topics, entities) = extract_managed_from_frontmatter(fm_body);
        let wrapped = remaining.map(|body| format!("---\n{}\n---", body));
        (wrapped, title, topics, entities)
    } else {
        (None, None, Vec::new(), Vec::new())
    };
    EditableDoc {
        title,
        body: after_fm.to_string(),
        topics,
        entities,
        unmanaged_frontmatter: unmanaged_fm,
    }
}
/// Build the full editable document content for reading.
///
/// Takes a title, body content, DB extraction rows, and optional stored unmanaged
/// frontmatter. Returns the full Markdown with frontmatter + body.
pub(crate) fn build_editable_content(
    title: Option<&str>,
    body: &str,
    topics: &[String],
    entities: &[String],
    stored_frontmatter: Option<&str>,
) -> String {
    let mut parts = Vec::new();
    // Frontmatter
    let fm = render_frontmatter(title, topics, entities, stored_frontmatter);
    if let Some(fm) = fm {
        parts.push(fm);
    }
    let body = body.trim_end();
    if !body.is_empty() {
        if !parts.is_empty() {
            parts.push(String::new());
        }
        parts.push(body.to_string());
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
    use std::sync::Mutex;

    static TEST_LOGGER: TestLogger = TestLogger;
    static TEST_LOG_MESSAGES: Mutex<Vec<String>> = Mutex::new(Vec::new());

    struct TestLogger;

    impl log::Log for TestLogger {
        fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
            metadata.level() <= log::Level::Warn
        }

        fn log(&self, record: &log::Record<'_>) {
            if !self.enabled(record.metadata()) {
                return;
            }
            TEST_LOG_MESSAGES
                .lock()
                .unwrap()
                .push(record.args().to_string());
        }

        fn flush(&self) {}
    }

    fn clear_test_logs() {
        match log::set_logger(&TEST_LOGGER) {
            Ok(()) | Err(_) => {}
        }
        log::set_max_level(log::LevelFilter::Warn);
        TEST_LOG_MESSAGES.lock().unwrap().clear();
    }

    fn test_logs() -> Vec<String> {
        TEST_LOG_MESSAGES.lock().unwrap().clone()
    }

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
    fn test_parse_editable_doc_full() {
        let input = "---\ntitle: My Title\ntopics:\n  - rust\nentities:\n  - PowerSync\ncustom: keep\n---\n# Body Heading\n\nBody goes here.\n";
        let doc = parse_editable_doc(input);
        assert_eq!(doc.title, Some("My Title".to_string()));
        assert_eq!(doc.body, "# Body Heading\n\nBody goes here.\n");
        assert_eq!(doc.topics, vec!["rust".to_string()]);
        assert_eq!(doc.entities, vec!["PowerSync".to_string()]);
        assert!(doc.unmanaged_frontmatter.is_some());
    }
    #[test]
    fn test_parse_editable_doc_no_frontmatter() {
        let input = "# Just a Title\n\nBody.\n";
        let doc = parse_editable_doc(input);
        assert_eq!(doc.title, None);
        assert_eq!(doc.body, "# Just a Title\n\nBody.\n");
        assert!(doc.topics.is_empty());
        assert!(doc.entities.is_empty());
        assert!(doc.unmanaged_frontmatter.is_none());
    }
    #[test]
    fn test_parse_editable_doc_unmanaged_includes_delimiters() {
        // unmanaged_frontmatter must include --- delimiters so write paths
        // store a valid frontmatter block that round-trips on next read.
        let input = "---\ncustom: keep me\npriority: high\n---\n# Title\n\nBody.\n";
        let doc = parse_editable_doc(input);
        assert!(
            doc.unmanaged_frontmatter.is_some(),
            "should have unmanaged frontmatter"
        );
        let fm = doc.unmanaged_frontmatter.unwrap();
        assert!(
            fm.starts_with("---"),
            "unmanaged frontmatter must start with --- delimiter, got: {fm:?}"
        );
        assert!(
            fm.ends_with("---"),
            "unmanaged frontmatter must end with --- delimiter, got: {fm:?}"
        );
        assert!(fm.contains("custom: keep me"), "must contain custom keys");
    }
    #[test]
    fn test_parse_editable_doc_stored_custom_keys_only() {
        // Stored content has custom keys only; read output adds managed keys
        let input = "---\ncustom: keep me\npriority: high\n---\n# Title\n\nBody.\n";
        let doc = parse_editable_doc(input);
        assert_eq!(doc.title, None);
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
        let fm = render_frontmatter(None, &topics, &entities, None);
        assert!(fm.is_some());
        let fm = fm.unwrap();
        assert!(fm.contains("topics:"));
        assert!(fm.contains("- rust"));
        assert!(fm.contains("entities:"));
        assert!(fm.contains("- PowerSync"));
    }
    #[test]
    fn test_render_frontmatter_none() {
        let fm = render_frontmatter(None, &[], &[], None);
        assert!(fm.is_none());
    }
    #[test]
    fn test_render_frontmatter_strips_stored_managed_keys_without_rendering_empty_map() {
        let fm = render_frontmatter(None, &[], &[], Some("---\ntitle: Old\n---"));

        assert!(fm.is_none());
    }
    #[test]
    fn test_render_frontmatter_merge_custom() {
        // Merge managed extraction values with existing user frontmatter
        let topics = vec!["rust".to_string()];
        let entities = vec!["PowerSync".to_string()];
        let user_fm = "---\ncustom: keep\npriority: high\n---\n";
        let fm = render_frontmatter(None, &topics, &entities, Some(user_fm));
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
        assert!(content.contains("title: My Title"));
        assert!(!content.contains("# My Title"));
        assert!(content.contains("Body text."));
    }
    #[test]
    fn test_build_editable_content_renders_title_as_frontmatter() {
        let content = build_editable_content(Some("My Title"), "Body text.", &[], &[], None);

        assert_eq!(content, "---\ntitle: My Title\n---\n\nBody text.\n");
    }
    #[test]
    fn test_parse_editable_doc_uses_frontmatter_title_and_preserves_h1_body() {
        let input = "---\ntitle: Frontmatter Title\ntopics: [rust]\n---\n# Body Heading\n\nBody.\n";
        let doc = parse_editable_doc(input);

        assert_eq!(doc.title, Some("Frontmatter Title".to_string()));
        assert_eq!(doc.body, "# Body Heading\n\nBody.\n");
        assert_eq!(doc.topics, vec!["rust".to_string()]);
        assert!(doc.unmanaged_frontmatter.is_none());
    }
    #[test]
    fn test_parse_editable_doc_no_frontmatter_title_does_not_extract_h1() {
        let doc = parse_editable_doc("# Body Heading\n\nBody.\n");

        assert_eq!(doc.title, None);
        assert_eq!(doc.body, "# Body Heading\n\nBody.\n");
    }
    #[test]
    fn test_build_editable_content_no_extractions() {
        let content = build_editable_content(Some("Title"), "Body.", &[], &[], None);
        assert!(content.starts_with("---"));
        assert!(content.contains("title: Title"));
        assert!(!content.contains("# Title"));
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
        assert!(content.contains("title: Title"));
        assert!(!content.contains("# Title"));
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
        let (remaining, title, topics, entities) = extract_managed_from_frontmatter(fm_body);
        assert!(title.is_none());
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
        let (remaining, title, topics, entities) = extract_managed_from_frontmatter(fm_body);
        assert!(title.is_none());
        assert_eq!(topics, vec!["a".to_string()]);
        assert_eq!(entities, vec!["b".to_string()]);
        assert!(remaining.is_none());
    }
    // ─── Full-note write edge case tests ──────────────────────────────────
    #[test]
    fn test_parse_editable_doc_h1_after_frontmatter() {
        // H1 after frontmatter remains normal body content.
        let input = "---\ntopics:\n  - rust\n---\n# After FM\n\nBody.\n";
        let doc = parse_editable_doc(input);
        assert_eq!(doc.title, None);
        assert_eq!(doc.body, "# After FM\n\nBody.\n");
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

    #[test]
    fn test_extract_managed_from_frontmatter_inline_topics() {
        let fm_body = "topics: [rust, async]\ncustom: keep\n";
        let (remaining, title, topics, entities) = extract_managed_from_frontmatter(fm_body);

        assert!(title.is_none());
        assert_eq!(topics, vec!["rust".to_string(), "async".to_string()]);
        assert!(entities.is_empty());
        let remaining = remaining.expect("custom frontmatter should remain");
        assert!(remaining.contains("custom: keep"));
        assert!(!remaining.contains("topics:"));
    }

    #[test]
    fn test_extract_managed_from_frontmatter_quoted_strings() {
        let fm_body = "topics: [\"rust lang\", 'async runtime']\nentities:\n  - \"PowerSync\"\n";
        let (remaining, title, topics, entities) = extract_managed_from_frontmatter(fm_body);

        assert!(title.is_none());
        assert_eq!(
            topics,
            vec!["rust lang".to_string(), "async runtime".to_string()]
        );
        assert_eq!(entities, vec!["PowerSync".to_string()]);
        assert!(remaining.is_none());
    }

    #[test]
    fn test_extract_managed_from_frontmatter_warns_for_multiple_title_values() {
        clear_test_logs();

        let (_remaining, title, topics, entities) =
            extract_managed_from_frontmatter("title: [First, Second]\n");

        assert_eq!(title.as_deref(), Some("First"));
        assert!(topics.is_empty());
        assert!(entities.is_empty());
        assert!(
            test_logs()
                .iter()
                .any(|message| message.contains("title frontmatter sequence has 2 values")),
            "expected warning for multiple title values, got {:?}",
            test_logs()
        );
    }

    #[test]
    fn test_parse_editable_doc_nested_custom_yaml_round_trips() {
        let input = "---\ntopics: [rust, async]\nmetadata:\n  status: draft\n  tags:\n    - local-first\nnested:\n  child:\n    enabled: true\n---\n# Title\n\nBody.\n";
        let doc = parse_editable_doc(input);

        assert_eq!(doc.topics, vec!["rust".to_string(), "async".to_string()]);
        assert!(doc.entities.is_empty());
        let fm = doc
            .unmanaged_frontmatter
            .expect("nested custom YAML should remain");
        assert!(fm.contains("metadata:"));
        assert!(fm.contains("status: draft"));
        assert!(fm.contains("local-first"));
        assert!(fm.contains("nested:"));
        assert!(fm.contains("enabled: true"));
        assert!(!fm.contains("topics:"));
    }

    #[test]
    fn test_parse_editable_doc_managed_only_frontmatter_leaves_no_unmanaged_frontmatter() {
        let input = "---\ntopics: [rust, async]\nentities:\n  - PowerSync\n---\n# Title\n\nBody.\n";
        let doc = parse_editable_doc(input);

        assert_eq!(doc.topics, vec!["rust".to_string(), "async".to_string()]);
        assert_eq!(doc.entities, vec!["PowerSync".to_string()]);
        assert!(doc.unmanaged_frontmatter.is_none());
    }

    #[test]
    fn test_parse_editable_doc_invalid_yaml_frontmatter_is_preserved() {
        let input = "---\ncustom: [unterminated\n---\n# Title\n\nBody.\n";
        let doc = parse_editable_doc(input);

        assert!(doc.topics.is_empty());
        assert!(doc.entities.is_empty());
        assert_eq!(
            doc.unmanaged_frontmatter,
            Some("---\ncustom: [unterminated\n---".to_string())
        );
    }

    #[test]
    fn test_render_frontmatter_invalid_user_frontmatter_is_preserved_without_managed_values() {
        let fm = render_frontmatter(None, &[], &[], Some("---\ncustom: [unterminated\n---"));

        assert_eq!(fm, Some("---\ncustom: [unterminated\n---".to_string()));
    }

    #[test]
    fn test_render_frontmatter_invalid_user_frontmatter_keeps_managed_values_visible() {
        let fm = render_frontmatter(
            None,
            &["rust".to_string()],
            &["PowerSync".to_string()],
            Some("---\ncustom: [unterminated\n---"),
        )
        .expect("frontmatter should render");

        assert!(fm.contains("topics:"));
        assert!(fm.contains("- rust"));
        assert!(fm.contains("entities:"));
        assert!(fm.contains("- PowerSync"));
        assert!(fm.contains("custom: [unterminated"));
    }

    #[test]
    fn test_serialized_yaml_body_error_returns_none() {
        let err = <yaml_serde::Error as serde::ser::Error>::custom("serialize failed");

        let body = serialized_yaml_body(Err(err), "test frontmatter");

        assert!(body.is_none());
    }

    #[test]
    fn test_parse_editable_doc_invalid_frontmatter_keeps_managed_values() {
        let input = "---\ntopics:\n- rust\nentities:\n- PowerSync\ncustom: [unterminated\n---\n# Title\n\nBody.\n";
        let doc = parse_editable_doc(input);

        assert_eq!(doc.topics, vec!["rust".to_string()]);
        assert_eq!(doc.entities, vec!["PowerSync".to_string()]);
        assert_eq!(
            doc.unmanaged_frontmatter,
            Some("---\ncustom: [unterminated\n---".to_string())
        );
    }
}
