use clap::Args;
use flicknote_core::error::CliError;
use flicknote_core::services::dto::OpenResult;
use flicknote_core::services::error::ServiceError;
use flicknote_core::services::ports::BrowserOpener;
use flicknote_sync::ipc::{AppRequest, DaemonClient};

#[derive(Args)]
pub(crate) struct OpenArgs {
    /// Note ID. Use the numeric short ID shown in list/detail. Full UUIDs are also accepted.
    id: String,
}

pub(crate) struct SystemBrowserOpener;

impl BrowserOpener for SystemBrowserOpener {
    fn open(&self, url: &str) -> Result<(), ServiceError> {
        open::that(url).map_err(ServiceError::Io)
    }
}

pub(crate) async fn run(daemon: &DaemonClient<'_>, args: &OpenArgs) -> Result<(), CliError> {
    let result: OpenResult = daemon
        .call(AppRequest::NoteOpen {
            id: args.id.clone(),
        })
        .await?;
    SystemBrowserOpener.open(&result.url)?;
    println!("Opened {}", result.url);
    Ok(())
}
