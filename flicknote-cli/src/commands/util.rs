use flicknote_core::db::Database;
use flicknote_core::error::CliError;
use rusqlite::params;

pub(crate) fn resolve_note_id(db: &Database, prefix: &str) -> Result<String, CliError> {
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
