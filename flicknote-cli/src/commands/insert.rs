use clap::Args;
use flicknote_core::config::Config;
use flicknote_core::db::Database;
use flicknote_core::error::CliError;
use rusqlite::params;

use flicknote_core::hooks;

use super::util::{find_section, get_note, get_note_content, read_stdin_required, resolve_note_id};

#[derive(Args)]
#[command(group(clap::ArgGroup::new("position").required(true)))]
pub(crate) struct InsertArgs {
    /// Note ID (full UUID or prefix)
    id: String,
    /// Insert before this section
    #[arg(long, group = "position")]
    before: Option<String>,
    /// Insert after this section
    #[arg(long, group = "position")]
    after: Option<String>,
}

pub(crate) fn run(db: &Database, config: &Config, args: &InsertArgs) -> Result<(), CliError> {
    let user_id = flicknote_core::session::get_user_id(config)?;
    let full_id = resolve_note_id(db, &args.id)?;
    let now = chrono::Utc::now().to_rfc3339();

    let (section_name, insert_before) = match (&args.before, &args.after) {
        (Some(s), None) => (s.as_str(), true),
        (None, Some(s)) => (s.as_str(), false),
        _ => {
            return Err(CliError::Other(
                "Exactly one of --before or --after is required.".into(),
            ));
        }
    };

    let content = get_note_content(db, &full_id, &user_id, &args.id)?;
    let doc = crate::markdown::parse_markdown(&content);
    let bounds = find_section(&doc, section_name, &args.id)?;

    let insert_content = read_stdin_required()?;

    let split_point = if insert_before {
        bounds.start
    } else {
        bounds.end
    };
    let before = content[..split_point].trim_end_matches('\n');
    let after = content[split_point..].trim_start_matches('\n');

    let new_content = if before.is_empty() {
        format!("{}\n\n{after}", insert_content.trim_end())
    } else if after.is_empty() {
        format!("{before}\n\n{}", insert_content.trim_end())
    } else {
        format!("{before}\n\n{}\n\n{after}", insert_content.trim_end())
    };

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
        "insert",
        &config_dir,
    )?;

    db.write(|conn| {
        conn.execute(
            "UPDATE notes SET content = ?, status = 'ai_queued', updated_at = ? WHERE id = ? AND user_id = ?",
            params![new_content.trim(), now, full_id, user_id],
        )?;
        Ok(())
    })?;

    let position = if insert_before { "before" } else { "after" };
    println!(
        "Inserted content {position} '{}' in note {}.",
        bounds.heading.text,
        &full_id[..8]
    );
    Ok(())
}
