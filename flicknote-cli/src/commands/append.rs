use clap::Args;
use flicknote_core::backend::NoteDb;
use flicknote_core::config::Config;
use flicknote_core::error::CliError;

use flicknote_core::hooks;

use super::util::{get_note, get_note_content_optional, read_stdin_required, resolve_note_id};

#[derive(Args)]
pub(crate) struct AppendArgs {
    /// Note ID (full UUID or prefix)
    id: String,
}

pub(crate) fn run(db: &dyn NoteDb, config: &Config, args: &AppendArgs) -> Result<(), CliError> {
    let full_id = resolve_note_id(db, &args.id)?;
    let now = chrono::Utc::now().to_rfc3339();

    // Get existing content
    let existing = get_note_content_optional(db, &full_id)?;

    // Get new content from stdin
    let new_content = read_stdin_required()?;

    // Concatenate: existing + separator + new
    let combined = match existing.as_deref() {
        Some(e) if !e.is_empty() => format!("{e}\n\n{new_content}"),
        _ => new_content,
    };

    // Notify on-modify hook (may reject)
    let old_note = get_note(db, &full_id)?;
    let mut new_note = old_note.clone();
    new_note.content = Some(combined.clone());
    new_note.updated_at = Some(now.clone());

    let old_json = serde_json::to_string(&old_note)?;
    let new_json = serde_json::to_string(&new_note)?;
    let config_dir = config.paths.config_dir.to_string_lossy();
    hooks::run_on_modify(
        &config.paths.hooks_dir,
        &old_json,
        &new_json,
        "append",
        &config_dir,
    )?;

    // Update — no status change (do not re-queue for AI)
    db.update_note_content(&full_id, &combined, false)?;

    println!("Appended to note {}.", &full_id[..8]);
    Ok(())
}
