use clap::Args;
use flicknote_core::backend::NoteDb;
use flicknote_core::error::CliError;

const CONTENT_HELP: &str = include_str!("../help/content.md");

#[derive(Args)]
#[command(after_help = CONTENT_HELP)]
pub(crate) struct ContentArgs {
    /// Note ID. Use the numeric short ID shown in list/detail. Full UUIDs are also accepted for compatibility.
    id: String,
    /// Extract a specific section by section ID (2-char base62)
    #[arg(short = 's', long = "section")]
    section: Option<String>,
}
pub(crate) async fn run(db: &dyn NoteDb, args: &ContentArgs) -> Result<(), CliError> {
    let full_id = db.resolve_note_id(&args.id).await?;
    let note = db.find_note(&full_id).await?;
    // --section: operates on note body without frontmatter
    if let Some(ref section_id) = args.section {
        let content = note.content.as_deref().ok_or_else(|| {
            CliError::Other("This note has no text content (link or file note)".into())
        })?;
        // Build display content for section extraction
        let display_content = if let Some(ref t) = note.title {
            format!("# {t}\n\n{content}")
        } else {
            content.to_string()
        };
        let doc = crate::markdown::parse_markdown(&display_content);
        let bounds = super::util::find_section(&doc, section_id, &full_id)?;
        let output = display_content[bounds.start..bounds.end].trim().to_string();
        print!("{}", render_content_output(&output));
        return Ok(());
    }
    if note.content.is_none() {
        return Err(CliError::Other(
            "This note has no text content (link or file note)".into(),
        ));
    }
    let output = crate::editable_document::render_editable_note(db, &note).await?;
    print!("{}", render_content_output(&output));
    Ok(())
}

fn render_content_output(content: &str) -> &str {
    content
}

#[cfg(test)]
mod tests {
    #[test]
    fn render_content_output_keeps_heading_text_clean() {
        let content = "# Title\n\n## Section\n\nBody.";

        assert_eq!(super::render_content_output(content), content);
    }
}
