use clap::Args;
use flicknote_core::backend::NoteDb;
use flicknote_core::config::Config;
use flicknote_core::error::CliError;
#[derive(Args)]
pub(crate) struct DetailArgs {
    /// Note ID (full UUID or short prefix)
    id: String,
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
pub(crate) async fn run(
    db: &dyn NoteDb,
    _config: &Config,
    args: &DetailArgs,
) -> Result<(), CliError> {
    if !args.id.chars().all(|c| c.is_ascii_hexdigit() || c == '-') {
        return Err(CliError::NoteNotFound {
            id: args.id.clone(),
        });
    }
    if args.tree {
        let full_id = if args.archived {
            db.resolve_archived_note_id(&args.id).await?
        } else {
            db.resolve_note_id(&args.id).await?
        };
        let note = if args.archived {
            db.find_archived_note(&full_id).await?
        } else {
            db.find_note(&full_id).await?
        };
        let display_content = crate::editable_document::render_editable_note(db, &note).await?;
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
    let full_id = if args.archived {
        db.resolve_archived_note_id(&args.id).await?
    } else {
        db.resolve_note_id(&args.id).await?
    };
    let note = if args.archived {
        db.find_archived_note(&full_id).await?
    } else {
        db.find_note(&full_id).await?
    };
    let project_name = if let Some(ref pid) = note.project_id {
        db.find_project_name_by_id(pid).await?
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
        if let Some(ref _content) = note.content {
            println!("\nContent:");
            let display_content = crate::editable_document::render_editable_note(db, &note).await?;
            println!("{display_content}");
        }
        if let Some(url) = note.link_url() {
            println!("Link:       {url}");
        }
    }
    Ok(())
}
