use clap::Args;
use flicknote_core::config::Config;
use flicknote_core::db::Database;
use flicknote_core::error::CliError;
use flicknote_core::session;
use rusqlite::params;

use super::util::resolve_note_id;

#[derive(Args)]
pub(crate) struct LinkArgs {
    /// Note ID (full UUID or prefix)
    id: String,
    /// Taskwarrior task UUID to link
    #[arg(long)]
    task: String,
}

pub(crate) fn run(db: &Database, config: &Config, args: &LinkArgs) -> Result<(), CliError> {
    let user_id = session::get_user_id(config)?;
    let now = chrono::Utc::now().to_rfc3339();

    let full_id = resolve_note_id(db, &args.id)?;

    let exists = db.read(|conn| {
        let mut stmt = conn.prepare(
            "SELECT COUNT(*) FROM note_tasks WHERE note_id = ? AND json_extract(external_id, '$.tw') = ?",
        )?;
        let count: i64 = stmt.query_row(params![full_id, args.task], |row| row.get(0))?;
        Ok(count > 0)
    })?;

    if exists {
        println!(
            "Note {} already linked to task {}.",
            &full_id[..8],
            &args.task[..8.min(args.task.len())]
        );
        return Ok(());
    }

    let title = db.read(|conn| {
        let mut stmt = conn.prepare("SELECT title FROM notes WHERE id = ?")?;
        let title: Option<String> = stmt.query_row(params![full_id], |row| row.get(0))?;
        Ok(title.unwrap_or_else(|| "Linked note".to_string()))
    })?;

    let link_id = uuid::Uuid::new_v4().to_string();
    let external_id = serde_json::json!({ "tw": &args.task }).to_string();
    db.write(|conn| {
        conn.execute(
            "INSERT INTO note_tasks (id, note_id, user_id, title, external_id, created_at)
             VALUES (?, ?, ?, ?, ?, ?)",
            params![link_id, full_id, user_id, title, external_id, now],
        )?;
        Ok(())
    })?;

    println!(
        "Linked note {} to task {}.",
        &full_id[..8],
        &args.task[..8.min(args.task.len())]
    );
    Ok(())
}
