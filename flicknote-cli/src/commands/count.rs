use clap::Args;
use flicknote_core::backend::NoteDb;
use flicknote_core::error::CliError;
use flicknote_core::services::error::ServiceError;
use flicknote_core::services::note::{NoteCountInput, NoteService};

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

pub(crate) async fn run(db: &dyn NoteDb, args: &CountArgs) -> Result<(), CliError> {
    let project = resolve_project_arg(&args.project);
    let count = match NoteService::new(db)
        .count(NoteCountInput {
            keywords: args.keywords.clone(),
            project: project.clone(),
            note_type: args.r#type.clone(),
            archived: args.archived,
        })
        .await
    {
        Ok(count) => count,
        Err(ServiceError::ProjectNotFound(_)) => {
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
