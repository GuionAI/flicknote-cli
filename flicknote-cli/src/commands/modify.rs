use super::util::{
    apply_project_move, find_section, get_note_content, resolve_note_id, try_read_stdin,
    write_content,
};
use clap::Args;
use flicknote_core::backend::NoteDb;
use flicknote_core::config::Config;
use flicknote_core::error::CliError;
#[derive(Args)]
pub(crate) struct ModifyArgs {
    /// Note ID (full UUID or prefix)
    id: String,
    /// Edit only the named section (scope = full section including heading)
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
    args: &ModifyArgs,
) -> Result<(), CliError> {
    let full_id = resolve_note_id(db, &args.id).await?;
    let has_metadata = args.project.is_some() || args.flagged || args.unflagged;
    let piped = try_read_stdin()?;
    // Guard: stdin present but not edit-mode shape → redirect to `replace`.
    if let Some(ref s) = piped
        && !super::edit_match::is_edit_mode(s)
    {
        return Err(CliError::Other(
            "stdin doesn't look like edit mode (===BEFORE===/===AFTER===). \
             For overwrite, use `flicknote replace <id>` (with optional --section)."
                .into(),
        ));
    }
    // Guard: nothing to do.
    if piped.is_none() && !has_metadata {
        return Err(CliError::Other(
            "Nothing to modify. Provide edit-mode stdin and/or use \
             --project, --flagged, --unflagged."
                .into(),
        ));
    }
    // Guard: --section without stdin.
    if args.section.is_some() && piped.is_none() {
        return Err(CliError::Other("--section requires edit-mode stdin".into()));
    }
    // Step 1: edit-mode content change (if stdin).
    if let Some(input) = piped {
        let (before, after) = super::edit_match::parse_edit_input(&input)?;
        if let Some(ref section_id) = args.section {
            // Section-scoped edit: operates on raw content, no frontmatter
            let full_content = get_note_content(db, &full_id).await?;
            let doc = crate::markdown::parse_markdown(&full_content);
            let bounds = find_section(&doc, section_id, &full_id)?;
            let scope = &full_content[bounds.start..bounds.end];
            let m = super::edit_match::find_unique(scope, &before)?;
            let abs = super::edit_match::MatchInfo {
                start: bounds.start + m.start,
                end: bounds.start + m.end,
            };
            let new_content = super::edit_match::splice(&full_content, &abs, &after);
            write_content(db, &full_id, new_content.trim()).await?;
            println!("edit applied to note {} (1 replacement)\n", full_id);
            print!("{}", crate::markdown::render_tree(new_content.trim()));
        } else {
            // Whole-note edit: build editable document, apply edit, parse result
            let full_content = get_note_content(db, &full_id).await?;
            // Build the editable document as seen by the user
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
            let (stored_frontmatter, body_without_fm) =
                crate::frontmatter::split_frontmatter(&full_content);
            let note = db.find_note(&full_id).await?;
            let display_content = crate::frontmatter::build_editable_content(
                note.title.as_deref(),
                body_without_fm,
                &topics,
                &entities,
                stored_frontmatter,
            );
            // Apply edit-mode replacement against the display content
            let m = super::edit_match::find_unique(&display_content, &before)?;
            let new_display = super::edit_match::splice(&display_content, &m, &after);
            // Parse the result back
            let doc = crate::frontmatter::parse_editable_doc(&new_display);
            // Validate: full-note write requires a non-empty H1 title
            crate::frontmatter::validate_title_required(&doc).map_err(|e| {
                CliError::Other(e.message)
            })?;
            // Update title
            if let Some(ref new_title) = doc.title {
                db.update_note_title(&full_id, new_title).await?;
            }
            // Update extractions
            db.set_note_extractions(&full_id, "topic", &doc.topics).await?;
            db.set_note_extractions(&full_id, "entity", &doc.entities).await?;
            // Store body
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
            println!("edit applied to note {} (1 replacement)\n", full_id);
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
    use super::super::util::classify_stdin_buf;
    #[test]
    fn test_classify_stdin_buf_via_util() {
        assert_eq!(classify_stdin_buf("  \n  "), None);
        assert_eq!(classify_stdin_buf("x"), Some("x".to_string()));
        assert_eq!(classify_stdin_buf(" foo "), Some(" foo".to_string()));
    }
}
