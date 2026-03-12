use clap::Args;
use flicknote_core::backend::NoteDb;
use flicknote_core::config::Config;
use flicknote_core::error::CliError;
use flicknote_core::hooks;

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

pub(crate) fn run(db: &dyn NoteDb, config: &Config, args: &GetArgs) -> Result<(), CliError> {
    // Reject LIKE wildcards
    if !args.id.chars().all(|c| c.is_ascii_hexdigit() || c == '-') {
        return Err(CliError::NoteNotFound {
            id: args.id.clone(),
        });
    }

    // Tree view or section extraction — both need parsed markdown
    if args.tree || args.section.is_some() {
        let full_id = db.resolve_note_id(&args.id)?;
        let content = db
            .find_note_content(&full_id)?
            .ok_or_else(|| CliError::Other("Note has no content".into()))?;
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
        let body_start = content[bounds.start..]
            .find('\n')
            .map(|i| bounds.start + i + 1)
            .unwrap_or(bounds.end);
        let section_content = content[body_start..bounds.end].trim();
        println!("{}", section_content);
        return Ok(());
    }

    let full_id = db.resolve_note_id(&args.id)?;
    let note = db.find_note(&full_id)?;

    let note_json = serde_json::to_string(&note)?;
    let config_dir = config.paths.config_dir.to_string_lossy();
    hooks::run_on_get(&config.paths.hooks_dir, &note_json, &config_dir);

    let project_name = if let Some(ref pid) = note.project_id {
        db.find_project_name_by_id(pid)?
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
