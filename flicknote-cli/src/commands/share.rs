use clap::Args;
use flicknote_core::backend::NoteDb;
use flicknote_core::config::Config;
use flicknote_core::error::CliError;
use flicknote_sync::ipc::{
    DaemonRequest, DaemonResponse, ShareRequest, ShareResource, send_request,
};

const SHARE_HELP: &str = include_str!("../help/share.md");
const UNSHARE_HELP: &str = include_str!("../help/unshare.md");

#[derive(Args)]
#[command(after_help = SHARE_HELP)]
pub(crate) struct ShareArgs {
    /// Note ID. Use the numeric short ID shown in list/detail. Full UUIDs are also accepted for compatibility.
    pub(crate) id: String,
}

#[derive(Args)]
#[command(after_help = UNSHARE_HELP)]
pub(crate) struct UnshareArgs {
    /// Note ID. Use the numeric short ID shown in list/detail. Full UUIDs are also accepted for compatibility.
    pub(crate) id: String,
}

async fn share(config: &Config, resource: ShareResource, id: String) -> Result<String, CliError> {
    let request = DaemonRequest::GetOrCreateShare(ShareRequest { resource, id });
    match send_request(config, &request)
        .await
        .map_err(|error| CliError::Other(error.to_string()))?
    {
        DaemonResponse::ShareUrl(response) => Ok(response.url),
        DaemonResponse::Error(error) => Err(CliError::Other(error.to_string())),
        _ => Err(CliError::Other(
            "Sync daemon returned an unexpected response to the share request".into(),
        )),
    }
}

async fn unshare(config: &Config, resource: ShareResource, id: String) -> Result<(), CliError> {
    let request = DaemonRequest::RevokeShare(ShareRequest { resource, id });
    match send_request(config, &request)
        .await
        .map_err(|error| CliError::Other(error.to_string()))?
    {
        DaemonResponse::ShareRevoked => Ok(()),
        DaemonResponse::Error(error) => Err(CliError::Other(error.to_string())),
        _ => Err(CliError::Other(
            "Sync daemon returned an unexpected response to the unshare request".into(),
        )),
    }
}

pub(crate) async fn run_note(
    db: &dyn NoteDb,
    config: &Config,
    args: &ShareArgs,
) -> Result<(), CliError> {
    let id = db.resolve_note_id(&args.id).await?;
    let url = share(config, ShareResource::Note, id).await?;
    println!("{url}");
    Ok(())
}

pub(crate) async fn run_project(
    db: &dyn NoteDb,
    config: &Config,
    id: &str,
) -> Result<(), CliError> {
    let id = db.resolve_project_id(id).await?;
    let url = share(config, ShareResource::Project, id).await?;
    println!("{url}");
    Ok(())
}

pub(crate) async fn run_unshare_note(
    db: &dyn NoteDb,
    config: &Config,
    args: &UnshareArgs,
) -> Result<(), CliError> {
    let id = db.resolve_note_id(&args.id).await?;
    unshare(config, ShareResource::Note, id).await?;
    println!("Share link revoked.");
    Ok(())
}

pub(crate) async fn run_unshare_project(
    db: &dyn NoteDb,
    config: &Config,
    id: &str,
) -> Result<(), CliError> {
    let id = db.resolve_project_id(id).await?;
    unshare(config, ShareResource::Project, id).await?;
    println!("Share link revoked.");
    Ok(())
}
