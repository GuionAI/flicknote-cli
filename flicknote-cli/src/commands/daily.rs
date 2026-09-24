use clap::Args;
use flicknote_core::error::CliError;
use flicknote_core::services::dto::DailyReceipt;
use flicknote_sync::ipc::{AppRequest, DaemonClient};

#[derive(Args)]
pub(crate) struct DailyArgs {
    /// Output the Daily receipt as JSON
    #[arg(long)]
    json: bool,
}

pub(crate) async fn run(daemon: &DaemonClient<'_>, args: &DailyArgs) -> Result<(), CliError> {
    let daily: DailyReceipt = daemon.call(AppRequest::DailyGetOrCreate).await?;
    if args.json {
        println!("{}", serde_json::to_string(&daily).map_err(CliError::Json)?);
    } else {
        let id = daily
            .short_id
            .map_or_else(|| daily.uuid.clone(), |id| id.to_string());
        println!("{} ({id})", daily.title);
    }
    Ok(())
}
