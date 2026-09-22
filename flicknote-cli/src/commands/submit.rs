use clap::Args;
use flicknote_core::error::CliError;
use flicknote_core::services::dto::NoteMutationResult;
use flicknote_sync::ipc::{AppRequest, DaemonClient};

use super::util::display_summary_id;

const SUBMIT_HELP: &str = include_str!("../help/submit.md");

#[derive(Args)]
#[command(after_help = SUBMIT_HELP)]
pub(crate) struct SubmitArgs {
    /// Draft note ID. Use the numeric short ID shown in list/detail.
    id: String,
    /// Output the canonical mutation result as JSON
    #[arg(long)]
    json: bool,
}

pub(crate) async fn run(daemon: &DaemonClient<'_>, args: &SubmitArgs) -> Result<(), CliError> {
    let result: NoteMutationResult = daemon
        .call(AppRequest::NoteSubmit {
            id: args.id.clone(),
        })
        .await?;
    if args.json {
        println!(
            "{}",
            serde_json::to_string(&result).map_err(CliError::Json)?
        );
    } else {
        println!("Submitted draft note {}.", display_summary_id(&result.note));
    }
    Ok(())
}
