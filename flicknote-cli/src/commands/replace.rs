use clap::Args;
use flicknote_core::backend::NoteDb;
use flicknote_core::config::Config;
use flicknote_core::error::CliError;

use flicknote_core::hooks;

use super::util::{find_section, get_note, read_stdin_required, resolve_note_id};

#[derive(Args)]
pub(crate) struct ReplaceArgs {
    /// Note ID (full UUID or prefix)
    id: String,
    /// Replace only the named section (case-insensitive contains match)
    #[arg(short = 's', long = "section")]
    section: Option<String>,
}

/// Run the on-modify hook and write updated content to the database.
fn write_note(
    db: &dyn NoteDb,
    config: &Config,
    full_id: &str,
    new_content: &str,
) -> Result<(), CliError> {
    let now = chrono::Utc::now().to_rfc3339();

    let old_note = get_note(db, full_id)?;
    let mut new_note = old_note.clone();
    new_note.content = Some(new_content.to_string());
    new_note.status = "ai_queued".to_string();
    new_note.updated_at = Some(now.clone());

    let old_json = serde_json::to_string(&old_note)?;
    let new_json = serde_json::to_string(&new_note)?;
    let config_dir = config.paths.config_dir.to_string_lossy();
    hooks::run_on_modify(
        &config.paths.hooks_dir,
        &old_json,
        &new_json,
        "replace",
        &config_dir,
    )?;

    db.update_note_content(full_id, new_content, true)
}

/// Validate that replacement content starts with a heading.
fn validate_replacement_heading(
    content: &str,
    section_id: &str,
    section_heading_text: &str,
) -> Result<(), CliError> {
    let first_non_empty = content.lines().find(|l| !l.trim().is_empty());
    let starts_with_heading = first_non_empty
        .and_then(crate::markdown::heading_level)
        .is_some();
    if starts_with_heading {
        return Ok(());
    }
    Err(CliError::Other(format!(
        "error: replacement content must start with a heading (root of the subtree)\n\n  You are replacing a subtree rooted at:\n    [{}] {}",
        section_id, section_heading_text,
    )))
}

pub(crate) fn run(db: &dyn NoteDb, config: &Config, args: &ReplaceArgs) -> Result<(), CliError> {
    let full_id = resolve_note_id(db, &args.id)?;

    if let Some(section_id) = &args.section {
        let content = db
            .find_note_content(&full_id)?
            .ok_or_else(|| CliError::Other("Note has no content".into()))?;
        let doc = crate::markdown::parse_markdown(&content);
        let bounds = find_section(&doc, section_id, &args.id)?;
        let heading_level = bounds.heading.level;
        let start = bounds.start;
        let end = bounds.end;

        let new_body = read_stdin_required()?;
        validate_replacement_heading(&new_body, section_id, &bounds.heading.text)?;

        let shifted = crate::markdown::cap_heading_level(new_body.trim(), heading_level);
        let new_content = crate::markdown::replace_entire_section(&content, start, end, &shifted);

        write_note(db, config, &full_id, new_content.trim())?;
        println!("Replaced section in note {}.\n", &full_id[..8]);
        print!("{}", crate::markdown::render_tree(new_content.trim()));
    } else {
        let content = read_stdin_required()?;
        write_note(db, config, &full_id, &content)?;
        println!("Replaced content for note {}.\n", &full_id[..8]);
        print!("{}", crate::markdown::render_tree(&content));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_replace_section_errors_if_no_leading_heading() {
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
        assert!(
            msg.contains("Section Title"),
            "error should include heading text, got: {msg}"
        );
    }

    #[test]
    fn test_replace_section_ok_with_leading_heading() {
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
    fn test_replace_section_errors_on_hash_no_space() {
        let result = validate_replacement_heading("#NoSpace", "kE", "Section Title");
        assert!(
            result.is_err(),
            "#NoSpace should not be accepted as a heading"
        );
    }

    #[test]
    fn test_replace_section_ok_with_leading_blank_lines() {
        let result = validate_replacement_heading(
            "\n\n## Updated Section\n\nContent.",
            "kE",
            "Section Title",
        );
        assert!(
            result.is_ok(),
            "leading blank lines before heading should return Ok"
        );
    }
}
