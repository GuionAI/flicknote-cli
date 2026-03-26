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
}

pub(crate) fn run(db: &dyn NoteDb, args: &ContentArgs) -> Result<(), CliError> {
    if !args.id.chars().all(|c| c.is_ascii_hexdigit() || c == '-') {
        return Err(CliError::NoteNotFound {
            id: args.id.clone(),
        });
    }

    let full_id = db.resolve_note_id(&args.id)?;
    let note = db.find_note(&full_id)?;

    let content = note.content.as_deref().ok_or_else(|| {
        CliError::Other("This note has no text content (link or file note)".into())
    })?;

    // Synthesize H1 from title for display
    let display_content = if let Some(ref t) = note.title {
        format!("# {t}\n\n{content}")
    } else {
        content.to_string()
    };

    if let Some(ref section_id) = args.section {
        // For --section we need to parse with IDs and extract
        let doc = crate::markdown::parse_markdown(&display_content);
        let bounds = super::util::find_section(&doc, section_id, &args.id)?;
        let section_content = &display_content[bounds.start..bounds.end];
        // Inject IDs into the section content only
        print!(
            "{}",
            crate::markdown::render_content_with_ids(section_content.trim())
        );
    } else {
        print!(
            "{}",
            crate::markdown::render_content_with_ids(&display_content)
        );
    }

    Ok(())
}
