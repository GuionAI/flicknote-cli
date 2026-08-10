use clap::Args;
use flicknote_core::error::CliError;
use flicknote_core::services::dto::{NoteArchiveResult, NoteMutationResult};
use flicknote_sync::ipc::{AppRequest, DaemonClient};

use super::util::{display_summary_id, print_section_tree};

#[derive(Args)]
pub(crate) struct DeleteArgs {
    /// Note ID. Use the numeric short ID shown in list/detail. Full UUIDs are also accepted for compatibility.
    id: String,
    /// Remove a specific section by section ID (2-char base62) instead of deleting the note
    #[arg(short = 's', long = "section")]
    section: Option<String>,
}

pub(crate) async fn run(daemon: &DaemonClient<'_>, args: &DeleteArgs) -> Result<(), CliError> {
    if let Some(ref section_id) = args.section {
        let result: NoteMutationResult = daemon
            .call(AppRequest::NoteDeleteSection {
                id: args.id.clone(),
                section: section_id.clone(),
            })
            .await?;
        println!(
            "Removed section {} from note {}.\n",
            section_id,
            display_summary_id(&result.note)
        );
        print_section_tree(&result.sections);
    } else {
        let result: NoteArchiveResult = daemon
            .call(AppRequest::NoteArchive {
                id: args.id.clone(),
            })
            .await?;
        let display_id = result
            .short_id
            .map(|id| id.to_string())
            .unwrap_or(result.uuid);
        println!("Deleted note {}.", display_id);
    }

    Ok(())
}
