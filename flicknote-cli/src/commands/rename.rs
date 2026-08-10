use clap::Args;
use flicknote_core::error::CliError;
use flicknote_core::services::dto::NoteMutationResult;
use flicknote_sync::ipc::{AppRequest, DaemonClient};

use super::util::{display_summary_id, print_section_tree};

#[derive(Args)]
pub(crate) struct RenameArgs {
    /// Note ID. Use the numeric short ID shown in list/detail. Full UUIDs are also accepted for compatibility.
    id: String,
    /// Section heading to rename (case-insensitive contains match)
    #[arg(short = 's', long = "section")]
    section: String,
    /// New heading text (without # prefix — level is preserved)
    name: String,
}

pub(crate) async fn run(daemon: &DaemonClient<'_>, args: &RenameArgs) -> Result<(), CliError> {
    let result: NoteMutationResult = daemon
        .call(AppRequest::NoteRenameSection {
            id: args.id.clone(),
            section: args.section.clone(),
            name: args.name.clone(),
        })
        .await?;
    println!(
        "Renamed section {} → '{}' in note {}.\n",
        args.section,
        args.name,
        display_summary_id(&result.note)
    );
    print_section_tree(&result.sections);
    Ok(())
}
