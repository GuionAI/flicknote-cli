use clap::Args;
use flicknote_core::error::CliError;
use flicknote_core::services::dto::NoteMutationResult;
use flicknote_sync::ipc::{AppRequest, DaemonClient};

use super::util::{display_summary_id, read_stdin_required};

const APPEND_HELP: &str = include_str!("../help/append.md");

#[derive(Args)]
#[command(after_help = APPEND_HELP)]
pub(crate) struct AppendArgs {
    /// Note ID. Use the numeric short ID shown in list/detail. Full UUIDs are also accepted for compatibility.
    id: String,
}

pub(crate) async fn run(daemon: &DaemonClient<'_>, args: &AppendArgs) -> Result<(), CliError> {
    let new_content = read_stdin_required()?;
    let result: NoteMutationResult = daemon
        .call(AppRequest::NoteAppend {
            id: args.id.clone(),
            content: new_content,
        })
        .await?;
    println!("Appended to note {}.", display_summary_id(&result.note));
    Ok(())
}
