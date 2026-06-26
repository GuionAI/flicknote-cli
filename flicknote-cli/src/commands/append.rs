use clap::Args;
use flicknote_core::backend::NoteDb;
use flicknote_core::config::Config;
use flicknote_core::error::CliError;

use super::util::{
    display_note_id, get_note_content_optional, read_stdin_required, resolve_note_id,
};

#[derive(Args)]
pub(crate) struct AppendArgs {
    /// Note ID. Use the numeric short ID shown in list/detail. Full UUIDs are also accepted for compatibility.
    id: String,
}

pub(crate) async fn run(
    db: &dyn NoteDb,
    _config: &Config,
    args: &AppendArgs,
) -> Result<(), CliError> {
    let full_id = resolve_note_id(db, &args.id).await?;

    // Get existing content
    let existing = get_note_content_optional(db, &full_id).await?;

    // Get new content from stdin
    let new_content = read_stdin_required()?;

    // Concatenate: existing + separator + new
    let combined = match existing.as_deref() {
        Some(e) if !e.is_empty() => format!("{e}\n\n{new_content}"),
        _ => new_content,
    };

    // Update — no status change (do not re-queue for AI)
    db.update_note_content(&full_id, &combined, false).await?;

    let note = db.find_note(&full_id).await?;
    println!("Appended to note {}.", display_note_id(&note));
    Ok(())
}
