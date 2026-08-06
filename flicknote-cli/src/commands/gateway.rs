use clap::{Args, Subcommand};
use flicknote_core::config::Config;
use flicknote_core::error::CliError;
use reqwest::Method;
use std::io::{IsTerminal, Read, Write};

use crate::gateway::{GatewayClient, GatewayRequestError};

#[derive(Args)]
pub(crate) struct GatewayArgs {
    #[command(subcommand)]
    command: GatewayCommand,
}

#[derive(Subcommand)]
enum GatewayCommand {
    /// Make an authenticated request to a path on the configured Gateway origin
    Request(GatewayRequestArgs),
}

#[derive(Args)]
struct GatewayRequestArgs {
    /// HTTP method to use
    #[arg(long, default_value = "GET")]
    method: String,
    /// Absolute path on the configured Gateway origin (for example /web/v1/search)
    #[arg(long)]
    path: String,
    /// JSON request body. Use without a value to read JSON from stdin.
    #[arg(long, num_args = 0..=1, default_missing_value = "-", conflicts_with = "stdin")]
    json: Option<String>,
    /// Read the request body from stdin even when stdin is a terminal.
    #[arg(long)]
    stdin: bool,
}

pub(crate) async fn run(config: &Config, args: &GatewayArgs) -> Result<(), CliError> {
    match &args.command {
        GatewayCommand::Request(args) => request(config, args).await,
    }
}

async fn request(config: &Config, args: &GatewayRequestArgs) -> Result<(), CliError> {
    let method = Method::from_bytes(args.method.as_bytes())
        .map_err(|_| CliError::Other("--method must be a valid HTTP method".into()))?;
    let client = GatewayClient::new(config)?;
    let mut response = match args.json.as_deref() {
        Some("-") => {
            let body = read_stdin(true)?
                .ok_or_else(|| CliError::Other("No request body provided on stdin".into()))?;
            let _: serde_json::Value = serde_json::from_slice(&body).map_err(CliError::Json)?;
            client
                .request_json_bytes(method, &args.path, &body)
                .await
                .map_err(GatewayRequestError::into_cli_error)?
        }
        Some(body) => {
            let body = serde_json::from_str(body).map_err(CliError::Json)?;
            client
                .request_json(method, &args.path, &body)
                .await
                .map_err(GatewayRequestError::into_cli_error)?
        }
        None => {
            let body = read_stdin(args.stdin)?;
            client
                .request(method, &args.path, body.as_deref())
                .await
                .map_err(GatewayRequestError::into_cli_error)?
        }
    };

    eprintln!("Gateway response: {}", response.status());
    let stdout = std::io::stdout();
    let mut stdout = stdout.lock();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| CliError::Http("Gateway response interrupted".into()))?
    {
        stdout.write_all(&chunk)?;
        stdout.flush()?;
    }
    Ok(())
}

fn read_stdin(required: bool) -> Result<Option<Vec<u8>>, CliError> {
    if std::io::stdin().is_terminal() && !required {
        return Ok(None);
    }
    let mut body = Vec::new();
    std::io::stdin().read_to_end(&mut body)?;
    if body.is_empty() && required {
        return Err(CliError::Other("No request body provided on stdin".into()));
    }
    Ok((!body.is_empty()).then_some(body))
}
