use clap::Args;
use flicknote_core::config::Config;
use flicknote_core::db::Database;
use flicknote_core::error::CliError;
use rusqlite::params;

use super::util::{get_note_content, read_content_or_stdin, resolve_note_id};

#[derive(Args)]
pub(crate) struct EditArgs {
    /// Note ID (full UUID or prefix)
    id: String,
    /// Section heading to replace (case-insensitive contains match)
    #[arg(short = 's', long = "section")]
    section: String,
    /// New content for the section. Reads from stdin if omitted.
    content: Option<String>,
}

pub(crate) fn run(db: &Database, config: &Config, args: &EditArgs) -> Result<(), CliError> {
    let user_id = flicknote_core::session::get_user_id(config)?;
    let full_id = resolve_note_id(db, &args.id)?;
    let now = chrono::Utc::now().to_rfc3339();

    let content = get_note_content(db, &full_id, &user_id, &args.id)?;
    let doc = crate::markdown::parse_markdown(&content);

    // Find section by contains match, enforce uniqueness
    let matches = doc.filter_headings(&args.section);
    let heading = match matches.len() {
        0 => {
            return Err(CliError::Other(format!(
                "Section '{}' not found. Use `flicknote get {} --tree` to see structure.",
                args.section, args.id
            )));
        }
        1 => matches[0],
        _ => {
            let names: Vec<_> = matches.iter().map(|h| format!("  - {}", h.text)).collect();
            return Err(CliError::Other(format!(
                "'{}' matches {} headings — be more specific:\n{}",
                args.section,
                matches.len(),
                names.join("\n")
            )));
        }
    };

    // Get replacement content from arg or stdin (empty = section removal)
    let new_section = read_content_or_stdin(&args.content, true)?;

    // Calculate section byte range (heading line through end of section)
    let heading_idx = doc
        .headings
        .iter()
        .position(|h| h.text == heading.text && h.offset == heading.offset)
        .unwrap();

    let start = heading.offset;
    let end = doc
        .headings
        .iter()
        .skip(heading_idx + 1)
        .find(|h| h.level <= heading.level)
        .map(|h| h.offset)
        .unwrap_or(content.len());

    // Build new content: before + replacement + after
    let before = &content[..start];
    let after = &content[end..];
    let new_content = if new_section.is_empty() {
        format!(
            "{}{}",
            before.trim_end_matches('\n'),
            if after.is_empty() {
                "".to_string()
            } else {
                format!("\n\n{}", after.trim_start_matches('\n'))
            }
        )
    } else {
        format!("{}{}\n\n{}", before, new_section.trim_end(), after)
    };

    // Write back + re-queue for AI
    db.write(|conn| {
        conn.execute(
            "UPDATE notes SET content = ?, status = 'ai_queued', updated_at = ? WHERE id = ? AND user_id = ?",
            params![new_content.trim(), now, full_id, user_id],
        )?;
        Ok(())
    })?;

    if new_section.is_empty() {
        println!(
            "Removed section '{}' from note {}.",
            heading.text,
            &full_id[..8]
        );
    } else {
        println!(
            "Edited section '{}' in note {}.",
            heading.text,
            &full_id[..8]
        );
    }
    Ok(())
}
