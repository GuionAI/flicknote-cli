//! `flicknote replace` — overwrite note content (whole note or section).
use clap::Args;
use flicknote_core::backend::NoteDb;
use flicknote_core::config::Config;
use flicknote_core::error::CliError;
use super::util::{
    apply_project_move, content_starts_with_heading, find_section, get_note_content,
    resolve_note_id, try_read_stdin, write_content,
};
#[derive(Args)]
pub(crate) struct ReplaceArgs {
    /// Note ID (full UUID or prefix)
    id: String,
    /// Replace only the named section (stdin must start with a heading)
    #[arg(short = 's', long = "section")]
    section: Option<String>,
    /// Move note to this project
    #[arg(short = 'p', long = "project")]
    project: Option<String>,
    /// Mark note as flagged
    #[arg(long, conflicts_with = "unflagged")]
    flagged: bool,
    /// Remove flagged status
    #[arg(long, conflicts_with = "flagged")]
    unflagged: bool,
}
pub(crate) async fn run(
    db: &dyn NoteDb,
    _config: &Config,
    args: &ReplaceArgs,
) -> Result<(), CliError> {
    let full_id = resolve_note_id(db, &args.id).await?;
    let has_metadata = args.project.is_some() || args.flagged || args.unflagged;
    let piped = try_read_stdin()?;
    // Nothing to do — no stdin and no metadata.
    if piped.is_none() && !has_metadata {
        return Err(CliError::Other(
            "Nothing to replace. Provide content via stdin or use metadata flags.".into(),
        ));
    }
    // --section requires stdin.
    if args.section.is_some() && piped.is_none() {
        return Err(CliError::Other(
            "--section requires content from stdin".into(),
        ));
    }
    // Step 1: overwrite content (if stdin provided).
    if let Some(new_body) = piped {
        if let Some(ref section_id) = args.section {
            // Section-scoped replace: no frontmatter parsing
            let content = get_note_content(db, &full_id).await?;
            let doc = crate::markdown::parse_markdown(&content);
            let bounds = find_section(&doc, section_id, &full_id)?;
            // Validate that stdin starts with a heading.
            if !content_starts_with_heading(&new_body) {
                return Err(CliError::Other(
                    "stdin must start with a heading (ATX or setext) — \
                     for body-only edits, use `flicknote modify <id> --section <s>` \
                     with ===BEFORE===/===AFTER==="
                        .into(),
                ));
            }
            // Cap heading levels at the original section's level.
            let shifted = crate::markdown::cap_heading_level(new_body.trim(), bounds.heading.level);
            let new_content = crate::markdown::replace_entire_section(
                &content,
                bounds.start,
                bounds.end,
                &shifted,
            );
            write_content(db, &full_id, new_content.trim()).await?;
            println!("Replaced section in note {}.\n", full_id);
            print!("{}", crate::markdown::render_tree(new_content.trim()));
        } else {
            // Whole-note replace: parse editable document format
            let doc = crate::frontmatter::parse_editable_doc(&new_body);
            // Validate: full-note write requires a non-empty H1 title
            crate::frontmatter::validate_title_required(&doc).map_err(|e| {
                CliError::Other(e.message)
            })?;
            // Update title from H1
            if let Some(ref new_title) = doc.title {
                db.update_note_title(&full_id, new_title).await?;
            }
            // Update extractions
            db.set_note_extractions(&full_id, "topic", &doc.topics).await?;
            db.set_note_extractions(&full_id, "entity", &doc.entities).await?;
            // Store body content: either body alone, or with unmanaged frontmatter
            let stored_content = if let Some(ref fm) = doc.unmanaged_frontmatter {
                if doc.body.is_empty() {
                    fm.clone()
                } else {
                    format!("{}\n\n{}", fm, doc.body)
                }
            } else {
                doc.body.clone()
            };
            write_content(db, &full_id, &stored_content).await?;
            println!("Replaced content for note {}.\n", full_id);
            print!("{}", crate::markdown::render_tree(&stored_content));
        }
    }
    // Step 2: metadata updates.
    if let Some(ref project_name) = args.project {
        apply_project_move(db, &full_id, project_name).await?;
    }
    if args.flagged {
        db.update_note_flagged(&full_id, true).await?;
        println!("Flagged note {}.", full_id);
    } else if args.unflagged {
        db.update_note_flagged(&full_id, false).await?;
        println!("Unflagged note {}.", full_id);
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_replace_section_setext_atx() {
        // Setext heading is recognized as a valid heading.
        assert!(content_starts_with_heading("My Section\n=========="));
        assert!(content_starts_with_heading("My Section\n----------"));
    }
}
