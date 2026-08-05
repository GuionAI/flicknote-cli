use clap::Args;
use flicknote_core::backend::NoteDb;
use flicknote_core::config::Config;
use flicknote_core::error::CliError;
use flicknote_core::services::note::NoteService;

#[derive(Args)]
pub(crate) struct RestoreArgs {
    /// Note ID. Use the numeric short ID shown in list/detail. Full UUIDs are also accepted for compatibility.
    id: String,
}

pub(crate) async fn run(
    db: &dyn NoteDb,
    _config: &Config,
    args: &RestoreArgs,
) -> Result<(), CliError> {
    let result = NoteService::new(db).restore(&args.id).await?;
    let display_id = result
        .short_id
        .map(|id| id.to_string())
        .unwrap_or(result.uuid);
    println!("Restored note {}.", display_id);
    Ok(())
}
