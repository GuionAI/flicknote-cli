use clap::Args;
use flicknote_client::dto::NoteCountInput;
use flicknote_client::{AppRequest, DaemonClient};
use flicknote_core::error::CliError;

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
}

pub(crate) async fn run(daemon: &DaemonClient, args: &CountArgs) -> Result<(), CliError> {
    let project = args.project.clone();
    let count: u64 = match daemon
        .call(AppRequest::NoteCount(NoteCountInput {
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
