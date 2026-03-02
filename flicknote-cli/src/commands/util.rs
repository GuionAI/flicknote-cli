use flicknote_core::db::Database;
use flicknote_core::error::CliError;
use rusqlite::params;
use std::io::{IsTerminal, Read};

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

/// Fetch note content from DB. Shared by get --tree, get -s, and edit.
pub(crate) fn get_note_content(
    db: &Database,
    full_id: &str,
    user_id: &str,
    display_id: &str,
) -> Result<String, CliError> {
    db.read(|conn| {
        let mut stmt = conn.prepare(
            "SELECT content FROM notes WHERE id = ? AND user_id = ? AND deleted_at IS NULL",
        )?;
        let mut rows = stmt.query(params![full_id, user_id])?;
        match rows.next()? {
            Some(row) => {
                let content: Option<String> = row.get(0)?;
                content.ok_or_else(|| CliError::Other("Note has no content".into()))
            }
            None => Err(CliError::NoteNotFound {
                id: display_id.to_string(),
            }),
        }
    })
}

/// Read content from an optional arg, falling back to stdin. Returns an error
/// if stdin is a terminal and no value was provided.
/// When `allow_empty` is false, also rejects empty stdin input.
pub(crate) fn read_content_or_stdin(
    content: &Option<String>,
    allow_empty: bool,
) -> Result<String, CliError> {
    if let Some(v) = content {
        return Ok(v.clone());
    }

    if std::io::stdin().is_terminal() {
        return Err(CliError::Other(
            "No content provided. Pass a value or pipe from stdin.".into(),
        ));
    }

    let mut buf = String::new();
    std::io::stdin().read_to_string(&mut buf)?;
    let trimmed = buf.trim_end().to_string();
    if !allow_empty && trimmed.is_empty() {
        return Err(CliError::Other("No content provided".into()));
    }
    Ok(trimmed)
}
