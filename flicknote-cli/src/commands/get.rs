use clap::Args;
use flicknote_core::config::Config;
use flicknote_core::db::Database;
use flicknote_core::error::CliError;
use flicknote_core::types::Note;

#[derive(Args)]
pub(crate) struct GetArgs {
    /// Note ID (full UUID or short prefix)
    id: String,
    /// Extract a specific section by heading name (case-insensitive contains match)
    #[arg(short = 's', long = "section")]
    section: Option<String>,
    /// Show markdown heading structure
    #[arg(long)]
    tree: bool,
    /// Output as JSON
    #[arg(long)]
    json: bool,
}

pub(crate) fn run(db: &Database, config: &Config, args: &GetArgs) -> Result<(), CliError> {
    // Reject LIKE wildcards
    if !args.id.chars().all(|c| c.is_ascii_hexdigit() || c == '-') {
        return Err(CliError::NoteNotFound {
            id: args.id.clone(),
        });
    }

    // Tree view or section extraction — both need parsed markdown
    if args.tree || args.section.is_some() {
        let user_id = flicknote_core::session::get_user_id(config)?;
        let full_id = super::util::resolve_note_id(db, &args.id)?;
        let content = super::util::get_note_content(db, &full_id, &user_id, &args.id)?;
        let doc = crate::markdown::parse_markdown(&content);

        if args.tree {
            let tree = doc.build_tree();

            if tree.is_empty() {
                println!("(no headings found)");
                return Ok(());
            }

            for (i, node) in tree.iter().enumerate() {
                let is_last = i == tree.len() - 1;
                print!("{}", node.render_box_tree("", is_last));
            }
            return Ok(());
        }

        let section_id = args.section.as_ref().unwrap();
        let bounds = super::util::find_section(&doc, section_id, &args.id)?;
        // Skip the heading line, extract only the body using the pre-computed byte offsets.
        // Avoids a second name-based scan which would return the wrong section for duplicate headings.
        // `.unwrap_or(bounds.end)` is intentional: a heading with no body returns empty string.
        let body_start = content[bounds.start..]
            .find('\n')
            .map(|i| bounds.start + i + 1)
            .unwrap_or(bounds.end);
        let section_content = content[body_start..bounds.end].trim();
        println!("{}", section_content);
        return Ok(());
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
