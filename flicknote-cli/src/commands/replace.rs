use clap::Args;
use flicknote_core::config::Config;
use flicknote_core::db::Database;
use flicknote_core::error::CliError;
use rusqlite::params;

use flicknote_core::hooks;

use super::util::{find_section, get_note, get_note_content, read_stdin_required, resolve_note_id};

#[derive(Args)]
pub(crate) struct ReplaceArgs {
    /// Note ID (full UUID or prefix)
    id: String,
    /// Replace only the named section (case-insensitive contains match)
    #[arg(short = 's', long = "section")]
    section: Option<String>,
}

/// Run the on-modify hook and write updated content to the database.
fn write_note(
    db: &Database,
    config: &Config,
    full_id: &str,
    user_id: &str,
    new_content: &str,
    now: &str,
) -> Result<(), CliError> {
    let old_note = get_note(db, full_id, user_id)?;
    let mut new_note = old_note.clone();
    new_note.content = Some(new_content.to_string());
    new_note.status = "ai_queued".to_string();
    new_note.updated_at = Some(now.to_string());

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

    db.write(|conn| {
        conn.execute(
            "UPDATE notes SET content = ?, status = 'ai_queued', updated_at = ? WHERE id = ? AND user_id = ?",
            params![new_content, now, full_id, user_id],
        )?;
        Ok(())
    })
}

pub(crate) fn run(db: &Database, config: &Config, args: &ReplaceArgs) -> Result<(), CliError> {
    let user_id = flicknote_core::session::get_user_id(config)?;
    let full_id = resolve_note_id(db, &args.id)?;
    let now = chrono::Utc::now().to_rfc3339();

    if let Some(section_id) = &args.section {
        // Section-level replace (formerly `edit`). Use `flicknote remove` to delete a section.
        let content = get_note_content(db, &full_id, &user_id, &args.id)?;
        let doc = crate::markdown::parse_markdown(&content);
        let bounds = find_section(&doc, section_id, &args.id)?;
        let heading_level = bounds.heading.level;
        let start = bounds.start;
        let end = bounds.end;

        let new_body = read_stdin_required()?;

        // Shift entire piped content so its root heading matches section_level.
        // cap_heading_level finds the shallowest heading and shifts all headings relatively.
        let shifted = crate::markdown::cap_heading_level(new_body.trim(), heading_level);
        let new_content = crate::markdown::replace_entire_section(&content, start, end, &shifted);

        write_note(db, config, &full_id, &user_id, new_content.trim(), &now)?;
        println!("Replaced section in note {}.\n", &full_id[..8]);
        print!("{}", crate::markdown::render_tree(new_content.trim()));
    } else {
        let content = read_stdin_required()?;
        write_note(db, config, &full_id, &user_id, &content, &now)?;
        println!("Replaced content for note {}.\n", &full_id[..8]);
        print!("{}", crate::markdown::render_tree(&content));
    }

    Ok(())
}
