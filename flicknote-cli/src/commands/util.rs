use flicknote_core::db::Database;
use flicknote_core::error::CliError;
use flicknote_core::types::Note;
use rusqlite::params;
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

pub(crate) fn resolve_note_id(db: &Database, prefix: &str) -> Result<String, CliError> {
    // Reject LIKE wildcards — only hex digits and dashes are valid UUID characters
    if !prefix.chars().all(|c| c.is_ascii_hexdigit() || c == '-') {
        return Err(CliError::NoteNotFound {
            id: prefix.to_string(),
        });
    }

    db.read(|conn| {
        let mut stmt =
            conn.prepare("SELECT id FROM notes WHERE id LIKE ? AND deleted_at IS NULL LIMIT 2")?;
        let mut rows = stmt.query(params![format!("{prefix}%")])?;
        let first = rows.next()?.map(|r| r.get::<_, String>(0)).transpose()?;
        let second = rows.next()?.is_some();

        match (first, second) {
            (Some(_), true) => Err(CliError::Other(format!("Ambiguous ID prefix: {prefix}"))),
            (Some(id), false) => Ok(id),
            (None, _) => Err(CliError::NoteNotFound {
                id: prefix.to_string(),
            }),
        }
    })
}

pub(crate) fn resolve_archived_note_id(db: &Database, prefix: &str) -> Result<String, CliError> {
    if !prefix.chars().all(|c| c.is_ascii_hexdigit() || c == '-') {
        return Err(CliError::NoteNotFound {
            id: prefix.to_string(),
        });
    }

    db.read(|conn| {
        let mut stmt = conn
            .prepare("SELECT id FROM notes WHERE id LIKE ? AND deleted_at IS NOT NULL LIMIT 2")?;
        let mut rows = stmt.query(params![format!("{prefix}%")])?;
        let first = rows.next()?.map(|r| r.get::<_, String>(0)).transpose()?;
        let second = rows.next()?.is_some();

        match (first, second) {
            (Some(_), true) => Err(CliError::Other(format!("Ambiguous ID prefix: {prefix}"))),
            (Some(id), false) => Ok(id),
            (None, _) => Err(CliError::NoteNotFound {
                id: prefix.to_string(),
            }),
        }
    })
}

/// Fetch note content from DB, returning `None` for NULL/missing content.
/// Shared by `get_note_content` and the append command.
pub(crate) fn get_note_content_optional(
    db: &Database,
    full_id: &str,
    user_id: &str,
    display_id: &str,
) -> Result<Option<String>, CliError> {
    db.read(|conn| {
        let mut stmt = conn.prepare(
            "SELECT content FROM notes WHERE id = ? AND user_id = ? AND deleted_at IS NULL",
        )?;
        let mut rows = stmt.query(params![full_id, user_id])?;
        match rows.next()? {
            Some(row) => Ok(row.get::<_, Option<String>>(0)?),
            None => Err(CliError::NoteNotFound {
                id: display_id.to_string(),
            }),
        }
    })
}

/// Fetch note content from DB. Shared by get --tree, get -s, and replace.
pub(crate) fn get_note_content(
    db: &Database,
    full_id: &str,
    user_id: &str,
    display_id: &str,
) -> Result<String, CliError> {
    get_note_content_optional(db, full_id, user_id, display_id)?
        .ok_or_else(|| CliError::Other("Note has no content".into()))
}

/// Fetch a full Note by ID. Returns error if not found or deleted.
pub(crate) fn get_note(db: &Database, full_id: &str, user_id: &str) -> Result<Note, CliError> {
    db.read(|conn| {
        let mut stmt = conn.prepare(
            "SELECT id, user_id, type, status, title, content, summary, is_flagged, \
             project_id, metadata, source, external_id, created_at, updated_at, deleted_at \
             FROM notes WHERE id = ? AND user_id = ? AND deleted_at IS NULL",
        )?;
        let mut rows = stmt.query(params![full_id, user_id])?;
        match rows.next()? {
            Some(row) => Ok(Note::from_row(row)?),
            None => Err(CliError::NoteNotFound {
                id: full_id.to_string(),
            }),
        }
    })
}

/// Print notes as a formatted table to stdout.
pub(crate) fn print_notes_table(notes: &[Note]) {
    println!(
        "{:<10} {:<8} {:<14} {:<12} Title",
        "ID", "Type", "Status", "Date"
    );
    println!("{}", "-".repeat(70));
    for note in notes {
        let id = &note.id[..8.min(note.id.len())];
        let date = note
            .created_at
            .as_deref()
            .and_then(|d| d.get(..10))
            .unwrap_or("-");
        let title = note
            .title
            .as_deref()
            .or(note.content.as_deref())
            .unwrap_or("(untitled)");
        let title: String = if title.chars().count() > 60 {
            title.chars().take(60).collect()
        } else {
            title.to_string()
        };
        println!(
            "{:<10} {:<8} {:<14} {:<12} {}",
            id, note.r#type, note.status, date, title
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
