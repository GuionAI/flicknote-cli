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
    /// Upload a file attachment to a note
    Upload(UploadArgs),
    /// Download a note's attachment
    Download(DownloadArgs),
    /// Delete a note's attachment
    Delete(DeleteArgs),
}

#[derive(Args)]
pub struct UploadArgs {
    /// Note ID to attach the file to
    pub note_id: String,
    /// Path to the file to upload
    pub file: PathBuf,
}

#[derive(Args)]
pub struct DownloadArgs {
    /// Note ID to download attachment from
    pub note_id: String,
    /// Output file path
    #[arg(short, long)]
    pub output: Option<PathBuf>,
}

#[derive(Args)]
pub struct DeleteArgs {
    /// Note ID to delete attachment from
    pub note_id: String,
}

pub fn run(config: &Config, args: &ApiArgs) -> Result<(), CliError> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    rt.block_on(run_async(config, args))
}

async fn run_async(config: &Config, args: &ApiArgs) -> Result<(), CliError> {
    let client = ApiClient::new(config).await?;

    match &args.command {
        ApiCommands::Upload(a) => {
            if !a.file.exists() {
                return Err(CliError::Other(format!(
                    "File not found: {}",
                    a.file.display()
                )));
            }
            println!("Uploading {}...", a.file.display());
            client.upload_file(&a.note_id, &a.file).await?;
            println!("Uploaded successfully");
            Ok(())
        }
        ApiCommands::Download(a) => {
            let output = a
                .output
                .clone()
                .unwrap_or_else(|| PathBuf::from(format!("{}.bin", a.note_id)));
            println!("Downloading attachment for note {}...", a.note_id);
            let bytes = client.download_attachment(&a.note_id, &output).await?;
            println!("Saved to {} ({} bytes)", output.display(), bytes);
            Ok(())
        }
        ApiCommands::Delete(a) => {
            println!("Deleting attachment for note {}...", a.note_id);
            client.delete_attachment(&a.note_id).await?;
            println!("Deleted.");
            Ok(())
        }
    }
}
