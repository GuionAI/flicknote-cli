use flicknote_core::db::Database;
use flicknote_core::error::CliError;
use flicknote_core::types::Note;
use rusqlite::params;
use std::io::{IsTerminal, Read};

use crate::markdown::{Document, Heading};

/// Byte-range boundaries of a matched section in a markdown document.
pub(crate) struct SectionBounds<'a> {
    /// The matched heading.
    pub heading: &'a Heading,
    /// Byte offset where the heading line starts.
    pub start: usize,
    /// Byte offset where the section ends (next same/higher-level heading, or EOF).
    pub end: usize,
}

/// Find a section by heading name in a parsed document.
///
/// Uses case-insensitive contains matching. Errors if zero or multiple matches.
pub(crate) fn find_section<'a>(
    doc: &'a Document,
    section: &str,
    display_id: &str,
) -> Result<SectionBounds<'a>, CliError> {
    let matches = doc.filter_headings(section);
    let heading = match matches.len() {
        0 => {
            return Err(CliError::Other(format!(
                "Section '{section}' not found. Use `flicknote get {display_id} --tree` to see structure."
            )));
        }
        1 => matches[0],
        _ => {
            let names: Vec<_> = matches.iter().map(|h| format!("  - {}", h.text)).collect();
            return Err(CliError::Other(format!(
                "'{section}' matches {} headings — be more specific:\n{}",
                matches.len(),
                names.join("\n")
            )));
        }
    };

    let heading_idx = doc
        .headings
        .iter()
        .position(|h| h.text == heading.text && h.offset == heading.offset)
        .unwrap();

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
