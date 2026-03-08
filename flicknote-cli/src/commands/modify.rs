use clap::Args;
use flicknote_core::config::Config;
use flicknote_core::db::Database;
use flicknote_core::error::CliError;
use flicknote_core::hooks;
use rusqlite::params;

use super::add::resolve_or_create_project;
use super::util::{get_note, resolve_note_id};

#[derive(Args)]
pub(crate) struct ModifyArgs {
    /// Note ID (full UUID or prefix)
    id: String,
    /// Move note to this project (creates project if it doesn't exist)
    #[arg(short = 'p', long = "project")]
    project: Option<String>,
}

pub(crate) fn run(db: &Database, config: &Config, args: &ModifyArgs) -> Result<(), CliError> {
    let user_id = flicknote_core::session::get_user_id(config)?;
    let full_id = resolve_note_id(db, &args.id)?;
    let now = chrono::Utc::now().to_rfc3339();

    let old_note = get_note(db, &full_id, &user_id)?;

    let Some(ref project_name) = args.project else {
        return Err(CliError::Other(
            "Nothing to modify. Use --project <name> to change the note's project.".into(),
        ));
    };

    let old_project_id = old_note.project_id.clone();
    let new_project_id = resolve_or_create_project(db, &user_id, project_name)?;

    // No-op if already in same project
    if old_project_id.as_deref() == Some(new_project_id.as_str()) {
        println!(
            "Note {} is already in project \"{}\".",
            &full_id[..8],
            project_name
        );
        return Ok(());
    }

    // Run on-modify hook before writing
    let mut new_note = old_note.clone();
    new_note.project_id = Some(new_project_id.clone());
    new_note.updated_at = Some(now.clone());

    let old_json = serde_json::to_string(&old_note)?;
    let new_json = serde_json::to_string(&new_note)?;
    let config_dir = config.paths.config_dir.to_string_lossy();
    hooks::run_on_modify(
        &config.paths.hooks_dir,
        &old_json,
        &new_json,
        "modify",
        &config_dir,
    )?;

    // Fetch old project name (for deletion message) before write
    let old_project_name: Option<String> = if let Some(ref old_pid) = old_project_id {
        db.read(|conn| {
            let mut stmt = conn.prepare("SELECT name FROM projects WHERE id = ?")?;
            let mut rows = stmt.query(params![old_pid])?;
            if let Some(row) = rows.next()? {
                Ok(Some(row.get::<_, String>(0)?))
            } else {
                Ok(None)
            }
        })?
    } else {
        None
    };

    // Update note project + conditionally delete empty old project
    let deleted_old = db.write(|conn| {
        let affected = conn.execute(
            "UPDATE notes SET project_id = ?, updated_at = ? WHERE id = ? AND user_id = ?",
            params![new_project_id, now, full_id, user_id],
        )?;
        if affected == 0 {
            return Err(CliError::NoteNotFound {
                id: full_id.clone(),
            });
        }

        if let Some(ref old_pid) = old_project_id {
            let mut stmt = conn.prepare(
                "SELECT COUNT(*) FROM notes WHERE project_id = ? AND deleted_at IS NULL",
            )?;
            let mut rows = stmt.query(params![old_pid])?;
            let count: i64 = rows
                .next()?
                .ok_or_else(|| CliError::Other("COUNT(*) returned no rows".into()))?
                .get::<_, i64>(0)?;

            if count == 0 {
                conn.execute("DELETE FROM projects WHERE id = ?", params![old_pid])?;
                return Ok(true);
            }
        }
        Ok(false)
    })?;

    println!(
        "Moved note {} to project \"{}\".",
        &full_id[..8],
        project_name
    );

    if deleted_old && let Some(name) = old_project_name {
        println!("Deleted empty project \"{}\".", name);
    }

    Ok(())
}
