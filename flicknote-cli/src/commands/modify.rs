use clap::Args;
use flicknote_core::error::CliError;
use flicknote_core::services::dto::{NoteModifyInput, NoteMutationResult, Patch};
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
    #[arg(
        short = 'p',
        long = "project",
        group = "metadata",
        conflicts_with = "clear_project"
    )]
    project: Option<String>,
    /// Remove the note from its project
    #[arg(long, group = "metadata", conflicts_with = "project")]
    clear_project: bool,
    /// Set the note title
    #[arg(long, group = "metadata", conflicts_with = "clear_title")]
    title: Option<String>,
    /// Clear the note title
    #[arg(long, group = "metadata", conflicts_with = "title")]
    clear_title: bool,
    /// Set the note summary
    #[arg(long, group = "metadata", conflicts_with = "clear_summary")]
    summary: Option<String>,
    /// Clear the note summary
    #[arg(long, group = "metadata", conflicts_with = "summary")]
    clear_summary: bool,
    /// Mark note as flagged
    #[arg(long, group = "metadata", conflicts_with = "unflagged")]
    flagged: bool,
    /// Remove flagged status
    #[arg(long, group = "metadata", conflicts_with = "flagged")]
    unflagged: bool,
}

pub(crate) async fn run(daemon: &DaemonClient<'_>, args: &ModifyArgs) -> Result<(), CliError> {
    let flagged = if args.flagged {
        Patch::Value(true)
    } else if args.unflagged {
        Patch::Value(false)
    } else {
        Patch::Missing
    };
    let result: NoteMutationResult = daemon
        .call(AppRequest::NoteModify(NoteModifyInput {
            id: args.id.clone(),
            before: None,
            after: None,
            section: None,
            title: patch_value_or_clear(&args.title, args.clear_title),
            summary: patch_value_or_clear(&args.summary, args.clear_summary),
            project: patch_value_or_clear(&args.project, args.clear_project),
            flagged,
        }))
        .await?;

    println!("Modified note {}.\n", display_summary_id(&result.note));
    print_section_tree(&result.sections);
    Ok(())
}

fn patch_value_or_clear(value: &Option<String>, clear: bool) -> Patch<String> {
    if let Some(value) = value {
        Patch::Value(value.clone())
    } else if clear {
        Patch::Null
    } else {
        Patch::Missing
    }
}
