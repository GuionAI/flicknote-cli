use clap::Args;
use flicknote_core::backend::NoteDb;
use flicknote_core::config::Config;
use flicknote_core::error::CliError;
use flicknote_core::hooks;

#[derive(Args)]
pub(crate) struct RestoreArgs {
    /// Note ID (full UUID or prefix)
    id: String,
}

pub(crate) fn run(db: &dyn NoteDb, config: &Config, args: &RestoreArgs) -> Result<(), CliError> {
    let now = chrono::Utc::now().to_rfc3339();
    let full_id = db.resolve_archived_note_id(&args.id)?;

    let old_note = db.find_archived_note(&full_id)?;
    let mut new_note = old_note.clone();
    new_note.deleted_at = None;
    new_note.updated_at = Some(now.clone());

    let old_json = serde_json::to_string(&old_note)?;
    let new_json = serde_json::to_string(&new_note)?;
    let config_dir = config.paths.config_dir.to_string_lossy();
    hooks::run_on_archive(
        &config.paths.hooks_dir,
        &old_json,
        &new_json,
        "unarchive",
        &config_dir,
    )?;

    db.set_note_deleted_at(&full_id, None, &now)?;
    println!("Restored note {}.", &full_id[..8]);
    Ok(())
}
