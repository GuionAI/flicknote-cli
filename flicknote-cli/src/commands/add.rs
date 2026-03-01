use clap::Args;
use flicknote_core::config::Config;
use flicknote_core::db::Database;
use flicknote_core::error::CliError;
use flicknote_core::session;
use rusqlite::params;

#[derive(Args)]
pub(crate) struct AddArgs {
    /// Note content or URL (URLs are auto-detected as link notes)
    value: String,
    /// Assign to project by name (creates project if it doesn't exist)
    #[arg(long)]
    project: Option<String>,
    /// Link to a taskwarrior task by UUID
    #[arg(long)]
    task: Option<String>,
}

pub(crate) fn run(db: &Database, config: &Config, args: &AddArgs) -> Result<(), CliError> {
    let user_id = session::get_user_id(config)?;
    let id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();

    let is_url = args.value.starts_with("http://") || args.value.starts_with("https://");

    let project_id = if let Some(ref name) = args.project {
        Some(resolve_or_create_project(db, &user_id, name)?)
    } else {
        None
    };

    db.write(|conn| {
        if is_url {
            let metadata = serde_json::json!({ "link": { "url": &args.value } }).to_string();
            conn.execute(
                "INSERT INTO notes (id, user_id, type, status, title, metadata, project_id, created_at, updated_at)
                 VALUES (?, ?, 'link', 'source_queued', NULL, ?, ?, ?, ?)",
                params![id, user_id, metadata, project_id, now, now],
            )?;
        } else {
            conn.execute(
                "INSERT INTO notes (id, user_id, type, status, content, project_id, created_at, updated_at)
                 VALUES (?, ?, 'normal', 'ai_queued', ?, ?, ?, ?)",
                params![id, user_id, args.value, project_id, now, now],
            )?;
        }

        if let Some(ref tw_uuid) = args.task {
            let link_id = uuid::Uuid::new_v4().to_string();
            let external_id = serde_json::json!({ "tw": tw_uuid }).to_string();
            let title = if is_url { "Link note" } else { &args.value };
            conn.execute(
                "INSERT INTO note_tasks (id, note_id, user_id, title, external_id, created_at)
                 VALUES (?, ?, ?, ?, ?, ?)",
                params![link_id, id, user_id, title, external_id, now],
            )?;
        }

        Ok(())
    })?;

    println!("Created note {}.", &id[..8]);
    Ok(())
}

pub(crate) fn resolve_or_create_project(
    db: &Database,
    user_id: &str,
    name: &str,
) -> Result<String, CliError> {
    let existing = db.read(|conn| {
        let mut stmt = conn.prepare(
            "SELECT id FROM projects WHERE name = ? AND user_id = ? AND (is_archived = 0 OR is_archived IS NULL) LIMIT 1",
        )?;
        let mut rows = stmt.query(params![name, user_id])?;
        if let Some(row) = rows.next()? {
            Ok(Some(row.get::<_, String>(0)?))
        } else {
            Ok(None)
        }
    })?;

    if let Some(id) = existing {
        return Ok(id);
    }

    let id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();
    db.write(|conn| {
        conn.execute(
            "INSERT INTO projects (id, user_id, name, is_archived, created_at)
             VALUES (?, ?, ?, 0, ?)",
            params![id, user_id, name, now],
        )?;
        Ok(())
    })?;

    println!("Created project \"{name}\".");
    Ok(id)
}
