use clap::Args;
use flicknote_core::error::CliError;
use flicknote_core::services::dto::{NoteModifyInput, NoteMutationResult};
use flicknote_core::services::edit_match::{is_edit_mode, parse_edit_input};
use flicknote_sync::ipc::{AppRequest, DaemonClient};

use super::util::{display_summary_id, print_section_tree, try_read_stdin};

const MODIFY_HELP: &str = include_str!("../help/modify.md");

#[derive(Args)]
#[command(after_help = MODIFY_HELP)]
pub(crate) struct ModifyArgs {
    /// Note ID. Use the numeric short ID shown in list/detail. Full UUIDs are also accepted for compatibility.
    id: String,
    /// Edit only the named section (scope = full section including heading)
    #[arg(short = 's', long = "section")]
    section: Option<String>,
    /// Move note to this project
    #[arg(short = 'p', long = "project")]
    project: Option<String>,
    /// Mark note as flagged
    #[arg(long, conflicts_with = "unflagged")]
    flagged: bool,
    /// Remove flagged status
    #[arg(long, conflicts_with = "flagged")]
    unflagged: bool,
}

pub(crate) async fn run(daemon: &DaemonClient<'_>, args: &ModifyArgs) -> Result<(), CliError> {
    let piped = try_read_stdin()?;
    if let Some(input) = piped.as_deref()
        && !is_edit_mode(input)
    {
        return Err(CliError::Other(
            "stdin doesn't look like edit mode (===BEFORE===/===AFTER===). \
             Use `flicknote replace <id> --section <section>` for section overwrite."
                .into(),
        ));
    }
    let (before, after) = match piped.as_deref() {
        Some(input) => {
            let (before, after) = parse_edit_input(input)?;
            (Some(before), Some(after))
        }
        None => (None, None),
    };
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
            before,
            after,
            section: args.section.clone(),
            project: args.project.clone(),
            flagged,
        }))
        .await?;

    println!("Modified note {}.\n", display_summary_id(&result.note));
    print_section_tree(&result.sections);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::util::classify_stdin_buf;

    #[test]
    fn test_classify_stdin_buf_via_util() {
        assert_eq!(classify_stdin_buf("  \n  "), None);
        assert_eq!(classify_stdin_buf("x"), Some("x".to_string()));
        assert_eq!(classify_stdin_buf(" foo "), Some(" foo".to_string()));
    }
}
