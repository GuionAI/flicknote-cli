use clap::Args;
use flicknote_core::backend::NoteDb;
use flicknote_core::config::Config;
use flicknote_core::error::CliError;

use super::util::{display_note_id, find_section, get_note_content, resolve_note_id};

#[derive(Args)]
pub(crate) struct DeleteArgs {
    /// Note short ID. A full UUID is also accepted for pending-sync notes.
    id: String,
    /// Remove a specific section by section ID (2-char base62) instead of deleting the note
    #[arg(short = 's', long = "section")]
    section: Option<String>,
}

pub(crate) async fn run(
    db: &dyn NoteDb,
    _config: &Config,
    args: &DeleteArgs,
) -> Result<(), CliError> {
    let full_id = resolve_note_id(db, &args.id).await?;
    let note = db.find_note(&full_id).await?;
    let display_id = display_note_id(&note);

    if let Some(ref section_id) = args.section {
        // Section deletion — remove the section from content
        let content = get_note_content(db, &full_id).await?;
        let doc = crate::markdown::parse_markdown(&content);
        let bounds = find_section(&doc, section_id, &args.id)?;

        let before = &content[..bounds.start];
        let after = &content[bounds.end..];
        let new_content = format!(
            "{}{}",
            before.trim_end_matches('\n'),
            if after.is_empty() {
                String::new()
            } else {
                format!("\n\n{}", after.trim_start_matches('\n'))
            }
        );

        db.update_note_content(&full_id, new_content.trim(), true)
            .await?;

        println!(
            "Removed section '{}' from note {}.\n",
            bounds.heading.text, display_id
        );
        print!("{}", crate::markdown::render_tree(new_content.trim()));
    } else {
        // Soft-delete (archive) the note
        let now = chrono::Utc::now().to_rfc3339();
        db.set_note_deleted_at(&full_id, Some(&now), &now).await?;
        println!("Deleted note {}.", display_id);
    }

    Ok(())
}
