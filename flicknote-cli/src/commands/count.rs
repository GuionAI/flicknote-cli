use clap::Args;
use flicknote_core::backend::{NoteDb, NoteFilter};
use flicknote_core::error::CliError;

use super::util::resolve_project_arg;

#[derive(Args)]
pub(crate) struct CountArgs {
    /// Filter by project name
    #[arg(long)]
    project: Option<String>,
    /// Filter by type
    #[arg(long, value_parser = ["normal", "voice", "link", "file"])]
    r#type: Option<String>,
    /// Count archived (deleted) notes instead of active
    #[arg(long)]
    archived: bool,
    /// Filter by keywords (OR match across title, content, summary)
    keywords: Vec<String>,
}

pub(crate) fn run(db: &dyn NoteDb, args: &CountArgs) -> Result<(), CliError> {
    let effective_project = resolve_project_arg(&args.project);

    let project_id: Option<String> = if let Some(ref name) = effective_project {
        match db.find_project_by_name(name)? {
            Some(id) => Some(id),
            None => {
                eprintln!("Warning: no project found with name \"{name}\".");
                println!("0");
                return Ok(());
            }
        }
    } else {
        None
    };

    let filter = NoteFilter {
        project_id: project_id.as_deref(),
        note_type: args.r#type.as_deref(),
        archived: args.archived,
        limit: u32::MAX,
    };

    let count = if args.keywords.is_empty() {
        db.count_notes(&filter)?
    } else {
        // Use search_notes and count results
        let notes = db.search_notes(&args.keywords, &filter)?;
        notes.len() as u64
    };

    println!("{count}");
    Ok(())
}
