use clap::Args;
use flicknote_core::backend::NoteDb;
use flicknote_core::error::CliError;
#[derive(Args)]
pub(crate) struct ContentArgs {
    /// Note ID (full UUID or short prefix)
    id: String,
    /// Extract a specific section by section ID (2-char base62)
    #[arg(short = 's', long = "section")]
    section: Option<String>,
    /// Output raw markdown without section ID annotations (safe for piping to sed/awk)
    #[arg(long = "raw")]
    raw: bool,
}
pub(crate) async fn run(db: &dyn NoteDb, args: &ContentArgs) -> Result<(), CliError> {
    if !args.id.chars().all(|c| c.is_ascii_hexdigit() || c == '-') {
        return Err(CliError::NoteNotFound {
            id: args.id.clone(),
        });
    }
    let full_id = db.resolve_note_id(&args.id).await?;
    let note = db.find_note(&full_id).await?;
    let content = note.content.as_deref().ok_or_else(|| {
        CliError::Other("This note has no text content (link or file note)".into())
    })?;
    // --section: operates on note body without frontmatter
    if args.section.is_some() {
        let section_id = args.section.as_ref().unwrap();
        // Build display content for section extraction
        let display_content = if let Some(ref t) = note.title {
            format!("# {t}\n\n{content}")
        } else {
            content.to_string()
        };
        let doc = crate::markdown::parse_markdown(&display_content);
        let bounds = super::util::find_section(&doc, section_id, &args.id)?;
        let output = display_content[bounds.start..bounds.end].trim().to_string();
        if args.raw {
            print!("{output}");
        } else {
            print!("{}", crate::markdown::render_content_with_ids(&output));
        }
        return Ok(());
    }
    // Full-note display: fetch extractions and build editable document
    let extractions = db
        .list_note_extractions(&[&full_id], &["topic", "entity"])
        .await?;
    let note_extractions = extractions.get(&full_id);
    let mut topics: Vec<String> = Vec::new();
    let mut entities: Vec<String> = Vec::new();
    if let Some(pairs) = note_extractions {
        for (ext_type, value) in pairs {
            match ext_type.as_str() {
                "topic" => topics.push(value.clone()),
                "entity" => entities.push(value.clone()),
                _ => {}
            }
        }
    }
    // Check for stored frontmatter in content
    let (stored_frontmatter, body_without_fm) = crate::frontmatter::split_frontmatter(content);
    let output = crate::frontmatter::build_editable_content(
        note.title.as_deref(),
        body_without_fm,
        &topics,
        &entities,
        stored_frontmatter,
    );
    if args.raw {
        print!("{output}");
    } else {
        print!("{}", crate::markdown::render_content_with_ids(&output));
    }
    Ok(())
}
