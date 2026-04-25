use flicknote_core::backend::NoteDb;
use flicknote_core::error::CliError;
use flicknote_core::types::Note;
use std::io::{IsTerminal, Read};

use crate::markdown::Document;

/// Byte-range boundaries of a matched section in a markdown document.
pub(crate) struct SectionBounds<'a> {
    /// The matched heading.
    pub heading: &'a crate::markdown::Heading,
    /// Byte offset where the heading line starts.
    pub start: usize,
    /// Byte offset where the section ends (next same/higher-level heading, or EOF).
    pub end: usize,
}

/// Find a section by 2-3 char section ID (exact match).
///
/// Returns an error with a helpful message if the ID is not found.
pub(crate) fn find_section<'a>(
    doc: &'a Document,
    section_id: &str,
    display_id: &str,
) -> Result<SectionBounds<'a>, CliError> {
    let heading_idx = doc
        .headings
        .iter()
        .position(|h| h.id == section_id)
        .ok_or_else(|| {
            CliError::Other(format!(
                "error: unknown section ID {section_id:?} — run `flicknote get {display_id} --tree` to see current IDs"
            ))
        })?;

    let heading = &doc.headings[heading_idx];
    let start = heading.offset;
    let end = doc
        .headings
        .iter()
        .skip(heading_idx + 1)
        .find(|h| h.level <= heading.level)
        .map(|h| h.offset)
        .unwrap_or(doc.content.len());

    Ok(SectionBounds {
        heading,
        start,
        end,
    })
}

pub(crate) fn resolve_note_id(db: &dyn NoteDb, prefix: &str) -> Result<String, CliError> {
    db.resolve_note_id(prefix)
}

/// Fetch note content from DB, returning `None` for NULL/missing content.
pub(crate) fn get_note_content_optional(
    db: &dyn NoteDb,
    full_id: &str,
) -> Result<Option<String>, CliError> {
    db.find_note_content(full_id)
}

/// Fetch note content from DB. Shared by get --tree, get -s, and replace.
pub(crate) fn get_note_content(db: &dyn NoteDb, full_id: &str) -> Result<String, CliError> {
    db.find_note_content(full_id)?
        .ok_or_else(|| CliError::Other("Note has no content".into()))
}

/// Write updated content to the database.
pub(crate) fn write_content(
    db: &dyn NoteDb,
    full_id: &str,
    new_content: &str,
) -> Result<(), CliError> {
    db.update_note_content(full_id, new_content, true)
}

/// Print notes as a formatted table to stdout.
/// Columns: ID (full uuid) | Type | Title | Project | Topics | Flagged | Created
pub(crate) fn print_notes_table(
    notes: &[Note],
    topics_map: &std::collections::HashMap<String, Vec<String>>,
    project_names: &std::collections::HashMap<String, String>,
) {
    println!(
        "{:<36} {:<8} {:<30} {:<15} {:<20} {:<7} Created",
        "ID", "Type", "Title", "Project", "Topics", "Flagged"
    );
    println!("{}", "-".repeat(130));
    for note in notes {
        let date = note
            .created_at
            .as_deref()
            .and_then(|d| d.get(..10))
            .unwrap_or("-");
        let title = note.title.as_deref().unwrap_or("(untitled)");
        let title: String = if title.chars().count() > 28 {
            let truncated: String = title.chars().take(27).collect();
            format!("{truncated}…")
        } else {
            title.to_string()
        };
        let project = note
            .project_id
            .as_ref()
            .and_then(|pid| project_names.get(pid))
            .map(std::string::String::as_str)
            .unwrap_or("-");
        let project: String = if project.chars().count() > 13 {
            let truncated: String = project.chars().take(12).collect();
            format!("{truncated}…")
        } else {
            project.to_string()
        };
        let topics = topics_map
            .get(&note.id)
            .map(|v| v.join(", "))
            .unwrap_or_default();
        let topics: String = if topics.chars().count() > 18 {
            let truncated: String = topics.chars().take(17).collect();
            format!("{truncated}…")
        } else {
            topics
        };
        let flagged = if note.is_flagged == Some(1) {
            "✓"
        } else {
            ""
        };
        println!(
            "{:<36} {:<8} {:<30} {:<15} {:<20} {:<7} {}",
            note.id, note.r#type, title, project, topics, flagged, date
        );
    }
}

/// Return the effective project name: arg wins, then $FLICKNOTE_PROJECT, then None.
pub(crate) fn resolve_project_arg(arg: &Option<String>) -> Option<String> {
    if arg.is_some() {
        return arg.clone();
    }
    std::env::var("FLICKNOTE_PROJECT")
        .ok()
        .filter(|s| !s.is_empty())
}

/// Read content from stdin. Errors if stdin is a terminal or if the input is empty.
pub(crate) fn read_stdin_required() -> Result<String, CliError> {
    if std::io::stdin().is_terminal() {
        return Err(CliError::Other(
            "No content provided. Pipe content from stdin.".into(),
        ));
    }
    let mut buf = String::new();
    std::io::stdin().read_to_string(&mut buf)?;
    let trimmed = buf.trim_end().to_string();
    if trimmed.is_empty() {
        return Err(CliError::Other("No content provided".into()));
    }
    Ok(trimmed)
}

/// Read optional stdin content. Returns `Ok(None)` when stdin is a terminal or empty.
pub(crate) fn try_read_stdin() -> Result<Option<String>, CliError> {
    if std::io::stdin().is_terminal() {
        return Ok(None);
    }
    let mut buf = String::new();
    std::io::stdin().read_to_string(&mut buf)?;
    Ok(classify_stdin_buf(&buf))
}

/// Classify a freshly-read stdin buffer. Pure helper, testable without a TTY.
pub(crate) fn classify_stdin_buf(buf: &str) -> Option<String> {
    let trimmed = buf.trim_end_matches(|c: char| c.is_ascii_whitespace());
    let trimmed = trimmed.trim_end_matches(' ');
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::markdown::parse_markdown;

    #[test]
    fn test_find_section_by_id_returns_index() {
        let md = "# Root\n\n## Alpha\n\nContent A.\n\n## Beta\n\nContent B.";
        let doc = parse_markdown(md);
        let alpha_id = doc.headings[1].id.clone(); // "## Alpha" is index 1
        let bounds = find_section(&doc, &alpha_id, "test-id").unwrap();
        assert_eq!(bounds.heading.text, "Alpha");
    }

    #[test]
    fn test_find_section_unknown_id_returns_error() {
        let md = "# Root\n\n## Alpha\n\nContent.";
        let doc = parse_markdown(md);
        let result = find_section(&doc, "zz", "test-id");
        assert!(result.is_err(), "unknown ID should return error");
        if let Err(err) = result {
            let msg = format!("{err}");
            assert!(
                msg.contains("zz"),
                "error message should include the unknown ID"
            );
            assert!(
                msg.contains("--tree"),
                "error message should suggest --tree"
            );
        }
    }

    #[test]
    fn test_find_section_name_string_rejected() {
        // Old-style name selector is now an error, not a fallback
        let md = "# Root\n\n## Alpha Section\n\nContent.";
        let doc = parse_markdown(md);
        let result = find_section(&doc, "Alpha", "test-id");
        assert!(result.is_err(), "name string should be rejected");
    }
}
