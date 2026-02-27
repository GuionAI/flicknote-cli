use clap::Args;
use flicknote_core::db::Database;
use flicknote_core::error::CliError;
use flicknote_core::types::Note;

#[derive(Args)]
pub struct ListArgs {
    /// Search notes by title
    #[arg(long)]
    search: Option<String>,
    /// Filter by type
    #[arg(long, value_parser = ["normal", "voice", "link"])]
    r#type: Option<String>,
    /// Maximum number of results
    #[arg(long, default_value = "20")]
    limit: u32,
    /// Output as JSON
    #[arg(long)]
    json: bool,
}

pub fn run(db: &Database, args: &ListArgs) -> Result<(), CliError> {
    let notes = db.read(|conn| {
        let mut sql = String::from("SELECT * FROM notes WHERE deleted_at IS NULL");
        let mut params_vec: Vec<Box<dyn rusqlite::types::ToSql>> = vec![];

        if let Some(ref t) = args.r#type {
            sql.push_str(" AND type = ?");
            params_vec.push(Box::new(t.clone()));
        }
        if let Some(ref search) = args.search {
            sql.push_str(" AND title LIKE ?");
            params_vec.push(Box::new(format!("%{search}%")));
        }

        sql.push_str(" ORDER BY created_at DESC LIMIT ?");
        params_vec.push(Box::new(args.limit));

        let mut stmt = conn.prepare(&sql)?;
        let param_refs: Vec<&dyn rusqlite::types::ToSql> =
            params_vec.iter().map(|p| p.as_ref()).collect();
        let rows = stmt.query_map(param_refs.as_slice(), Note::from_row)?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(CliError::from)
    })?;

    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&notes).map_err(CliError::Json)?
        );
    } else {
        println!(
            "{:<10} {:<8} {:<14} {:<12} {}",
            "ID", "Type", "Status", "Date", "Title"
        );
        println!("{}", "-".repeat(70));
        for note in &notes {
            let id = &note.id[..8.min(note.id.len())];
            let date = note
                .created_at
                .as_deref()
                .and_then(|d| d.get(..10))
                .unwrap_or("-");
            let title = note.title.as_deref().unwrap_or("(untitled)");
            println!(
                "{:<10} {:<8} {:<14} {:<12} {}",
                id, note.r#type, note.status, date, title
            );
        }
    }

    Ok(())
}
