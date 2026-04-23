use clap::Args;
use flicknote_core::backend::NoteDb;
use flicknote_core::config::Config;
use flicknote_core::error::CliError;

use super::util::{find_section, get_note_content, read_stdin_required, resolve_note_id};

#[derive(Args)]
#[command(group(clap::ArgGroup::new("position").required(true)))]
pub(crate) struct InsertArgs {
    /// Note ID (full UUID or prefix)
    id: String,
    /// Insert before this section
    #[arg(long, group = "position")]
    before: Option<String>,
    /// Insert after this section
    #[arg(long, group = "position")]
    after: Option<String>,
}

pub(crate) fn run(db: &dyn NoteDb, _config: &Config, args: &InsertArgs) -> Result<(), CliError> {
    let full_id = resolve_note_id(db, &args.id)?;
    let (section_name, insert_before) = match (&args.before, &args.after) {
        (Some(s), None) => (s.as_str(), true),
        (None, Some(s)) => (s.as_str(), false),
        _ => {
            return Err(CliError::Other(
                "Exactly one of --before or --after is required.".into(),
            ));
        }
    };

    let content = get_note_content(db, &full_id)?;
    let doc = crate::markdown::parse_markdown(&content);
    let bounds = find_section(&doc, section_name, &args.id)?;

    let insert_content = read_stdin_required()?;

    let split_point = if insert_before {
        bounds.start
    } else {
        bounds.end
    };
    let before = content[..split_point].trim_end_matches('\n');
    let after = content[split_point..].trim_start_matches('\n');

    let new_content = if before.is_empty() {
        format!("{}\n\n{after}", insert_content.trim_end())
    } else if after.is_empty() {
        format!("{before}\n\n{}", insert_content.trim_end())
    } else {
        format!("{before}\n\n{}\n\n{after}", insert_content.trim_end())
    };

    db.update_note_content(&full_id, new_content.trim(), true)?;

    let position = if insert_before { "before" } else { "after" };
    println!(
        "Inserted content {position} '{}' in note {}.\n",
        bounds.heading.text, full_id
    );
    print!("{}", crate::markdown::render_tree(new_content.trim()));
    Ok(())
}
