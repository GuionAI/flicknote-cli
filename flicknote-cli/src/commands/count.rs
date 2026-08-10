use clap::Args;
use flicknote_core::error::CliError;
use flicknote_core::services::dto::NoteCountInput;
use flicknote_sync::ipc::{AppRequest, DaemonClient};

use super::util::resolve_project_arg;

#[derive(Args)]
pub(crate) struct CountArgs {
    /// Filter by project name
    #[arg(long)]
    project: Option<String>,
    /// Filter by type
    #[arg(long, value_parser = ["normal", "meeting", "link", "file"])]
    r#type: Option<String>,
    /// Count archived (deleted) notes instead of active
    #[arg(long)]
    archived: bool,
    /// Filter by keywords (OR match across title, content, summary)
    keywords: Vec<String>,
}

pub(crate) async fn run(daemon: &DaemonClient<'_>, args: &CountArgs) -> Result<(), CliError> {
    let project = resolve_project_arg(&args.project);
    let count: u64 = match daemon
        .call(AppRequest::NoteCount(NoteCountInput {
            keywords: args.keywords.clone(),
            project: project.clone(),
            note_type: args.r#type.clone(),
            archived: args.archived,
        }))
        .await
    {
        Ok(count) => count,
        Err(error) if error.code() == "project_not_found" => {
            eprintln!(
                "Warning: no project found with name \"{}\".",
                project.as_deref().unwrap_or_default()
            );
            0
        }
        Err(error) => return Err(error.into()),
    };
    println!("{count}");
    Ok(())
}
