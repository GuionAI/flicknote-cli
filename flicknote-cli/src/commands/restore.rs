use clap::Args;
use flicknote_client::dto::NoteArchiveResult;
use flicknote_client::{AppRequest, DaemonClient};
use flicknote_core::error::CliError;

#[derive(Args)]
pub(crate) struct RestoreArgs {
    /// Note ID. Use the numeric short ID shown in list/detail. Full UUIDs are also accepted for compatibility.
    id: String,
}

pub(crate) async fn run(daemon: &DaemonClient, args: &RestoreArgs) -> Result<(), CliError> {
    let result: NoteArchiveResult = daemon
        .call(AppRequest::NoteRestore {
            id: args.id.clone(),
        })
        .await?;
    let display_id = result
        .short_id
        .map(|id| id.to_string())
        .unwrap_or(result.uuid);
    println!("Restored note {}.", display_id);
    Ok(())
}
