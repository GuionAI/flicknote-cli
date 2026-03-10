use clap::Args;
use flicknote_core::backend::{NoteDb, NoteFilter};
use flicknote_core::error::CliError;

use super::util::{print_notes_table, resolve_project_arg};

#[derive(Args)]
pub(crate) struct ListArgs {
    /// Filter by type
    #[arg(long, value_parser = ["normal", "voice", "link"])]
    r#type: Option<String>,
    /// Filter by project name
    #[arg(long)]
    project: Option<String>,
    /// Show only archived notes
    #[arg(long)]
    archived: bool,
    /// Maximum number of results
    #[arg(long, default_value = "20")]
    limit: u32,
    /// Output as JSON
    #[arg(long)]
    json: bool,
}

pub(crate) fn run(db: &dyn NoteDb, args: &ListArgs) -> Result<(), CliError> {
    let effective_project = resolve_project_arg(&args.project);

    if args.project.is_none()
        && let Some(ref name) = effective_project
    {
        eprintln!("Filtering by project \"{name}\" from $FLICKNOTE_PROJECT.");
    }

    let project_id: Option<String> = if let Some(ref name) = effective_project {
        match db.find_project_by_name(name)? {
            Some(id) => Some(id),
            None => {
                eprintln!("Warning: no project found with name \"{name}\".");
                return Ok(());
            }
        }
    } else {
        None
    };

    let notes = db.list_notes(&NoteFilter {
        project_id: project_id.as_deref(),
        note_type: args.r#type.as_deref(),
        archived: args.archived,
        limit: args.limit,
    })?;

    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&notes).map_err(CliError::Json)?
        );
    } else {
        print_notes_table(&notes);
    }

    Ok(())
}
