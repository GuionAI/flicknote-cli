use clap::Args;
use flicknote_core::backend::NoteDb;
use flicknote_core::error::CliError;

#[derive(Args)]
pub(crate) struct UnarchiveArgs {
    /// Note ID (full UUID or prefix)
    id: String,
}

pub(crate) fn run(db: &dyn NoteDb, args: &UnarchiveArgs) -> Result<(), CliError> {
    let full_id = db.resolve_archived_note_id(&args.id)?;
    db.set_note_deleted_at(&full_id, None)?;
    println!("Unarchived note {}.", &full_id[..8]);
    Ok(())
}
