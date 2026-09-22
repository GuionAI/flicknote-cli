use clap::Args;
use flicknote_core::error::CliError;
use flicknote_core::services::dto::NoteMutationResult;
use flicknote_sync::ipc::{AppRequest, DaemonClient};

use super::util::{display_summary_id, read_stdin_required};

const WRITE_HELP: &str = include_str!("../help/write.md");

#[derive(Args)]
#[command(after_help = WRITE_HELP)]
pub(crate) struct WriteArgs {
    /// Note ID. Use the numeric short ID shown in list/detail. Full UUIDs are also accepted.
    id: String,
    /// Output the canonical mutation result as JSON
    #[arg(long)]
    json: bool,
}

pub(crate) async fn run(daemon: &DaemonClient<'_>, args: &WriteArgs) -> Result<(), CliError> {
    let content = read_stdin_required()?;
    let result: NoteMutationResult = daemon
        .call(AppRequest::NoteWrite {
            id: args.id.clone(),
            content,
        })
        .await?;
    if args.json {
        println!(
            "{}",
            serde_json::to_string(&result).map_err(CliError::Json)?
        );
    } else {
        println!("Wrote note {}.", display_summary_id(&result.note));
    }
    Ok(())
}
