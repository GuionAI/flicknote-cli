use clap::Args;
use flicknote_core::db::Database;
use flicknote_core::error::CliError;
use rusqlite::params;

use super::util::resolve_note_id;

#[derive(Args)]
pub(crate) struct ArchiveArgs {
    /// Note ID (full UUID or prefix)
    id: String,
}

pub(crate) fn run(db: &Database, args: &ArchiveArgs) -> Result<(), CliError> {
    let now = chrono::Utc::now().to_rfc3339();

    let full_id = resolve_note_id(db, &args.id)?;

    db.write(|conn| {
        conn.execute(
            "UPDATE notes SET deleted_at = ?, updated_at = ? WHERE id = ?",
            params![now, now, full_id],
        )?;
        Ok(())
    })?;

    println!("Archived note {}.", &full_id[..8]);
    Ok(())
}
