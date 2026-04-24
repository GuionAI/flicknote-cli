//! `flicknote replace` — overwrite note content (whole note or section).

use clap::Args;
use flicknote_core::backend::NoteDb;
use flicknote_core::config::Config;
use flicknote_core::error::CliError;
use std::io::{IsTerminal, Read};

use super::add::resolve_project;
use super::util::{find_section, get_note_content, resolve_note_id, write_content};

/// Check whether content starts with a markdown heading (ATX or setext).
pub(crate) fn content_starts_with_heading(content: &str) -> bool {
    use pulldown_cmark::{Event, Options, Parser, Tag};
    Parser::new_ext(content, Options::empty())
        .next()
        .is_some_and(|e| matches!(e, Event::Start(Tag::Heading { .. })))
}

/// Read optional stdin content. Returns `None` if stdin is a terminal or empty.
fn try_read_stdin() -> Result<Option<String>, CliError> {
    if std::io::stdin().is_terminal() {
        return Ok(None);
    }
    let mut buf = String::new();
    std::io::stdin().read_to_string(&mut buf)?;
    let trimmed = buf.trim_end_matches(|c: char| c.is_ascii_whitespace());
    let trimmed = trimmed.trim_end_matches(' ');
    if trimmed.is_empty() {
        Ok(None)
    } else {
        Ok(Some(trimmed.to_string()))
    }
}

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

pub(crate) fn run(db: &dyn NoteDb, _config: &Config, args: &ReplaceArgs) -> Result<(), CliError> {
    let full_id = resolve_note_id(db, &args.id)?;
    let has_metadata =
        args.project.is_some() || args.title.is_some() || args.flagged || args.unflagged;

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
            let content = get_note_content(db, &full_id)?;
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
            write_content(db, &full_id, new_content.trim())?;
            println!("Replaced section in note {}.\n", full_id);
            print!("{}", crate::markdown::render_tree(new_content.trim()));
        } else {
            // Replace entire note content.
            write_content(db, &full_id, &new_body)?;
            println!("Replaced content for note {}.\n", full_id);
            print!("{}", crate::markdown::render_tree(&new_body));
        }
    }

    // Step 2: metadata updates.
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
    use super::*;

    // ── content_starts_with_heading ───────────────────────────────────────────

    #[test]
    fn test_content_starts_with_heading_atx() {
        assert!(content_starts_with_heading("# Heading"));
        assert!(content_starts_with_heading("## Heading"));
        assert!(content_starts_with_heading("### Heading"));
        assert!(content_starts_with_heading("\n## Heading after blank"));
    }

    #[test]
    fn test_content_starts_with_heading_setext() {
        assert!(content_starts_with_heading("My Section\n=========="));
        assert!(content_starts_with_heading("My Section\n----------"));
        assert!(content_starts_with_heading("\nMy Section\n=========="));
    }

    #[test]
    fn test_content_starts_with_heading_false() {
        assert!(!content_starts_with_heading("plain text"));
        assert!(!content_starts_with_heading("some body\n\nmore text"));
        assert!(!content_starts_with_heading(""));
        assert!(!content_starts_with_heading("#NoSpace"));
    }

    // ── replace_entire_section integration ────────────────────────────────────

    #[test]
    fn test_replace_section_requires_heading() {
        // body-only stdin with --section → error containing "heading".
        let _content = "## Old Title\n\nold body.";
        let new_body = "new body without heading";
        assert!(!content_starts_with_heading(new_body));
    }

    #[test]
    fn test_replace_section_setext_atx() {
        // Setext heading is recognized as a valid heading.
        assert!(content_starts_with_heading("My Section\n=========="));
        assert!(content_starts_with_heading("My Section\n----------"));
    }
}
