use clap::Args;
use flicknote_core::error::CliError;
use flicknote_core::services::dto::{ShareResult, UnshareResult};
use flicknote_sync::ipc::{AppRequest, DaemonClient};

const SHARE_HELP: &str = include_str!("../help/share.md");
const UNSHARE_HELP: &str = include_str!("../help/unshare.md");

#[derive(Args)]
#[command(after_help = SHARE_HELP)]
pub(crate) struct ShareArgs {
    /// Note ID. Use the numeric short ID shown in list/detail. Full UUIDs are also accepted.
    pub(crate) id: String,
}

#[derive(Args)]
#[command(after_help = UNSHARE_HELP)]
pub(crate) struct UnshareArgs {
    /// Note ID. Use the numeric short ID shown in list/detail. Full UUIDs are also accepted.
    pub(crate) id: String,
}

pub(crate) async fn run_note(daemon: &DaemonClient<'_>, args: &ShareArgs) -> Result<(), CliError> {
    let result: ShareResult = daemon
        .call(AppRequest::NoteShare {
            id: args.id.clone(),
        })
        .await?;
    println!("{}", result.url);
    Ok(())
}

pub(crate) async fn run_project(daemon: &DaemonClient<'_>, id: &str) -> Result<(), CliError> {
    let result: ShareResult = daemon
        .call(AppRequest::ProjectShare { id: id.to_string() })
        .await?;
    println!("{}", result.url);
    Ok(())
}

pub(crate) async fn run_unshare_note(
    daemon: &DaemonClient<'_>,
    args: &UnshareArgs,
) -> Result<(), CliError> {
    let _: UnshareResult = daemon
        .call(AppRequest::NoteUnshare {
            id: args.id.clone(),
        })
        .await?;
    println!("Share link revoked.");
    Ok(())
}

pub(crate) async fn run_unshare_project(
    daemon: &DaemonClient<'_>,
    id: &str,
) -> Result<(), CliError> {
    let _: UnshareResult = daemon
        .call(AppRequest::ProjectUnshare { id: id.to_string() })
        .await?;
    println!("Share link revoked.");
    Ok(())
}
