use clap::Args;
use flicknote_core::db::Database;
use flicknote_core::error::CliError;
use rusqlite::params;

use super::util::resolve_archived_note_id;

#[derive(Args)]
pub(crate) struct UnarchiveArgs {
    /// Note ID (full UUID or prefix)
    id: String,
}

pub(crate) fn run(db: &Database, args: &UnarchiveArgs) -> Result<(), CliError> {
    let now = chrono::Utc::now().to_rfc3339();

    let full_id = resolve_archived_note_id(db, &args.id)?;

    db.write(|conn| {
        conn.execute(
            "UPDATE notes SET deleted_at = NULL, updated_at = ? WHERE id = ?",
            params![now, full_id],
        )?;
        Ok(())
    })?;

    println!("Unarchived note {}.", &full_id[..8]);
    Ok(())
}
