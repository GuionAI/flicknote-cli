use clap::Args;
use flicknote_core::error::CliError;
use flicknote_core::services::dto::{NoteModifyInput, NoteMutationResult};
use flicknote_sync::ipc::{AppRequest, DaemonClient};

use super::util::{display_summary_id, print_section_tree};

const MODIFY_HELP: &str = include_str!("../help/modify.md");

#[derive(Args)]
#[command(
    group(clap::ArgGroup::new("metadata").required(true).multiple(true)),
    after_help = MODIFY_HELP
)]
pub(crate) struct ModifyArgs {
    /// Note ID. Use the numeric short ID shown in list/detail. Full UUIDs are also accepted for compatibility.
    id: String,
    /// Move note to this project
    #[arg(short = 'p', long = "project", group = "metadata")]
    project: Option<String>,
    /// Mark note as flagged
    #[arg(long, group = "metadata", conflicts_with = "unflagged")]
    flagged: bool,
    /// Remove flagged status
    #[arg(long, group = "metadata", conflicts_with = "flagged")]
    unflagged: bool,
}

pub(crate) async fn run(daemon: &DaemonClient<'_>, args: &ModifyArgs) -> Result<(), CliError> {
    let flagged = if args.flagged {
        Some(true)
    } else if args.unflagged {
        Some(false)
    } else {
        None
    };
    let result: NoteMutationResult = daemon
        .call(AppRequest::NoteModify(NoteModifyInput {
            id: args.id.clone(),
            before: None,
            after: None,
            section: None,
            project: args.project.clone(),
            flagged,
        }))
        .await?;

    println!("Modified note {}.\n", display_summary_id(&result.note));
    print_section_tree(&result.sections);
    Ok(())
}
