use clap::Args;
use flicknote_core::config::Config;
use flicknote_core::error::CliError;
use flicknote_core::services::dto::RecallCandidate;
use flicknote_sync::ipc::{AppRequest, DaemonClient};
use serde::Deserialize;
use std::io::{self, Read, Write};
use std::time::Duration;

use super::util::resolve_project_arg;
use crate::recall::{
    McpRecallResult, RECALL_HOOK_EVENT, RECALL_HOOK_TIMEOUT, RECALL_HUMAN_TIMEOUT, current_time,
    normalize_timestamp, recall_call_with_timeout,
};

const HOOK_INPUT_MAX_BYTES: usize = 1024 * 1024;

#[derive(Args)]
#[command(after_help = RECALL_HELP)]
pub(crate) struct RecallArgs {
    /// Text supplied by a person or the current conversation
    #[arg(required_unless_present = "hook", conflicts_with = "hook")]
    query: Option<String>,
    /// Read one Codex UserPromptSubmit event from stdin and emit hook JSON
    #[arg(long, conflicts_with = "query")]
    pub(crate) hook: bool,
    /// Filter by project name
    #[arg(long)]
    project: Option<String>,
}

const RECALL_HELP: &str = include_str!("../help/recall.md");

#[derive(Debug, Deserialize)]
struct CodexPromptEvent {
    hook_event_name: String,
    prompt: String,
}

pub(crate) async fn run(config: &Config, args: &RecallArgs) -> Result<(), CliError> {
    let project = resolve_project_arg(&args.project);
    if args.hook {
        return run_hook(config, project).await;
    }

    let query = args
        .query
        .as_deref()
        .expect("clap requires a query outside hook mode");
    if args.project.is_none()
        && let Some(name) = project.as_deref()
    {
        eprintln!("Filtering by project \"{name}\" from $FLICKNOTE_PROJECT.");
    }
    let candidates = recall_candidates(config, query, project, RECALL_HUMAN_TIMEOUT).await?;
    println!("{}", render_human_candidates(query, &candidates));
    Ok(())
}

async fn run_hook(config: &Config, project: Option<String>) -> Result<(), CliError> {
    let event = read_hook_event(&mut io::stdin().lock())?;
    let candidates = recall_candidates(config, &event.prompt, project, RECALL_HOOK_TIMEOUT).await?;
    let output = serde_json::to_string(&McpRecallResult::from_candidates(
        &candidates,
        current_time(),
    ))
    .map_err(CliError::Json)?;
    let mut stdout = io::stdout().lock();
    writeln!(stdout, "{output}")?;
    Ok(())
}

async fn recall_candidates(
    config: &Config,
    prompt: &str,
    project: Option<String>,
    timeout: Duration,
) -> Result<Vec<RecallCandidate>, CliError> {
    recall_call_with_timeout(
        timeout,
        DaemonClient::new(config).call(AppRequest::NoteRecall {
            prompt: prompt.to_string(),
            project,
        }),
    )
    .await
    .map_err(CliError::from)
}

fn read_hook_event(input: &mut impl Read) -> Result<CodexPromptEvent, CliError> {
    let mut bytes = Vec::new();
    let mut limited = input.take((HOOK_INPUT_MAX_BYTES + 1) as u64);
    limited.read_to_end(&mut bytes)?;
    if bytes.len() > HOOK_INPUT_MAX_BYTES {
        return Err(CliError::Other(format!(
            "Codex hook input exceeds the {HOOK_INPUT_MAX_BYTES}-byte limit"
        )));
    }
    let event: CodexPromptEvent = serde_json::from_slice(&bytes)
        .map_err(|error| CliError::Other(format!("invalid Codex hook input: {error}")))?;
    if event.hook_event_name != RECALL_HOOK_EVENT {
        return Err(CliError::Other(format!(
            "invalid Codex hook event: expected {RECALL_HOOK_EVENT}, got {:?}",
            event.hook_event_name
        )));
    }
    Ok(event)
}

fn render_human_candidates(query: &str, candidates: &[RecallCandidate]) -> String {
    if candidates.is_empty() {
        return if query.is_empty() {
            "No recall candidates found for an empty query.".to_string()
        } else {
            "No recall candidates found.".to_string()
        };
    }

    let mut output = format!("Recall candidates ({}):\n", candidates.len());
    for candidate in candidates {
        let title = candidate
            .title
            .as_deref()
            .filter(|value| !value.is_empty())
            .map(single_line)
            .unwrap_or_else(|| "(untitled)".to_string());
        output.push_str(&format!("- #{} — {title}\n", candidate.id));
        let summary = candidate
            .summary
            .as_deref()
            .filter(|value| !value.is_empty())
            .map(single_line)
            .unwrap_or_else(|| "(none)".to_string());
        output.push_str(&format!("  Summary: {summary}\n"));
        let updated_at = candidate
            .updated_at
            .as_deref()
            .and_then(normalize_timestamp)
            .unwrap_or_else(|| "-".to_string());
        output.push_str(&format!("  Modified: {updated_at}\n"));
    }
    output.trim_end().to_string()
}

fn single_line(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use flicknote_core::config::{Config, ConfigPaths};
    use flicknote_sync::ipc::{
        AppResponse, DaemonResponse, read_request, socket_path, write_response,
    };
    use tokio::net::UnixListener;

    use super::*;

    fn test_config(directory: &Path) -> Config {
        Config {
            supabase_url: String::new(),
            supabase_anon_key: String::new(),
            powersync_url: String::new(),
            api_url: String::new(),
            gateway_url: String::new(),
            web_url: None,
            paths: ConfigPaths {
                config_dir: directory.to_path_buf(),
                data_dir: directory.to_path_buf(),
                config_file: directory.join("config.json"),
                session_file: directory.join("session.json"),
                db_file: directory.join("flicknote.db"),
                log_file: directory.join("daemon.log"),
            },
        }
    }

    fn delayed_recall_daemon(config: &Config, delay: Duration) -> tokio::task::JoinHandle<()> {
        let path = socket_path(config);
        let listener = UnixListener::bind(path).unwrap();
        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            read_request(&mut stream).await.unwrap();
            tokio::time::sleep(delay).await;
            let response = DaemonResponse::App(Box::new(AppResponse::NoteRecall(Vec::new())));
            drop(write_response(&mut stream, &response).await);
        })
    }

    fn candidate(
        id: i64,
        title: Option<&str>,
        summary: Option<&str>,
        updated_at: Option<&str>,
    ) -> RecallCandidate {
        RecallCandidate {
            id,
            title: title.map(str::to_string),
            summary: summary.map(str::to_string),
            updated_at: updated_at.map(str::to_string),
        }
    }

    #[test]
    fn hook_input_requires_the_event_and_prompt_shapes() {
        let event = read_hook_event(&mut r#"{"hook_event_name":"UserPromptSubmit","prompt":"quotes ' and \"\n下一步","cwd":"ignored","project":"ignored"}"#.as_bytes()).unwrap();
        assert_eq!(event.hook_event_name, RECALL_HOOK_EVENT);
        assert_eq!(event.prompt, "quotes ' and \"\n下一步");

        for input in [
            r#"{"prompt":"missing event"}"#,
            r#"{"hook_event_name":"Stop","prompt":"wrong event"}"#,
            r#"{"hook_event_name":"UserPromptSubmit","prompt":7}"#,
            r#"["not an object"]"#,
        ] {
            assert!(
                read_hook_event(&mut input.as_bytes()).is_err(),
                "accepted {input}"
            );
        }
    }

    #[test]
    fn hook_input_rejects_oversized_documents() {
        let input = format!(
            r#"{{"hook_event_name":"UserPromptSubmit","prompt":"{}"}}"#,
            "x".repeat(HOOK_INPUT_MAX_BYTES)
        );
        let error = read_hook_event(&mut input.as_bytes()).unwrap_err();
        assert!(error.to_string().contains("byte limit"));
    }

    #[test]
    fn human_recall_lists_only_candidate_projection() {
        let output = render_human_candidates(
            "query",
            &[candidate(
                42,
                Some("A\nNote"),
                Some("Summary\nwith details"),
                Some("2026-09-10T12:00:00+08:00"),
            )],
        );
        assert!(output.contains("Recall candidates (1):"));
        assert!(output.contains("#42 — A Note"));
        assert!(output.contains("Summary: Summary with details"));
        assert!(output.contains("Modified: 2026-09-10T04:00:00+00:00"));
        assert!(!output.contains("content"));
    }

    #[test]
    fn empty_human_queries_do_not_become_lists() {
        assert_eq!(
            render_human_candidates("", &[]),
            "No recall candidates found for an empty query."
        );
        assert_eq!(
            render_human_candidates("none", &[]),
            "No recall candidates found."
        );
    }

    #[tokio::test]
    async fn recall_entrypoints_keep_their_independent_daemon_budgets() {
        let human_directory = tempfile::tempdir().unwrap();
        let human_config = test_config(human_directory.path());
        let human_server = delayed_recall_daemon(&human_config, Duration::from_millis(100));
        let human_result = recall_candidates(
            &human_config,
            "human query",
            None,
            Duration::from_millis(200),
        )
        .await;
        assert!(
            human_result.is_ok(),
            "human recall failed: {human_result:?}"
        );
        human_server.await.unwrap();

        let hook_directory = tempfile::tempdir().unwrap();
        let hook_config = test_config(hook_directory.path());
        let hook_server = delayed_recall_daemon(&hook_config, Duration::from_millis(50));
        let hook_result = recall_candidates(
            &hook_config,
            "hook prompt",
            None,
            Duration::from_millis(100),
        )
        .await;
        assert!(hook_result.is_ok(), "hook recall failed: {hook_result:?}");
        hook_server.await.unwrap();

        let hook_timeout_directory = tempfile::tempdir().unwrap();
        let hook_timeout_config = test_config(hook_timeout_directory.path());
        let hook_timeout_server =
            delayed_recall_daemon(&hook_timeout_config, Duration::from_millis(200));
        let hook_error = recall_candidates(
            &hook_timeout_config,
            "slow hook prompt",
            None,
            Duration::from_millis(100),
        )
        .await
        .unwrap_err();
        assert!(hook_error.to_string().contains("timed out"));
        hook_timeout_server.await.unwrap();

        let human_timeout_directory = tempfile::tempdir().unwrap();
        let human_timeout_config = test_config(human_timeout_directory.path());
        let human_timeout_server =
            delayed_recall_daemon(&human_timeout_config, Duration::from_millis(300));
        let human_error = recall_candidates(
            &human_timeout_config,
            "slow human query",
            None,
            Duration::from_millis(200),
        )
        .await
        .unwrap_err();
        assert!(human_error.to_string().contains("timed out"));
        human_timeout_server.await.unwrap();
    }
}
