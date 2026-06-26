use clap::Args;
use flicknote_core::backend::NoteDb;
use flicknote_core::config::Config;
use flicknote_core::error::CliError;

use super::util::display_note_id;

#[derive(Args)]
pub(crate) struct RestoreArgs {
    /// Note ID. Use the numeric short ID shown in list/detail. Full UUIDs are also accepted for compatibility.
    id: String,
}

pub(crate) async fn run(
    db: &dyn NoteDb,
    _config: &Config,
    args: &RestoreArgs,
) -> Result<(), CliError> {
    let now = chrono::Utc::now().to_rfc3339();
    let full_id = db.resolve_archived_note_id(&args.id).await?;

    let old_note = db.find_archived_note(&full_id).await?;
    let display_id = display_note_id(&old_note);
    let mut new_note = old_note.clone();
    new_note.deleted_at = None;
    new_note.updated_at = Some(now.clone());

    db.set_note_deleted_at(&full_id, None, &now).await?;
    println!("Restored note {}.", display_id);
    Ok(())
}
