use clap::Args;
use flicknote_core::config::Config;
use flicknote_core::db::Database;
use flicknote_core::error::CliError;
use rusqlite::params;

use super::util::{get_note_content_optional, read_content_or_stdin, resolve_note_id};

#[derive(Args)]
pub(crate) struct AppendArgs {
    /// Note ID (full UUID or prefix)
    id: String,
    /// Content to append. Reads from stdin if omitted.
    content: Option<String>,
}

pub(crate) fn run(db: &Database, config: &Config, args: &AppendArgs) -> Result<(), CliError> {
    let user_id = flicknote_core::session::get_user_id(config)?;
    let full_id = resolve_note_id(db, &args.id)?;
    let now = chrono::Utc::now().to_rfc3339();

    // Get existing content
    let existing = get_note_content_optional(db, &full_id, &user_id, &args.id)?;

    // Get new content from arg or stdin
    let new_content = read_content_or_stdin(&args.content, false)?;

    // Concatenate: existing + separator + new
    let combined = match existing.as_deref() {
        Some(e) if !e.is_empty() => format!("{e}\n\n{new_content}"),
        _ => new_content,
    };

    // Update -- no status change (do not re-queue for AI)
    db.write(|conn| {
        conn.execute(
            "UPDATE notes SET content = ?, updated_at = ? WHERE id = ? AND user_id = ?",
            params![combined, now, full_id, user_id],
        )?;
        Ok(())
    })?;

    println!("Appended to note {}.", &full_id[..8]);
    Ok(())
}
