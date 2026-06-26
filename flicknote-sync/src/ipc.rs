use std::fmt;
use std::path::PathBuf;

use flicknote_core::config::Config;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;

pub const LOCAL_SYNC_TIMEOUT_SECS: u64 = 10;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", content = "payload", rename_all = "snake_case")]
pub enum DaemonRequest {
    CreateNote(CreateNoteRequest),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CreateNoteRequest {
    pub id: String,
    pub note_type: String,
    pub status: String,
    pub title: Option<String>,
    pub content: Option<String>,
    pub metadata: Option<String>,
    pub project_id: Option<String>,
    pub now: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub topics: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub entities: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", content = "payload", rename_all = "snake_case")]
pub enum DaemonResponse {
    NoteCreated(CreatedNote),
    Error(DaemonError),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CreatedNote {
    pub uuid: String,
    pub short_id: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "code", rename_all = "snake_case")]
pub enum DaemonError {
    RemoteCreatedLocalSyncTimeout { short_id: i64, timeout_secs: u64 },
    Other { message: String },
}

impl fmt::Display for DaemonError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RemoteCreatedLocalSyncTimeout {
                short_id,
                timeout_secs,
            } => write!(
                f,
                "Created note remotely as #{short_id}, but PowerSync did not update the local database within {timeout_secs}s.\nDo not create it again. Check `flicknote sync status`; note #{short_id} should appear after sync catches up."
            ),
            Self::Other { message } => f.write_str(message),
        }
    }
}

impl std::error::Error for DaemonError {}

pub fn socket_path(config: &Config) -> PathBuf {
    config.paths.data_dir.join("sync.sock")
}

pub async fn send_request(
    config: &Config,
    request: &DaemonRequest,
) -> Result<DaemonResponse, DaemonError> {
    let path = socket_path(config);
    let mut stream = UnixStream::connect(&path)
        .await
        .map_err(|e| DaemonError::Other {
            message: format!("Sync daemon is not available at {}: {e}", path.display()),
        })?;
    write_json(&mut stream, request).await?;
    let mut buf = Vec::new();
    stream
        .read_to_end(&mut buf)
        .await
        .map_err(|e| DaemonError::Other {
            message: format!("Failed to read daemon response: {e}"),
        })?;
    serde_json::from_slice(&buf).map_err(|e| DaemonError::Other {
        message: format!("Failed to parse daemon response: {e}"),
    })
}

pub async fn read_request(stream: &mut UnixStream) -> Result<DaemonRequest, DaemonError> {
    let mut buf = Vec::new();
    stream
        .read_to_end(&mut buf)
        .await
        .map_err(|e| DaemonError::Other {
            message: format!("Failed to read daemon request: {e}"),
        })?;
    serde_json::from_slice(&buf).map_err(|e| DaemonError::Other {
        message: format!("Failed to parse daemon request: {e}"),
    })
}

pub async fn write_response(
    stream: &mut UnixStream,
    response: &DaemonResponse,
) -> Result<(), DaemonError> {
    write_json(stream, response).await
}

async fn write_json<T: Serialize>(stream: &mut UnixStream, value: &T) -> Result<(), DaemonError> {
    let bytes = serde_json::to_vec(value).map_err(|e| DaemonError::Other {
        message: format!("Failed to serialize daemon message: {e}"),
    })?;
    stream
        .write_all(&bytes)
        .await
        .map_err(|e| DaemonError::Other {
            message: format!("Failed to write daemon message: {e}"),
        })?;
    stream.shutdown().await.map_err(|e| DaemonError::Other {
        message: format!("Failed to close daemon message: {e}"),
    })
}

#[cfg(test)]
mod tests {
    use flicknote_core::config::{Config, ConfigPaths};
    use serde_json::json;

    use super::*;

    #[test]
    fn socket_path_lives_in_data_dir() {
        let suffix = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "flicknote-ipc-test-{}-{suffix}",
            std::process::id()
        ));
        let config = Config {
            supabase_url: String::new(),
            supabase_anon_key: String::new(),
            powersync_url: String::new(),
            api_url: String::new(),
            web_url: None,
            paths: ConfigPaths {
                config_dir: dir.clone(),
                data_dir: dir.clone(),
                config_file: dir.join("config.json"),
                session_file: dir.join("session.json"),
                db_file: dir.join("flicknote.db"),
                log_file: dir.join("sync.log"),
            },
        };

        assert_eq!(socket_path(&config), dir.join("sync.sock"));
    }

    #[test]
    fn create_note_request_serializes_as_tagged_json() {
        let req = DaemonRequest::CreateNote(CreateNoteRequest {
            id: "note-id".to_string(),
            note_type: "normal".to_string(),
            status: "ai_queued".to_string(),
            title: Some("Title".to_string()),
            content: Some("Body".to_string()),
            metadata: None,
            project_id: Some("project-id".to_string()),
            now: "2026-06-26T00:00:00Z".to_string(),
            topics: vec!["rust".to_string()],
            entities: vec!["PowerSync".to_string()],
        });

        assert_eq!(
            serde_json::to_value(req).unwrap(),
            json!({
                "type": "create_note",
                "payload": {
                    "id": "note-id",
                    "note_type": "normal",
                    "status": "ai_queued",
                    "title": "Title",
                    "content": "Body",
                    "metadata": null,
                    "project_id": "project-id",
                    "now": "2026-06-26T00:00:00Z",
                    "topics": ["rust"],
                    "entities": ["PowerSync"]
                }
            })
        );
    }

    #[test]
    fn local_sync_timeout_message_warns_not_to_create_again() {
        let err = DaemonError::RemoteCreatedLocalSyncTimeout {
            short_id: 123,
            timeout_secs: 10,
        };

        assert_eq!(
            err.to_string(),
            "Created note remotely as #123, but PowerSync did not update the local database within 10s.\nDo not create it again. Check `flicknote sync status`; note #123 should appear after sync catches up."
        );
    }
}
