use clap::{Args, Subcommand};
use flicknote_core::error::CliError;
use flicknote_core::services::dto::{CommentCreateInput, CommentDto, CommentModifyInput};
use flicknote_sync::ipc::{AppRequest, DaemonClient};

#[derive(Args)]
pub(crate) struct CommentArgs {
    #[command(subcommand)]
    command: CommentCommands,
}

#[derive(Subcommand)]
enum CommentCommands {
    /// List comments for a note in chronological order
    List { note_id: String },
    /// Create a root or reply comment
    Add {
        note_id: String,
        block_text: String,
        content: String,
        #[arg(long)]
        author: String,
        #[arg(long)]
        parent_id: Option<String>,
        #[arg(long)]
        read: bool,
    },
    /// Patch comment JSON content and/or read state
    Modify {
        id: String,
        #[arg(long)]
        content: Option<String>,
        #[arg(long)]
        is_read: Option<bool>,
    },
}

pub(crate) async fn run(daemon: &DaemonClient<'_>, args: &CommentArgs) -> Result<(), CliError> {
    match &args.command {
        CommentCommands::List { note_id } => {
            let comments: Vec<CommentDto> = daemon
                .call(AppRequest::CommentList {
                    note_id: note_id.clone(),
                })
                .await?;
            println!(
                "{}",
                serde_json::to_string_pretty(&comments).map_err(CliError::Json)?
            );
        }
        CommentCommands::Add {
            note_id,
            block_text,
            content,
            author,
            parent_id,
            read,
        } => {
            let comment: CommentDto = daemon
                .call(AppRequest::CommentCreate(CommentCreateInput {
                    note_id: note_id.clone(),
                    block_text: block_text.clone(),
                    content: serde_json::from_str(content).map_err(CliError::Json)?,
                    author: author.clone(),
                    parent_id: parent_id.clone(),
                    is_read: *read,
                }))
                .await?;
            println!(
                "{}",
                serde_json::to_string(&comment).map_err(CliError::Json)?
            );
        }
        CommentCommands::Modify {
            id,
            content,
            is_read,
        } => {
            let content = content
                .as_deref()
                .map(serde_json::from_str)
                .transpose()
                .map_err(CliError::Json)?;
            let comment: CommentDto = daemon
                .call(AppRequest::CommentModify(CommentModifyInput {
                    id: id.clone(),
                    content,
                    is_read: *is_read,
                }))
                .await?;
            println!(
                "{}",
                serde_json::to_string(&comment).map_err(CliError::Json)?
            );
        }
    }
    Ok(())
}
