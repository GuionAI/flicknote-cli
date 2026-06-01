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
            let display_content =
                crate::editable_document::load_editable_note(db, &full_id).await?;
            // Apply edit-mode replacement against the display content
            let m = super::edit_match::find_unique(&display_content, &before)?;
            let new_display = super::edit_match::splice(&display_content, &m, &after);
            let result =
                crate::editable_document::save_editable_note(db, &full_id, &new_display).await?;
            println!("edit applied to note {} (1 replacement)\n", full_id);
            print!("{}", crate::markdown::render_tree(&result.stored_content));
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
    use super::super::util::{classify_stdin_buf, find_section};

    #[test]
    fn test_classify_stdin_buf_via_util() {
        assert_eq!(classify_stdin_buf("  \n  "), None);
        assert_eq!(classify_stdin_buf("x"), Some("x".to_string()));
        assert_eq!(classify_stdin_buf(" foo "), Some(" foo".to_string()));
    }

    #[test]
    fn test_modify_section_preserves_frontmatter_outside_section_scope() {
        let content = "---\ncustom: keep\n---\n\n## Target\nold body\n\n## Other\nother body";
        let doc = crate::markdown::parse_markdown(content);
        let heading = doc
            .headings
            .iter()
            .find(|heading| heading.text == "Target")
            .expect("target heading should parse");
        let bounds = find_section(&doc, &heading.id, "note-id").unwrap();
        let scope = &content[bounds.start..bounds.end];
        let match_info = super::super::edit_match::find_unique(scope, "old body").unwrap();
        let absolute = super::super::edit_match::MatchInfo {
            start: bounds.start + match_info.start,
            end: bounds.start + match_info.end,
        };

        let updated = super::super::edit_match::splice(content, &absolute, "new body");

        assert!(updated.starts_with("---\ncustom: keep\n---"));
        assert!(updated.contains("## Target\nnew body"));
        assert!(updated.contains("## Other\nother body"));
    }
}
