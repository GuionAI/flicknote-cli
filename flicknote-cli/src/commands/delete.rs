use clap::Args;
use flicknote_core::error::CliError;
use flicknote_core::services::dto::NoteArchiveResult;
use flicknote_sync::ipc::{AppRequest, DaemonClient};

#[derive(Args)]
pub(crate) struct DeleteArgs {
    /// Note ID. Use the numeric short ID shown in list/detail. Full UUIDs are also accepted for compatibility.
    id: String,
}

pub(crate) async fn run(daemon: &DaemonClient<'_>, args: &DeleteArgs) -> Result<(), CliError> {
    let result: NoteArchiveResult = daemon
        .call(AppRequest::NoteArchive {
            id: args.id.clone(),
        })
        .await?;
    let display_id = result
        .short_id
        .map(|id| id.to_string())
        .unwrap_or(result.uuid);
    println!("Deleted note {}.", display_id);
    Ok(())
}
