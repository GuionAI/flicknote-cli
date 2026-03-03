use clap::Args;
use flicknote_core::config::Config;
use flicknote_core::db::Database;
use flicknote_core::error::CliError;
use rusqlite::params;

use flicknote_core::hooks;

use super::util::{
    find_section, get_note, get_note_content, read_content_or_stdin, resolve_note_id,
};

#[derive(Args)]
pub(crate) struct EditArgs {
    /// Note ID (full UUID or prefix)
    id: String,
    /// Section heading to replace (case-insensitive contains match)
    #[arg(short = 's', long = "section")]
    section: String,
    /// New content for the section. Reads from stdin if omitted.
    content: Option<String>,
}

pub(crate) fn run(db: &Database, config: &Config, args: &EditArgs) -> Result<(), CliError> {
    let user_id = flicknote_core::session::get_user_id(config)?;
    let full_id = resolve_note_id(db, &args.id)?;
    let now = chrono::Utc::now().to_rfc3339();

    let content = get_note_content(db, &full_id, &user_id, &args.id)?;
    let doc = crate::markdown::parse_markdown(&content);

    let bounds = find_section(&doc, &args.section, &args.id)?;
    let heading = bounds.heading;
    let start = bounds.start;
    let end = bounds.end;

    // Get replacement content from arg or stdin (empty = section removal)
    let new_section = read_content_or_stdin(&args.content, true)?;

    // Build new content: before + replacement + after
    let before = &content[..start];
    let after = &content[end..];
    let new_content = if new_section.is_empty() {
        format!(
            "{}{}",
            before.trim_end_matches('\n'),
            if after.is_empty() {
                "".to_string()
            } else {
                format!("\n\n{}", after.trim_start_matches('\n'))
            }
        )
    } else {
        format!("{}{}\n\n{}", before, new_section.trim_end(), after)
    };

    // Notify on-modify hook (may reject)
    let old_note = get_note(db, &full_id, &user_id)?;
    let mut new_note = old_note.clone();
    new_note.content = Some(new_content.trim().to_string());
    new_note.status = "ai_queued".to_string();
    new_note.updated_at = Some(now.clone());

    let old_json = serde_json::to_string(&old_note)?;
    let new_json = serde_json::to_string(&new_note)?;
    let config_dir = config.paths.config_dir.to_string_lossy();
    hooks::run_on_modify(
        &config.paths.hooks_dir,
        &old_json,
        &new_json,
        "edit",
        &config_dir,
    )?;

    // Write back + re-queue for AI
    db.write(|conn| {
        conn.execute(
            "UPDATE notes SET content = ?, status = 'ai_queued', updated_at = ? WHERE id = ? AND user_id = ?",
            params![new_content.trim(), now, full_id, user_id],
        )?;
        Ok(())
    })?;

    if new_section.is_empty() {
        println!(
            "Removed section '{}' from note {}.",
            heading.text,
            &full_id[..8]
        );
    } else {
        println!(
            "Edited section '{}' in note {}.",
            heading.text,
            &full_id[..8]
        );
    }
    Ok(())
}
