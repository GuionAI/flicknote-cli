use clap::Args;
use flicknote_core::backend::NoteDb;
use flicknote_core::config::Config;
use flicknote_core::error::CliError;

use super::util::{find_section, get_note_content, resolve_note_id};

#[derive(Args)]
pub(crate) struct RenameArgs {
    /// Note ID (full UUID or prefix)
    id: String,
    /// Section heading to rename (case-insensitive contains match)
    #[arg(short = 's', long = "section")]
    section: String,
    /// New heading text (without # prefix — level is preserved)
    name: String,
}

pub(crate) fn run(db: &dyn NoteDb, _config: &Config, args: &RenameArgs) -> Result<(), CliError> {
    let full_id = resolve_note_id(db, &args.id)?;
    let content = get_note_content(db, &full_id)?;
    let doc = crate::markdown::parse_markdown(&content);
    let bounds = find_section(&doc, &args.section, &args.id)?;

    let heading_line_end = content[bounds.start..]
        .find('\n')
        .map(|i| bounds.start + i)
        .unwrap_or(content.len());

    let prefix = "#".repeat(bounds.heading.level);
    let new_heading_line = format!("{prefix} {}", args.name);

    let before = &content[..bounds.start];
    let after = &content[heading_line_end..];
    let new_content = format!("{before}{new_heading_line}{after}");

    db.update_note_content(&full_id, new_content.trim(), true)?;

    println!(
        "Renamed '{}' → '{}' in note {}.\n",
        bounds.heading.text, args.name, full_id
    );
    print!("{}", crate::markdown::render_tree(new_content.trim()));
    Ok(())
}
