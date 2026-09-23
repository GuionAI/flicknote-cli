use clap::Args;
use flicknote_core::error::CliError;
use flicknote_core::services::dto::{NoteAddInput, NoteCreateResult};
use flicknote_sync::ipc::{AppRequest, DaemonClient};
use std::io::{IsTerminal, Read};

use super::util::resolve_project_arg;

const ADD_HELP: &str = include_str!("../help/add.md");

#[derive(Args)]
#[command(after_help = ADD_HELP)]
pub(crate) struct AddArgs {
    /// Note content or URL. Reads from stdin if omitted.
    value: Option<String>,
    /// Assign to project by name
    #[arg(long)]
    project: Option<String>,
    /// Create a normal note in draft lifecycle state
    #[arg(long)]
    draft: bool,
    /// Output the created note ID as JSON
    #[arg(long)]
    json: bool,
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
    let note: NoteCreateResult = daemon
        .call(AppRequest::NoteAdd(NoteAddInput {
            content,
            project: project.clone(),
            interpret_as_url: args.value.is_some(),
            draft: args.draft,
            topics: Vec::new(),
            created_at: None,
        }))
        .await?;
    if args.json {
        println!("{}", serde_json::to_string(&note).map_err(CliError::Json)?);
    } else {
        match project.as_deref() {
            Some(name) => println!("Created note {} in project \"{name}\".", note.id),
            None => println!("Created note {}.", note.id),
        }
    }
    Ok(())
}
