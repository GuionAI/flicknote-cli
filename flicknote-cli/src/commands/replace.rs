use clap::Args;
use flicknote_core::config::Config;
use flicknote_core::db::Database;
use flicknote_core::error::CliError;
use rusqlite::params;

use super::util::{read_content_or_stdin, resolve_note_id};

#[derive(Args)]
pub(crate) struct ReplaceArgs {
    /// Note ID (full UUID or prefix)
    id: String,
    /// New content. Reads from stdin if omitted.
    content: Option<String>,
}

pub(crate) fn run(db: &Database, config: &Config, args: &ReplaceArgs) -> Result<(), CliError> {
    let user_id = flicknote_core::session::get_user_id(config)?;
    let full_id = resolve_note_id(db, &args.id)?;
    let now = chrono::Utc::now().to_rfc3339();

    // Verify note exists
    db.read(|conn| {
        let exists: bool = conn.query_row(
            "SELECT COUNT(*) > 0 FROM notes WHERE id = ? AND user_id = ? AND deleted_at IS NULL",
            params![full_id, user_id],
            |row| row.get(0),
        )?;
        if !exists {
            return Err(CliError::NoteNotFound {
                id: args.id.clone(),
            });
        }
        Ok(())
    })?;

    // Get content from arg or stdin
    let content = read_content_or_stdin(&args.content, false)?;

    // Full content replacement + re-queue for AI
    db.write(|conn| {
        conn.execute(
            "UPDATE notes SET content = ?, status = 'ai_queued', updated_at = ? WHERE id = ? AND user_id = ?",
            params![content, now, full_id, user_id],
        )?;
        Ok(())
    })?;

    println!("Replaced content for note {}.", &full_id[..8]);
    Ok(())
}
