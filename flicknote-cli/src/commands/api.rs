use clap::{Args, Subcommand};
use flicknote_core::config::Config;
use flicknote_core::error::CliError;
use std::path::PathBuf;

use crate::api_client::ApiClient;

#[derive(Args)]
pub struct ApiArgs {
    #[command(subcommand)]
    pub command: ApiCommands,
}

#[derive(Subcommand)]
pub enum ApiCommands {
    /// Upload a file attachment
    Upload(UploadArgs),
    /// Download an attachment by ID
    Download(DownloadArgs),
    /// Delete an attachment by ID
    Delete(DeleteArgs),
    /// Create a link note
    Link(LinkArgs),
    /// Create a text note
    Note(NoteArgs),
}

#[derive(Args)]
pub struct UploadArgs {
    /// Path to the file to upload
    pub file: PathBuf,
}

#[derive(Args)]
pub struct DownloadArgs {
    /// Attachment ID
    pub id: String,
    /// Output file path (defaults to original filename from server, or attachment ID)
    #[arg(short, long)]
    pub output: Option<PathBuf>,
}

#[derive(Args)]
pub struct DeleteArgs {
    /// Attachment ID
    pub id: String,
}

#[derive(Args)]
pub struct LinkArgs {
    /// URL to save
    pub url: String,
    /// Optional title
    #[arg(short, long)]
    pub title: Option<String>,
}

#[derive(Args)]
pub struct NoteArgs {
    /// Note content
    pub content: String,
    /// Optional title
    #[arg(short, long)]
    pub title: Option<String>,
}

pub fn run(config: &Config, args: &ApiArgs) -> Result<(), CliError> {
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(run_async(config, args))
}

async fn run_async(config: &Config, args: &ApiArgs) -> Result<(), CliError> {
    let client = ApiClient::new(config)?;

    match &args.command {
        ApiCommands::Upload(a) => cmd_upload(&client, a).await,
        ApiCommands::Download(a) => cmd_download(&client, a).await,
        ApiCommands::Delete(a) => cmd_delete(&client, a).await,
        ApiCommands::Link(a) => cmd_link(&client, a).await,
        ApiCommands::Note(a) => cmd_note(&client, a).await,
    }
}

async fn cmd_upload(client: &ApiClient, args: &UploadArgs) -> Result<(), CliError> {
    if !args.file.exists() {
        return Err(CliError::Other(format!(
            "File not found: {}",
            args.file.display()
        )));
    }

    println!("Uploading {}...", args.file.display());
    let resp = client.upload_file(&args.file).await?;

    if let Some(id) = &resp.id {
        println!("Uploaded — attachment ID: {id}");
    } else if let Some(msg) = &resp.message {
        println!("{msg}");
    } else {
        println!("Upload completed.");
    }
    Ok(())
}

async fn cmd_download(client: &ApiClient, args: &DownloadArgs) -> Result<(), CliError> {
    let output_path = match &args.output {
        Some(p) => p.clone(),
        None => {
            // Try to get original filename from Content-Disposition header
            let filename = client
                .get_attachment_filename(&args.id)
                .await
                .unwrap_or(None)
                .unwrap_or_else(|| args.id.clone());
            PathBuf::from(filename)
        }
    };

    println!("Downloading attachment {}...", args.id);
    let bytes_written = client.download_attachment(&args.id, &output_path).await?;
    println!(
        "Saved to {} ({} bytes)",
        output_path.display(),
        bytes_written
    );
    Ok(())
}

async fn cmd_delete(client: &ApiClient, args: &DeleteArgs) -> Result<(), CliError> {
    println!("Deleting attachment {}...", args.id);
    let resp = client.delete_attachment(&args.id).await?;

    if let Some(msg) = &resp.message {
        println!("{msg}");
    } else {
        println!("Deleted.");
    }
    Ok(())
}

async fn cmd_link(client: &ApiClient, args: &LinkArgs) -> Result<(), CliError> {
    println!("Creating link note...");
    let resp = client.create_link(&args.url, args.title.as_deref()).await?;

    if let Some(id) = &resp.id {
        println!("Created link note — ID: {id}");
    } else if let Some(msg) = &resp.message {
        println!("{msg}");
    } else {
        println!("Link created.");
    }
    Ok(())
}

async fn cmd_note(client: &ApiClient, args: &NoteArgs) -> Result<(), CliError> {
    println!("Creating note...");
    let resp = client
        .create_note(&args.content, args.title.as_deref())
        .await?;

    if let Some(id) = &resp.id {
        println!("Created note — ID: {id}");
    } else if let Some(msg) = &resp.message {
        println!("{msg}");
    } else {
        println!("Note created.");
    }
    Ok(())
}
