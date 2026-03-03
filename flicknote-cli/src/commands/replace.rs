use clap::Args;
use flicknote_core::config::Config;
use flicknote_core::db::Database;
use flicknote_core::error::CliError;
use rusqlite::params;

use flicknote_core::hooks;

use super::util::{get_note, read_content_or_stdin, resolve_note_id};

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

    // Get content from arg or stdin
    let content = read_content_or_stdin(&args.content, false)?;

    // Notify on-modify hook (may reject)
    let old_note = get_note(db, &full_id, &user_id)?;
    let mut new_note = old_note.clone();
    new_note.content = Some(content.clone());
    new_note.status = "ai_queued".to_string();
    new_note.updated_at = Some(now.clone());

    let old_json = serde_json::to_string(&old_note)?;
    let new_json = serde_json::to_string(&new_note)?;
    let config_dir = config.paths.config_dir.to_string_lossy();
    hooks::run_on_modify(
        &config.paths.hooks_dir,
        &old_json,
        &new_json,
        "replace",
        &config_dir,
    )?;

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
