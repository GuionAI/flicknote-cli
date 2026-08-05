use clap::Args;
use flicknote_core::backend::NoteDb;
use flicknote_core::config::Config;
use flicknote_core::error::CliError;

use flicknote_core::services::note::NoteService;

use super::util::{display_summary_id, read_stdin_required};

const APPEND_HELP: &str = include_str!("../help/append.md");

#[derive(Args)]
#[command(after_help = APPEND_HELP)]
pub(crate) struct AppendArgs {
    /// Note ID. Use the numeric short ID shown in list/detail. Full UUIDs are also accepted for compatibility.
    id: String,
}

pub(crate) async fn run(
    db: &dyn NoteDb,
    _config: &Config,
    args: &AppendArgs,
) -> Result<(), CliError> {
    let new_content = read_stdin_required()?;
    let result = NoteService::new(db).append(&args.id, &new_content).await?;
    println!("Appended to note {}.", display_summary_id(&result.note));
    Ok(())
}
