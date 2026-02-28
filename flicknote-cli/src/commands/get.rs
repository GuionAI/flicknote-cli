use clap::Args;
use flicknote_core::db::Database;
use flicknote_core::error::CliError;
use flicknote_core::types::Note;

#[derive(Args)]
pub struct GetArgs {
    /// Note ID (full UUID or short prefix)
    id: String,
    /// Output as JSON
    #[arg(long)]
    json: bool,
}

pub fn run(db: &Database, args: &GetArgs) -> Result<(), CliError> {
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

    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&note).map_err(CliError::Json)?
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
        if let Some(ref pid) = note.project_id {
            println!("Project:    {pid}");
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
