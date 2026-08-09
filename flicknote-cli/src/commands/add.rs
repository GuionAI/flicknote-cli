use clap::Args;
use flicknote_core::error::CliError;
use flicknote_core::services::dto::{NoteAddInput, NoteSummary};
use flicknote_sync::ipc::{AppRequest, DaemonClient};
use std::io::{IsTerminal, Read};

use super::util::{display_summary_id, resolve_project_arg};

const ADD_HELP: &str = include_str!("../help/add.md");

#[derive(Args)]
#[command(after_help = ADD_HELP)]
pub(crate) struct AddArgs {
    /// Note content or URL. Reads from stdin if omitted.
    value: Option<String>,
    /// Assign to project by name
    #[arg(long)]
    project: Option<String>,
}

pub(crate) async fn run(daemon: &DaemonClient<'_>, args: &AddArgs) -> Result<(), CliError> {
    let content = match &args.value {
        Some(v) => v.to_owned(),
        None => {
            if std::io::stdin().is_terminal() {
                return Err(CliError::Other(
                    "No content provided. Pass a value or pipe from stdin.".into(),
                ));
            }
            let mut buf = String::new();
            std::io::stdin().read_to_string(&mut buf)?;
            let trimmed = buf.trim_end().to_string();
            if trimmed.is_empty() {
                return Err(CliError::Other("No content provided".into()));
            }
            trimmed
        }
    };

    let project = resolve_project_arg(&args.project);
    let note: NoteSummary = daemon
        .call(AppRequest::NoteAdd(NoteAddInput {
            content,
            project: project.clone(),
            interpret_as_url: args.value.is_some(),
            topics: Vec::new(),
            created_at: None,
        }))
        .await?;
    match project.as_deref() {
        Some(name) => println!(
            "Created note {} in project \"{name}\".",
            display_summary_id(&note)
        ),
        None => println!("Created note {}.", display_summary_id(&note)),
    }
    Ok(())
}
