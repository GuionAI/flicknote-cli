use std::fmt;
use std::path::PathBuf;

use async_trait::async_trait;
use flicknote_core::backend::InsertedNote;
use flicknote_core::config::Config;
use flicknote_core::services::error::ServiceError;
use flicknote_core::services::ports::{
    CreateNote, NoteCreator, ShareGateway, ShareResource as CoreShareResource,
};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;

pub const LOCAL_SYNC_TIMEOUT_SECS: u64 = 10;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", content = "payload", rename_all = "snake_case")]
pub enum DaemonRequest {
    CreateNote(Box<CreateNoteRequest>),
    GetOrCreateShare(ShareRequest),
    RevokeShare(ShareRequest),
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ShareResource {
    Note,
    Project,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ShareRequest {
    pub resource: ShareResource,
    pub id: String,
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
    #[serde(default)]
    pub attachment_path: Option<String>,
}

impl CreateNoteRequest {
    pub fn with_attachment_path(mut self, path: impl Into<String>) -> Self {
        self.attachment_path = Some(path.into());
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", content = "payload", rename_all = "snake_case")]
pub enum DaemonResponse {
    NoteCreated(CreatedNote),
    ShareUrl(ShareUrlResponse),
    ShareRevoked,
    Error(DaemonError),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ShareUrlResponse {
    pub url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CreatedNote {
    pub uuid: String,
    pub short_id: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "code", rename_all = "snake_case")]
pub enum DaemonError {
    Unavailable { path: String, message: String },
    RemoteCreatedLocalSyncTimeout { short_id: i64, timeout_secs: u64 },
    Other { message: String },
}

impl fmt::Display for DaemonError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unavailable { path, message } => {
                write!(f, "Sync daemon is not available at {path}: {message}")
            }
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
    let mut stream =
        UnixStream::connect(&path)
            .await
            .map_err(|error| DaemonError::Unavailable {
                path: path.display().to_string(),
                message: error.to_string(),
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

pub struct DaemonClient<'a> {
    config: &'a Config,
}

impl<'a> DaemonClient<'a> {
    pub fn new(config: &'a Config) -> Self {
        Self { config }
    }

    async fn request(&self, request: DaemonRequest) -> Result<DaemonResponse, ServiceError> {
        send_request(self.config, &request)
            .await
            .map_err(|error| match error {
                DaemonError::Unavailable { .. } => ServiceError::DaemonUnavailable(format!(
                    "{error}. Start it with `flicknote sync start`."
                )),
                other => ServiceError::Daemon(other.to_string()),
            })
    }
}

#[async_trait(?Send)]
impl NoteCreator for DaemonClient<'_> {
    async fn create(&self, request: CreateNote) -> Result<InsertedNote, ServiceError> {
        let response = self
            .request(DaemonRequest::CreateNote(Box::new(CreateNoteRequest {
                id: request.id,
                note_type: request.note_type,
                status: request.status,
                title: request.title,
                content: request.content,
                metadata: request.metadata,
                project_id: request.project_id,
                now: request.now,
                topics: request.topics,
                attachment_path: None,
            })))
            .await?;
        match response {
            DaemonResponse::NoteCreated(note) => Ok(InsertedNote {
                uuid: note.uuid,
                short_id: Some(note.short_id),
            }),
            DaemonResponse::Error(error) => Err(ServiceError::Daemon(error.to_string())),
            _ => Err(ServiceError::Internal(
                "sync daemon returned an unexpected create response".to_string(),
            )),
        }
    }
}

#[async_trait(?Send)]
impl ShareGateway for DaemonClient<'_> {
    async fn share(&self, resource: CoreShareResource, id: &str) -> Result<String, ServiceError> {
        let response = self
            .request(DaemonRequest::GetOrCreateShare(ShareRequest {
                resource: resource.into(),
                id: id.to_string(),
            }))
            .await?;
        match response {
            DaemonResponse::ShareUrl(response) => Ok(response.url),
            DaemonResponse::Error(error) => Err(ServiceError::Daemon(error.to_string())),
            _ => Err(ServiceError::Internal(
                "sync daemon returned an unexpected share response".to_string(),
            )),
        }
    }

    async fn unshare(&self, resource: CoreShareResource, id: &str) -> Result<(), ServiceError> {
        let response = self
            .request(DaemonRequest::RevokeShare(ShareRequest {
                resource: resource.into(),
                id: id.to_string(),
            }))
            .await?;
        match response {
            DaemonResponse::ShareRevoked => Ok(()),
            DaemonResponse::Error(error) => Err(ServiceError::Daemon(error.to_string())),
            _ => Err(ServiceError::Internal(
                "sync daemon returned an unexpected unshare response".to_string(),
            )),
        }
    }
}

impl From<CoreShareResource> for ShareResource {
    fn from(resource: CoreShareResource) -> Self {
        match resource {
            CoreShareResource::Note => Self::Note,
            CoreShareResource::Project => Self::Project,
        }
    }
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
    use flicknote_core::services::ports::{
        CreateNote, NoteCreator, ShareGateway, ShareResource as CoreShareResource,
    };
    use serde_json::json;
    use tokio::net::UnixListener;

    use super::*;

    fn test_config(directory: &std::path::Path) -> Config {
        Config {
            supabase_url: String::new(),
            supabase_anon_key: String::new(),
            powersync_url: String::new(),
            api_url: String::new(),
            web_url: None,
            paths: ConfigPaths {
                config_dir: directory.to_path_buf(),
                data_dir: directory.to_path_buf(),
                config_file: directory.join("config.json"),
                session_file: directory.join("session.json"),
                db_file: directory.join("flicknote.db"),
                log_file: directory.join("sync.log"),
            },
        }
    }

    async fn serve_response(
        config: &Config,
        response: DaemonResponse,
    ) -> tokio::task::JoinHandle<DaemonRequest> {
        let listener = UnixListener::bind(socket_path(config)).unwrap();
        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let request = read_request(&mut stream).await.unwrap();
            write_response(&mut stream, &response).await.unwrap();
            request
        })
    }

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
        let req = DaemonRequest::CreateNote(Box::new(CreateNoteRequest {
            id: "note-id".to_string(),
            note_type: "normal".to_string(),
            status: "ai_queued".to_string(),
            title: Some("Title".to_string()),
            content: Some("Body".to_string()),
            metadata: None,
            project_id: Some("project-id".to_string()),
            now: "2026-06-26T00:00:00Z".to_string(),
            topics: vec!["rust".to_string()],
            attachment_path: None,
        }));

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
                    "attachment_path": null
                }
            })
        );
    }

    #[test]
    fn create_note_request_does_not_serialize_entities() {
        let req = DaemonRequest::CreateNote(Box::new(CreateNoteRequest {
            id: "note-id".to_string(),
            note_type: "normal".to_string(),
            status: "ai_queued".to_string(),
            title: Some("Title".to_string()),
            content: Some("Body".to_string()),
            metadata: None,
            project_id: None,
            now: "2026-06-26T00:00:00Z".to_string(),
            topics: vec!["rust".to_string()],
            attachment_path: None,
        }));

        let value = serde_json::to_value(req).unwrap();
        assert_eq!(value["payload"]["topics"], json!(["rust"]));
        assert!(value["payload"].get("entities").is_none());
    }

    #[test]
    fn share_request_deserializes() {
        let value = json!({
            "type": "get_or_create_share",
            "payload": {
                "resource": "note",
                "id": "550e8400-e29b-41d4-a716-446655440000"
            }
        });

        assert!(serde_json::from_value::<DaemonRequest>(value).is_ok());
    }

    #[test]
    fn unshare_request_deserializes() {
        let value = json!({
            "type": "revoke_share",
            "payload": {
                "resource": "project",
                "id": "550e8400-e29b-41d4-a716-446655440000"
            }
        });

        assert!(serde_json::from_value::<DaemonRequest>(value).is_ok());
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

    #[tokio::test]
    async fn daemon_client_maps_missing_socket_to_retryable_unavailable() {
        let directory = tempfile::tempdir().unwrap();
        let config = test_config(directory.path());

        let error = DaemonClient::new(&config)
            .share(CoreShareResource::Note, "note-id")
            .await
            .unwrap_err();

        assert_eq!(error.code(), "daemon_unavailable");
        assert!(error.retryable());
        assert!(error.to_string().contains("flicknote sync start"));
    }

    #[tokio::test]
    async fn daemon_client_maps_create_response_and_unexpected_variant() {
        let directory = tempfile::tempdir().unwrap();
        let config = test_config(directory.path());
        let server = serve_response(
            &config,
            DaemonResponse::NoteCreated(CreatedNote {
                uuid: "created-id".to_string(),
                short_id: 42,
            }),
        )
        .await;
        let request = CreateNote {
            id: "request-id".to_string(),
            note_type: "normal".to_string(),
            status: "ai_queued".to_string(),
            title: Some("Title".to_string()),
            content: Some("Body".to_string()),
            metadata: None,
            project_id: None,
            now: "2026-08-05T00:00:00Z".to_string(),
            topics: Vec::new(),
        };

        let created = DaemonClient::new(&config)
            .create(request.clone())
            .await
            .unwrap();
        assert_eq!(created.uuid, "created-id");
        assert_eq!(created.short_id, Some(42));
        assert!(matches!(
            server.await.unwrap(),
            DaemonRequest::CreateNote(_)
        ));

        let directory = tempfile::tempdir().unwrap();
        let config = test_config(directory.path());
        let server = serve_response(&config, DaemonResponse::ShareRevoked).await;
        let error = DaemonClient::new(&config)
            .create(request)
            .await
            .unwrap_err();
        assert_eq!(error.code(), "internal_error");
        assert!(error.to_string().contains("unexpected create response"));
        server.await.unwrap();
    }

    #[tokio::test]
    async fn daemon_client_maps_share_and_unshare_responses() {
        let directory = tempfile::tempdir().unwrap();
        let config = test_config(directory.path());
        let server = serve_response(
            &config,
            DaemonResponse::ShareUrl(ShareUrlResponse {
                url: "https://share.example/note".to_string(),
            }),
        )
        .await;
        let url = DaemonClient::new(&config)
            .share(CoreShareResource::Note, "note-id")
            .await
            .unwrap();
        assert_eq!(url, "https://share.example/note");
        assert!(matches!(
            server.await.unwrap(),
            DaemonRequest::GetOrCreateShare(_)
        ));

        let directory = tempfile::tempdir().unwrap();
        let config = test_config(directory.path());
        let server = serve_response(&config, DaemonResponse::ShareRevoked).await;
        DaemonClient::new(&config)
            .unshare(CoreShareResource::Project, "project-id")
            .await
            .unwrap();
        assert!(matches!(
            server.await.unwrap(),
            DaemonRequest::RevokeShare(_)
        ));

        let directory = tempfile::tempdir().unwrap();
        let config = test_config(directory.path());
        let server = serve_response(
            &config,
            DaemonResponse::Error(DaemonError::Other {
                message: "remote failure".to_string(),
            }),
        )
        .await;
        let error = DaemonClient::new(&config)
            .share(CoreShareResource::Note, "note-id")
            .await
            .unwrap_err();
        assert_eq!(error.code(), "daemon_error");
        assert!(error.to_string().contains("remote failure"));
        server.await.unwrap();
    }
}
