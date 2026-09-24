use clap::Args;
use flicknote_core::error::CliError;
use flicknote_core::services::dto::{NoteListInput, NoteListItem};
use flicknote_core::types::NoteStatus;
use flicknote_sync::ipc::{AppRequest, DaemonClient};

use super::util::print_summaries_table;

const LIST_HELP: &str = include_str!("../help/list.md");

#[derive(Args)]
#[command(after_help = LIST_HELP)]
pub(crate) struct ListArgs {
    /// Filter by type
    #[arg(long, value_parser = ["normal", "meeting", "link"])]
    r#type: Option<String>,
    /// Filter by lifecycle status
    #[arg(long)]
    status: Option<NoteStatus>,
    /// Filter by project name
    #[arg(long)]
    project: Option<String>,
    /// Show only notes without a project
    #[arg(long, conflicts_with = "project")]
    no_project: bool,
    /// Include notes created at or after this RFC3339 instant
    #[arg(long)]
    created_after: Option<String>,
    /// Include notes created before this RFC3339 instant
    #[arg(long)]
    created_before: Option<String>,
    /// Show only archived notes
    #[arg(long)]
    archived: bool,
    /// Maximum number of results
    #[arg(long, default_value = "20")]
    limit: u32,
    /// Continue after this note ID
    #[arg(long)]
    cursor: Option<i64>,
    /// Output as JSON
    #[arg(long)]
    json: bool,
}

pub(crate) async fn run(daemon: &DaemonClient<'_>, args: &ListArgs) -> Result<(), CliError> {
    let project = args.project.clone();
    let notes: Vec<NoteListItem> = match daemon
        .call(AppRequest::NoteList(NoteListInput {
            note_type: args.r#type.clone(),
            status: args.status.map(|status| status.as_str().to_string()),
            project: project.clone(),
            no_project: args.no_project,
            created_after: args.created_after.clone(),
            created_before: args.created_before.clone(),
            archived: args.archived,
            limit: args.limit,
            cursor: args.cursor,
        }))
        .await
    {
        Ok(notes) => notes,
        Err(error) if error.code() == "project_not_found" => {
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
