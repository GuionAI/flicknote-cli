use clap::Args;
use flicknote_core::backend::NoteDb;
use flicknote_core::error::CliError;
use flicknote_core::services::error::ServiceError;
use flicknote_core::services::note::{NoteListInput, NoteService};

use super::util::{print_summaries_table, resolve_project_arg};

const LIST_HELP: &str = include_str!("../help/list.md");

#[derive(Args)]
#[command(after_help = LIST_HELP)]
pub(crate) struct ListArgs {
    /// Filter by type
    #[arg(long, value_parser = ["normal", "meeting", "link"])]
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

pub(crate) async fn run(db: &dyn NoteDb, args: &ListArgs) -> Result<(), CliError> {
    let project = resolve_project_arg(&args.project);
    if args.project.is_none()
        && let Some(name) = project.as_deref()
    {
        eprintln!("Filtering by project \"{name}\" from $FLICKNOTE_PROJECT.");
    }
    let notes = match NoteService::new(db)
        .list(NoteListInput {
            note_type: args.r#type.clone(),
            project: project.clone(),
            archived: args.archived,
            limit: args.limit,
        })
        .await
    {
        Ok(notes) => notes,
        Err(ServiceError::ProjectNotFound(_)) => {
            eprintln!(
                "Warning: no project found with name \"{}\".",
                project.as_deref().unwrap_or_default()
            );
            return Ok(());
        }
        Err(error) => return Err(error.into()),
    };
    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&notes).map_err(CliError::Json)?
        );
    } else {
        print_summaries_table(&notes);
    }
    Ok(())
}
