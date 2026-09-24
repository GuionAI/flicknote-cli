use clap::Args;
use flicknote_core::error::CliError;
use flicknote_core::services::dto::NoteSummary;
use flicknote_sync::ipc::{AppRequest, DaemonClient};

use super::util::display_summary_id;

const UPLOAD_HELP: &str = include_str!("../help/upload.md");

#[derive(Args)]
#[command(after_help = UPLOAD_HELP)]
pub(crate) struct UploadArgs {
    /// File path to import or upload
    path: String,
    /// Assign to project by name
    #[arg(long)]
    project: Option<String>,
}

pub(crate) async fn run(daemon: &DaemonClient<'_>, args: &UploadArgs) -> Result<(), CliError> {
    let project = args.project.clone();
    let path = std::fs::canonicalize(&args.path)
        .map_err(|_| CliError::Other(format!("File not found or unsupported: {}", args.path)))?;
    let inserted: NoteSummary = daemon
        .call(AppRequest::NoteUpload {
            path: path.to_string_lossy().into_owned(),
            project: project.clone(),
            created_at: None,
        })
        .await?;

    match project.as_deref() {
        Some(name) => println!(
            "Created note {} in project \"{name}\".",
            display_summary_id(&inserted)
        ),
        None => println!("Created note {}.", display_summary_id(&inserted)),
    }
    Ok(())
}
