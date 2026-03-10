use clap::Args;
use flicknote_core::backend::NoteDb;
use flicknote_core::error::CliError;

#[derive(Args)]
pub(crate) struct ArchiveArgs {
    /// Note ID (full UUID or prefix)
    id: String,
}

pub(crate) fn run(db: &dyn NoteDb, args: &ArchiveArgs) -> Result<(), CliError> {
    let now = chrono::Utc::now().to_rfc3339();
    let full_id = db.resolve_note_id(&args.id)?;
    db.set_note_deleted_at(&full_id, Some(&now))?;
    println!("Archived note {}.", &full_id[..8]);
    Ok(())
}
