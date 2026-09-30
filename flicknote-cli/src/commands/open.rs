use clap::Args;
use flicknote_client::dto::OpenResult;
use flicknote_client::{AppRequest, DaemonClient};
use flicknote_core::error::CliError;
use flicknote_core::services::ports::BrowserOpener;
use flicknote_sync::browser::SystemBrowserOpener;

#[derive(Args)]
pub(crate) struct OpenArgs {
    /// Note ID. Use the numeric short ID shown in list/detail. Full UUIDs are also accepted.
    id: String,
}

pub(crate) async fn run(daemon: &DaemonClient, args: &OpenArgs) -> Result<(), CliError> {
    let result: OpenResult = daemon
        .call(AppRequest::NoteOpen {
            id: args.id.clone(),
        })
        .await?;
    SystemBrowserOpener.open(&result.url)?;
    println!("Opened {}", result.url);
    Ok(())
}
