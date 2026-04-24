use super::add::resolve_project;
use super::util::{find_section, get_note_content, resolve_note_id, try_read_stdin, write_content};
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
    /// Set new title
    #[arg(long)]
    title: Option<String>,
    /// Mark note as flagged
    #[arg(long, conflicts_with = "unflagged")]
    flagged: bool,
    /// Remove flagged status
    #[arg(long, conflicts_with = "flagged")]
    unflagged: bool,
}

pub(crate) fn run(db: &dyn NoteDb, _config: &Config, args: &ModifyArgs) -> Result<(), CliError> {
    let full_id = resolve_note_id(db, &args.id)?;
    let has_metadata =
        args.project.is_some() || args.title.is_some() || args.flagged || args.unflagged;

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
             --project, --title, --flagged, --unflagged."
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
        let full_content = get_note_content(db, &full_id)?;

        let (scope_start, scope_end) = match &args.section {
            Some(sid) => {
                let doc = crate::markdown::parse_markdown(&full_content);
                let bounds = find_section(&doc, sid, &full_id)?;
                // Scope = full section [start..end) including heading.
                // Users can edit the heading by including it in BEFORE.
                (bounds.start, bounds.end)
            }
            None => (0, full_content.len()),
        };

        let scope = &full_content[scope_start..scope_end];
        let m = super::edit_match::find_unique(scope, &before)?;

        // Adjust match offsets to absolute positions.
        let abs = super::edit_match::MatchInfo {
            start: scope_start + m.start,
            end: scope_start + m.end,
        };
        let new_content = super::edit_match::splice(&full_content, &abs, &after);

        write_content(db, &full_id, new_content.trim())?;
        println!("edit applied to note {} (1 replacement)\n", full_id);
        print!("{}", crate::markdown::render_tree(new_content.trim()));
    }

    // Step 2: metadata updates — unchanged from original.
    if let Some(ref project_name) = args.project {
        let old_note = db.find_note(&full_id)?;
        let old_project_id = old_note.project_id.clone();
        let new_project_id = resolve_project(db, project_name)?;

        if old_project_id.as_deref() == Some(new_project_id.as_str()) {
            println!(
                "Note {} is already in project \"{}\".",
                full_id, project_name
            );
        } else {
            let deleted_name =
                db.move_note_to_project(&full_id, &new_project_id, old_project_id.as_deref())?;
            println!("Moved note {} to project \"{}\".", full_id, project_name);
            if let Some(name) = deleted_name {
                println!("Deleted empty project \"{}\".", name);
            }
        }
    }

    if let Some(ref new_title) = args.title {
        db.update_note_title(&full_id, new_title)?;
        println!("Updated title for note {}.", full_id);
    }

    if args.flagged {
        db.update_note_flagged(&full_id, true)?;
        println!("Flagged note {}.", full_id);
    } else if args.unflagged {
        db.update_note_flagged(&full_id, false)?;
        println!("Unflagged note {}.", full_id);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    // Note: try_read_stdin and classify_stdin_buf are used via util:: but imported
    // here for the test helper to call them through the module hierarchy.
    use super::super::util::classify_stdin_buf;

    #[test]
    fn test_classify_stdin_buf_via_util() {
        assert_eq!(classify_stdin_buf("  \n  "), None);
        assert_eq!(classify_stdin_buf("x"), Some("x".to_string()));
        assert_eq!(classify_stdin_buf(" foo "), Some(" foo".to_string()));
    }
}
