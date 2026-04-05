use clap::Args;
use flicknote_core::backend::NoteDb;
use flicknote_core::config::Config;
use flicknote_core::error::CliError;

#[derive(Args)]
pub(crate) struct DetailArgs {
    /// Note ID (full UUID or short prefix)
    id: String,
    /// Extract a specific section by section ID (2-char base62)
    #[arg(short = 's', long = "section")]
    section: Option<String>,
    /// Show markdown heading structure
    #[arg(long)]
    tree: bool,
    /// Output as JSON
    #[arg(long)]
    json: bool,
    /// Read an archived note (looks up from archived notes instead of active)
    #[arg(long)]
    archived: bool,
}

pub(crate) fn run(db: &dyn NoteDb, _config: &Config, args: &DetailArgs) -> Result<(), CliError> {
    if !args.id.chars().all(|c| c.is_ascii_hexdigit() || c == '-') {
        return Err(CliError::NoteNotFound {
            id: args.id.clone(),
        });
    }

    let resolve = |id: &str| {
        if args.archived {
            db.resolve_archived_note_id(id)
        } else {
            db.resolve_note_id(id)
        }
    };
    let find = |id: &str| {
        if args.archived {
            db.find_archived_note(id)
        } else {
            db.find_note(id)
        }
    };

    // Tree view or section extraction — both need parsed markdown
    if args.tree || args.section.is_some() {
        let full_id = resolve(&args.id)?;
        let note = find(&full_id)?;
        let content = note.content.as_deref().unwrap_or("");

        if args.tree {
            let display_content = if let Some(ref t) = note.title {
                format!("# {t}\n\n{content}")
            } else {
                content.to_string()
            };
            let doc = crate::markdown::parse_markdown(&display_content);
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

        // --section: operates on raw stored content
        if note.content.is_none() {
            return Err(CliError::Other(
                "This note has no text content (link or file note)".into(),
            ));
        }
        let doc = crate::markdown::parse_markdown(content);
        let section_id = args.section.as_ref().unwrap();
        let bounds = super::util::find_section(&doc, section_id, &args.id)?;
        let body_start = content[bounds.start..]
            .find('\n')
            .map(|i| bounds.start + i + 1)
            .unwrap_or(bounds.end);
        let section_content = content[body_start..bounds.end].trim();
        println!("{}", section_content);
        return Ok(());
    }

    let full_id = resolve(&args.id)?;
    let note = find(&full_id)?;

    let project_name = if let Some(ref pid) = note.project_id {
        db.find_project_name_by_id(pid)?
    } else {
        None
    };

    if args.json {
        let json_output = serde_json::json!({
            "id": note.id,
            "type": note.r#type,
            "title": note.title,
            "project": project_name,
            "project_id": note.project_id,
            "summary": note.summary,
            "content": note.content,
            "is_flagged": note.is_flagged,
            "created_at": note.created_at,
            "updated_at": note.updated_at,
        });
        println!(
            "{}",
            serde_json::to_string_pretty(&json_output).map_err(CliError::Json)?
        );
    } else {
        println!("ID:         {}", note.id);
        println!("Type:       {}", note.r#type);
        println!(
            "Title:      {}",
            note.title.as_deref().unwrap_or("(untitled)")
        );
        if let Some(ref summary) = note.summary {
            println!("Summary:    {summary}");
        }
        if let Some(ref pid) = note.project_id {
            let name = project_name.as_deref().unwrap_or("(unknown)");
            println!("Project:    {name} ({pid})");
        }
        if note.is_flagged == Some(1) {
            println!("Flagged:    yes");
        }
        println!("Created:    {}", note.created_at.as_deref().unwrap_or("-"));
        println!("Updated:    {}", note.updated_at.as_deref().unwrap_or("-"));
        if let Some(ref content) = note.content {
            println!("\nContent:");
            // Synthesize H1 from title for display, then render with IDs
            let display_content = if let Some(ref t) = note.title {
                format!("# {t}\n\n{content}")
            } else {
                content.clone()
            };
            println!(
                "{}",
                crate::markdown::render_content_with_ids(&display_content)
            );
        }
        if let Some(url) = note.link_url() {
            println!("Link:       {url}");
        }
    }

    Ok(())
}
