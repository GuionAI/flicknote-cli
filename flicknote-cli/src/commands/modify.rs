use clap::Args;
use flicknote_core::backend::NoteDb;
use flicknote_core::config::Config;
use flicknote_core::error::CliError;
use std::io::{IsTerminal, Read};

use super::add::resolve_project;
use super::util::{find_section, resolve_note_id, write_content};

/// Read piped stdin content, returning `None` when stdin is a terminal or empty.
/// Empty stdin is treated the same as no stdin — matches ttal's readStdinIfPiped
/// pattern so non-TTY contexts (agents, CI) don't spuriously report piped data.
fn try_read_stdin() -> Result<Option<String>, CliError> {
    if std::io::stdin().is_terminal() {
        return Ok(None);
    }
    let mut buf = String::new();
    std::io::stdin().read_to_string(&mut buf)?;
    Ok(classify_stdin_buf(&buf))
}

/// Classify a freshly-read stdin buffer. Pure helper, testable without a TTY.
fn classify_stdin_buf(buf: &str) -> Option<String> {
    let trimmed = buf.trim_end_matches(|c: char| c.is_ascii_whitespace());
    let trimmed = trimmed.trim_end_matches(' ');
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

#[derive(Args)]
pub(crate) struct ModifyArgs {
    /// Note ID (full UUID or prefix)
    id: String,
    /// Replace only the named section by section ID (2-char base62)
    #[arg(short = 's', long = "section")]
    section: Option<String>,
    /// Stdin includes the heading line (otherwise heading is preserved)
    #[arg(long)]
    with_heading: bool,
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

/// Check whether content starts with a heading (ATX or setext).
fn content_starts_with_heading(content: &str) -> bool {
    use pulldown_cmark::{Event, Options, Parser, Tag};
    Parser::new_ext(content, Options::empty())
        .next()
        .is_some_and(|e| matches!(e, Event::Start(Tag::Heading { .. })))
}

/// Validate that replacement content starts with a heading.
fn validate_replacement_heading(
    content: &str,
    section_id: &str,
    section_heading_text: &str,
) -> Result<(), CliError> {
    if content_starts_with_heading(content) {
        return Ok(());
    }
    Err(CliError::Other(format!(
        "error: replacement content must start with a heading (root of the subtree)\n\n  You are replacing a subtree rooted at:\n    [{}] {}",
        section_id, section_heading_text,
    )))
}

pub(crate) fn run(db: &dyn NoteDb, _config: &Config, args: &ModifyArgs) -> Result<(), CliError> {
    let full_id = resolve_note_id(db, &args.id)?;

    let has_metadata =
        args.project.is_some() || args.title.is_some() || args.flagged || args.unflagged;

    let piped = try_read_stdin()?;
    let stdin_content = if has_metadata && args.section.is_none() && piped.is_some() {
        eprintln!(
            "warning: piped content ignored because metadata flags are set without --section. \
             To also replace note content, use --section."
        );
        None
    } else {
        piped
    };

    // --section without stdin is an error
    if args.section.is_some() && stdin_content.is_none() {
        return Err(CliError::Other(
            "--section requires content from stdin".into(),
        ));
    }

    // --with-heading without --section is meaningless
    if args.with_heading && args.section.is_none() {
        return Err(CliError::Other("--with-heading requires --section".into()));
    }

    if stdin_content.is_none() && !has_metadata {
        return Err(CliError::Other(
            "Nothing to modify. Provide content via stdin and/or use --project, --title, --flagged, --unflagged.".into(),
        ));
    }

    // Step 1: content replacement (if stdin has content)
    if let Some(new_body) = stdin_content {
        if let Some(ref section_id) = args.section {
            let content = db
                .find_note_content(&full_id)?
                .ok_or_else(|| CliError::Other("Note has no content".into()))?;
            let doc = crate::markdown::parse_markdown(&content);
            let bounds = find_section(&doc, section_id, &args.id)?;
            let heading_level = bounds.heading.level;
            let start = bounds.start;
            let end = bounds.end;

            if args.with_heading {
                // stdin includes heading — validate it starts with a heading
                validate_replacement_heading(&new_body, section_id, &bounds.heading.text)?;
                let shifted = crate::markdown::cap_heading_level(new_body.trim(), heading_level);
                let new_content =
                    crate::markdown::replace_entire_section(&content, start, end, &shifted);
                write_content(db, &full_id, new_content.trim())?;
                println!("Replaced section in note {}.\n", full_id);
                print!("{}", crate::markdown::render_tree(new_content.trim()));
            } else {
                // stdin is body-only — reject if it starts with a heading (ATX or setext)
                if content_starts_with_heading(&new_body) {
                    return Err(CliError::Other(
                        "stdin starts with a heading — did you mean to use --with-heading?".into(),
                    ));
                }

                // preserve original heading
                let original_heading_end = content[start..]
                    .find('\n')
                    .map(|i| start + i + 1)
                    .unwrap_or(end);
                let preserved_heading = content[start..original_heading_end].trim_end();
                let new_section = format!("{preserved_heading}\n\n{}", new_body.trim());
                let new_content =
                    crate::markdown::replace_entire_section(&content, start, end, &new_section);
                write_content(db, &full_id, new_content.trim())?;
                println!("Replaced section body in note {}.\n", full_id);
                print!("{}", crate::markdown::render_tree(new_content.trim()));
            }
        } else {
            // Replace all content
            write_content(db, &full_id, &new_body)?;
            println!("Replaced content for note {}.\n", full_id);
            print!("{}", crate::markdown::render_tree(&new_body));
        }
    }

    // Step 2: metadata updates
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

    #[test]
    fn test_validate_replacement_heading_errors_if_no_heading() {
        let result =
            validate_replacement_heading("Some body text without a heading", "kE", "Section Title");
        assert!(result.is_err(), "body-only content should return Err");
        let msg = format!("{}", result.unwrap_err());
        assert!(
            msg.contains("heading"),
            "error should mention 'heading', got: {msg}"
        );
        assert!(
            msg.contains("[kE]"),
            "error should include section ID, got: {msg}"
        );
    }

    #[test]
    fn test_validate_replacement_heading_ok_with_heading() {
        let result = validate_replacement_heading(
            "## Updated Section\n\nSome content here.",
            "kE",
            "Section Title",
        );
        assert!(
            result.is_ok(),
            "content starting with '## ' should return Ok"
        );
    }

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

    #[test]
    fn test_classify_stdin_buf() {
        // Empty and whitespace-only → None
        assert_eq!(classify_stdin_buf(""), None);
        assert_eq!(classify_stdin_buf("  \n  "), None);
        // Non-empty → content with trailing ASCII whitespace stripped
        assert_eq!(classify_stdin_buf("x"), Some("x".to_string()));
        assert_eq!(classify_stdin_buf(" foo "), Some(" foo".to_string()));
        assert_eq!(
            classify_stdin_buf("foo\nbar\n"),
            Some("foo\nbar".to_string())
        );
    }
}
