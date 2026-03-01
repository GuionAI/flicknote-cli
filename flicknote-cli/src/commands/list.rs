use clap::Args;
use flicknote_core::db::Database;
use flicknote_core::error::CliError;
use flicknote_core::types::Note;
use rusqlite::params;

#[derive(Args)]
pub(crate) struct ListArgs {
    /// Search notes by title or content
    #[arg(long)]
    search: Option<String>,
    /// Filter by type
    #[arg(long, value_parser = ["normal", "voice", "link"])]
    r#type: Option<String>,
    /// Filter by project name
    #[arg(long)]
    project: Option<String>,
    /// Filter by taskwarrior task UUID
    #[arg(long)]
    task: Option<String>,
    /// Show only archived notes
    #[arg(long)]
    archived: bool,
    /// Maximum number of results
    #[arg(long, default_value = "20")]
    limit: u32,
    /// Output as JSON
    #[arg(long)]
    json: bool,
}

pub(crate) fn run(db: &Database, args: &ListArgs) -> Result<(), CliError> {
    let notes = db.read(|conn| {
        let base_condition = if args.archived {
            "WHERE deleted_at IS NOT NULL"
        } else {
            "WHERE deleted_at IS NULL"
        };
        let mut sql = format!("SELECT * FROM notes {base_condition}");
        let mut params_vec: Vec<Box<dyn rusqlite::types::ToSql>> = vec![];

        if let Some(ref t) = args.r#type {
            sql.push_str(" AND type = ?");
            params_vec.push(Box::new(t.clone()));
        }
        if let Some(ref search) = args.search {
            sql.push_str(" AND (title LIKE ? OR content LIKE ?)");
            let pattern = format!("%{search}%");
            params_vec.push(Box::new(pattern.clone()));
            params_vec.push(Box::new(pattern));
        }
        if let Some(ref project_name) = args.project {
            let project_id: Option<String> = conn
                .prepare(
                    "SELECT id FROM projects WHERE name = ? AND (is_archived = 0 OR is_archived IS NULL) LIMIT 1",
                )?
                .query_row(params![project_name], |row| row.get(0))
                .ok();

            if let Some(pid) = project_id {
                sql.push_str(" AND project_id = ?");
                params_vec.push(Box::new(pid));
            } else {
                eprintln!("Warning: no project found with name \"{project_name}\".");
                return Ok(vec![]);
            }
        }
        if let Some(ref tw_uuid) = args.task {
            sql.push_str(" AND id IN (SELECT note_id FROM note_tasks WHERE json_extract(external_id, '$.tw') = ?)");
            params_vec.push(Box::new(tw_uuid.clone()));
        }

        sql.push_str(" ORDER BY created_at DESC LIMIT ?");
        params_vec.push(Box::new(args.limit));

        let mut stmt = conn.prepare(&sql)?;
        let param_refs: Vec<&dyn rusqlite::types::ToSql> =
            params_vec.iter().map(std::convert::AsRef::as_ref).collect();
        let rows = stmt.query_map(param_refs.as_slice(), Note::from_row)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(CliError::from)
    })?;

    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&notes).map_err(CliError::Json)?
        );
    } else {
        println!(
            "{:<10} {:<8} {:<14} {:<12} Title",
            "ID", "Type", "Status", "Date"
        );
        println!("{}", "-".repeat(70));
        for note in &notes {
            let id = &note.id[..8.min(note.id.len())];
            let date = note
                .created_at
                .as_deref()
                .and_then(|d| d.get(..10))
                .unwrap_or("-");
            let title = note
                .title
                .as_deref()
                .or(note.content.as_deref())
                .unwrap_or("(untitled)");
            let title: String = if title.chars().count() > 60 {
                title.chars().take(60).collect()
            } else {
                title.to_string()
            };
            println!(
                "{:<10} {:<8} {:<14} {:<12} {}",
                id, note.r#type, note.status, date, title
            );
        }
    }

    Ok(())
}
