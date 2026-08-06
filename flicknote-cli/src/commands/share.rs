use clap::Args;
use flicknote_core::backend::NoteDb;
use flicknote_core::config::Config;
use flicknote_core::error::CliError;
use flicknote_core::services::note::NoteService;
use flicknote_core::services::project::ProjectService;
use flicknote_sync::ipc::DaemonClient;

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

pub(crate) async fn run_note(
    db: &dyn NoteDb,
    config: &Config,
    args: &ShareArgs,
) -> Result<(), CliError> {
    let result = NoteService::new(db)
        .share(&DaemonClient::new(config), &args.id)
        .await?;
    println!("{}", result.url);
    Ok(())
}

pub(crate) async fn run_project(
    db: &dyn NoteDb,
    config: &Config,
    id: &str,
) -> Result<(), CliError> {
    let result = ProjectService::new(db)
        .share(&DaemonClient::new(config), id)
        .await?;
    println!("{}", result.url);
    Ok(())
}

pub(crate) async fn run_unshare_note(
    db: &dyn NoteDb,
    config: &Config,
    args: &UnshareArgs,
) -> Result<(), CliError> {
    NoteService::new(db)
        .unshare(&DaemonClient::new(config), &args.id)
        .await?;
    println!("Share link revoked.");
    Ok(())
}

pub(crate) async fn run_unshare_project(
    db: &dyn NoteDb,
    config: &Config,
    id: &str,
) -> Result<(), CliError> {
    ProjectService::new(db)
        .unshare(&DaemonClient::new(config), id)
        .await?;
    println!("Share link revoked.");
    Ok(())
}
