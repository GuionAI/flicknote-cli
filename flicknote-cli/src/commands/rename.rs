use clap::Args;
use flicknote_core::config::Config;
use flicknote_core::db::Database;
use flicknote_core::error::CliError;
use rusqlite::params;

use flicknote_core::hooks;

use super::util::{find_section, get_note, get_note_content, resolve_note_id};

#[derive(Args)]
pub(crate) struct RenameArgs {
    /// Note ID (full UUID or prefix)
    id: String,
    /// Section heading to rename (case-insensitive contains match)
    #[arg(short = 's', long = "section")]
    section: String,
    /// New heading text (without # prefix — level is preserved)
    name: String,
}

pub(crate) fn run(db: &Database, config: &Config, args: &RenameArgs) -> Result<(), CliError> {
    let user_id = flicknote_core::session::get_user_id(config)?;
    let full_id = resolve_note_id(db, &args.id)?;
    let now = chrono::Utc::now().to_rfc3339();

    let content = get_note_content(db, &full_id, &user_id, &args.id)?;
    let doc = crate::markdown::parse_markdown(&content);
    let bounds = find_section(&doc, &args.section, &args.id)?;

    // Find end of heading line (just the line, not the whole section)
    let heading_line_end = content[bounds.start..]
        .find('\n')
        .map(|i| bounds.start + i)
        .unwrap_or(content.len());

    // Build new heading line preserving level
    let prefix = "#".repeat(bounds.heading.level);
    let new_heading_line = format!("{prefix} {}", args.name);

    // Splice: before heading + new heading line + everything after heading line
    let before = &content[..bounds.start];
    let after = &content[heading_line_end..];
    let new_content = format!("{before}{new_heading_line}{after}");

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
        "rename",
        &config_dir,
    )?;

    db.write(|conn| {
        conn.execute(
            "UPDATE notes SET content = ?, status = 'ai_queued', updated_at = ? WHERE id = ? AND user_id = ?",
            params![new_content.trim(), now, full_id, user_id],
        )?;
        Ok(())
    })?;

    println!(
        "Renamed '{}' → '{}' in note {}.",
        bounds.heading.text,
        args.name,
        &full_id[..8]
    );
    Ok(())
}
