use clap::Args;
use flicknote_core::error::CliError;
use flicknote_core::services::dto::CaptureReceipt;
use flicknote_sync::ipc::{AppRequest, DaemonClient};

use super::util::read_stdin_required;

#[derive(Args)]
pub(crate) struct CaptureArgs {
    /// Exact capture text. Reads from stdin if omitted.
    text: Option<String>,
    /// Output the stable capture receipt as JSON
    #[arg(long)]
    json: bool,
}

pub(crate) async fn run(daemon: &DaemonClient<'_>, args: &CaptureArgs) -> Result<(), CliError> {
    let text = match &args.text {
        Some(text) if !text.trim().is_empty() => text.clone(),
        Some(_) => return Err(CliError::Other("Capture text must not be empty".into())),
        None => read_stdin_required()?,
    };
    let receipt: CaptureReceipt = daemon.call(AppRequest::Capture { text }).await?;
    if args.json {
        println!(
            "{}",
            serde_json::to_string(&receipt).map_err(CliError::Json)?
        );
    } else {
        let id = receipt
            .daily_short_id
            .map_or_else(|| receipt.daily_uuid.clone(), |id| id.to_string());
        println!(
            "Captured in Daily {id} (routing comment {}).",
            receipt.routing_comment_uuid
        );
    }
    Ok(())
}
