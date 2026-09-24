use std::io::{IsTerminal, Read};

use clap::Args;
use flicknote_core::error::CliError;
use flicknote_core::services::dto::CaptureReceipt;
use flicknote_sync::ipc::{AppRequest, DaemonClient};

#[derive(Args)]
pub(crate) struct CaptureArgs {
    /// Exact capture text. Reads from stdin if omitted.
    text: Option<String>,
    /// Output the stable capture receipt as JSON
    #[arg(long)]
    json: bool,
}

pub(crate) async fn run(daemon: &DaemonClient<'_>, args: &CaptureArgs) -> Result<(), CliError> {
    let stdin = std::io::stdin();
    let terminal = stdin.is_terminal();
    let text = capture_text(args.text.as_deref(), stdin.lock(), terminal)?;
    let receipt: CaptureReceipt = daemon.call(AppRequest::Capture { text }).await?;
    if args.json {
        println!(
            "{}",
            serde_json::to_string(&receipt).map_err(CliError::Json)?
        );
    } else {
        let id = receipt
            .daily_short_id
            .map_or_else(|| receipt.daily_uuid.clone(), |id| id.to_string());
        println!(
            "Captured in Daily {id} (routing comment {}).",
            receipt.routing_comment_uuid
        );
    }
    Ok(())
}

fn capture_text(
    positional: Option<&str>,
    mut stdin: impl Read,
    stdin_is_terminal: bool,
) -> Result<String, CliError> {
    if let Some(text) = positional {
        if !stdin_is_terminal {
            let mut piped = String::new();
            stdin.read_to_string(&mut piped)?;
            if !piped.is_empty() {
                return Err(CliError::Other(
                    "Capture text must be provided either positionally or through stdin, not both"
                        .into(),
                ));
            }
        }
        if text.trim().is_empty() {
            return Err(CliError::Other("Capture text must not be empty".into()));
        }
        return Ok(text.to_string());
    }
    if stdin_is_terminal {
        return Err(CliError::Other(
            "No capture text provided. Pass text or pipe it through stdin.".into(),
        ));
    }
    let mut piped = String::new();
    stdin.read_to_string(&mut piped)?;
    if piped.trim().is_empty() {
        return Err(CliError::Other("Capture text must not be empty".into()));
    }
    Ok(piped)
}

#[cfg(test)]
mod tests {
    use super::capture_text;

    #[test]
    fn positional_and_stdin_are_mutually_exclusive() {
        let error = capture_text(Some("positional"), "piped".as_bytes(), false).unwrap_err();
        assert!(error.to_string().contains("not both"));
    }

    #[test]
    fn multiline_stdin_stays_one_exact_capture() {
        let text = capture_text(None, "one\n\ntwo\n".as_bytes(), false).unwrap();
        assert_eq!(text, "one\n\ntwo\n");
    }
}
