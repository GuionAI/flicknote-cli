use clap::{Args, Subcommand};
use flicknote_core::error::CliError;
use flicknote_core::services::dto::{
    CommentBatchModifyInput, CommentCreateInput, CommentDto, CommentModifyInput,
};
use flicknote_sync::ipc::{AppRequest, DaemonClient};

#[derive(Args)]
pub(crate) struct CommentArgs {
    #[command(subcommand)]
    command: CommentCommands,
}

#[derive(Subcommand)]
enum CommentCommands {
    /// List comments for a note in chronological order
    List {
        note_id: String,
        /// Output typed comment records as JSON
        #[arg(long)]
        json: bool,
    },
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
    /// Atomically patch a JSON array of comments read from stdin
    ModifyBatch,
}

pub(crate) async fn run(daemon: &DaemonClient<'_>, args: &CommentArgs) -> Result<(), CliError> {
    match &args.command {
        CommentCommands::List { note_id, json } => {
            let comments: Vec<CommentDto> = daemon
                .call(AppRequest::CommentList {
                    note_id: note_id.clone(),
                })
                .await?;
            if *json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&comments).map_err(CliError::Json)?
                );
            } else {
                for comment in comments {
                    println!("{}\t{}\t{}", comment.id, comment.author, comment.block_text);
                }
            }
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
        CommentCommands::ModifyBatch => {
            let input = super::util::read_stdin_required()?;
            let comments = serde_json::from_str(&input).map_err(CliError::Json)?;
            let comments: Vec<CommentDto> = daemon
                .call(AppRequest::CommentBatchModify(CommentBatchModifyInput {
                    comments,
                }))
                .await?;
            println!(
                "{}",
                serde_json::to_string(&comments).map_err(CliError::Json)?
            );
        }
    }
    Ok(())
}
