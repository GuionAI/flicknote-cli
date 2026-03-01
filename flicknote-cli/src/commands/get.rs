use clap::Args;
use flicknote_core::db::Database;
use flicknote_core::error::CliError;
use flicknote_core::types::Note;

#[derive(Args)]
pub(crate) struct GetArgs {
    /// Note ID (full UUID or short prefix)
    id: String,
    /// Output as JSON
    #[arg(long)]
    json: bool,
}

pub(crate) fn run(db: &Database, args: &GetArgs) -> Result<(), CliError> {
    // Reject LIKE wildcards
    if !args.id.chars().all(|c| c.is_ascii_hexdigit() || c == '-') {
        return Err(CliError::NoteNotFound {
            id: args.id.clone(),
        });
    }

    let note = db.read(|conn| {
        let (sql, param): (&str, Box<dyn rusqlite::types::ToSql>) = if args.id.len() < 36 {
            (
                "SELECT * FROM notes WHERE id LIKE ? AND deleted_at IS NULL LIMIT 1",
                Box::new(format!("{}%", args.id)),
            )
        } else {
            (
                "SELECT * FROM notes WHERE id = ? AND deleted_at IS NULL LIMIT 1",
                Box::new(args.id.clone()),
            )
        };

        let mut stmt = conn.prepare(sql)?;
        stmt.query_row([param.as_ref()], Note::from_row)
            .map_err(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => CliError::NoteNotFound {
                    id: args.id.clone(),
                },
                other => CliError::from(other),
            })
    })?;

    let project_name: Option<String> = if let Some(ref pid) = note.project_id {
        db.read(|conn| {
            match conn
                .prepare("SELECT name FROM projects WHERE id = ? LIMIT 1")?
                .query_row(rusqlite::params![pid], |row| row.get(0))
            {
                Ok(name) => Ok(Some(name)),
                Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
                Err(e) => Err(e.into()),
            }
        })?
    } else {
        None
    };

    if args.json {
        let json_output = serde_json::json!({
            "id": note.id,
            "title": note.title,
            "project": project_name,
            "summary": note.summary,
            "content": note.content,
        });
        println!(
            "{}",
            serde_json::to_string_pretty(&json_output).map_err(CliError::Json)?
        );
    } else {
        println!(
            "Title:      {}",
            note.title.as_deref().unwrap_or("(untitled)")
        );
        println!("ID:         {}", note.id);
        println!("Type:       {}", note.r#type);
        println!("Status:     {}", note.status);
        println!("Created:    {}", note.created_at.as_deref().unwrap_or("-"));
        println!("Updated:    {}", note.updated_at.as_deref().unwrap_or("-"));
        if let Some(ref name) = project_name {
            println!("Project:    {name}");
        }
        if let Some(ref source) = note.source {
            println!("Source:     {source}");
        }
        if note.is_flagged == Some(1) {
            println!("Flagged:    yes");
        }
        if let Some(ref summary) = note.summary {
            println!("\n── Summary ──\n{summary}");
        }
        if let Some(ref content) = note.content {
            println!("\n── Content ──\n{content}");
        }
        if let Some(url) = note.link_url() {
            println!("Link:       {url}");
        }
    }

    Ok(())
}
