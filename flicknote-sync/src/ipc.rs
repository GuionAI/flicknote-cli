use std::fmt;
use std::path::PathBuf;

use async_trait::async_trait;
use flicknote_core::backend::InsertedNote;
use flicknote_core::config::Config;
use flicknote_core::services::dto::{
    InsertPosition, NoteAddInput, NoteArchiveResult, NoteCountInput, NoteDetail, NoteFindInput,
    NoteListInput, NoteModifyInput, NoteMutationResult, NoteSectionResult, NoteSummary, OpenResult,
    ProjectAddInput, ProjectDto, ProjectModifyInput, ShareResult, UnshareResult,
};
use flicknote_core::services::editable_document::EditableSaveResult;
use flicknote_core::services::error::ServiceError;
use flicknote_core::services::ports::{
    CreateNote, NoteCreator, ShareGateway, ShareResource as CoreShareResource,
};
use flicknote_core::services::source::{SourceResult, SourceView};
use flicknote_core::types::{Keyterm, Note, Project};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixListener;
use tokio::net::UnixStream;

use crate::app::Application;

pub const PROTOCOL_VERSION: u16 = 1;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BackendMode {
    Local,
    Managed,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Capability {
    Data,
    NoteAdd,
    Attachment,
    Share,
    LocalSync,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerInfo {
    pub protocol: u16,
    pub backend: BackendMode,
    pub capabilities: Vec<Capability>,
}

impl ServerInfo {
    pub fn local() -> Self {
        Self {
            protocol: PROTOCOL_VERSION,
            backend: BackendMode::Local,
            capabilities: vec![
                Capability::Data,
                Capability::NoteAdd,
                Capability::Attachment,
                Capability::Share,
                Capability::LocalSync,
            ],
        }
    }

    pub fn managed() -> Self {
        Self {
            protocol: PROTOCOL_VERSION,
            backend: BackendMode::Managed,
            capabilities: vec![Capability::Data, Capability::NoteAdd],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "payload", rename_all = "snake_case")]
pub enum AppRequest {
    NoteAdd(NoteAddInput),
    NoteAddEditable {
        document: String,
        project: Option<String>,
    },
    NoteUpload {
        path: String,
        project: Option<String>,
        created_at: Option<String>,
    },
    NoteList(NoteListInput),
    NoteFind(NoteFindInput),
    NoteCount(NoteCountInput),
    NoteGet {
        id: String,
        archived: bool,
    },
    NoteRecord {
        id: String,
        archived: bool,
    },
    NoteGetSection {
        id: String,
        section: String,
    },
    NoteSource {
        id: String,
        archived: bool,
        view: SourceView,
        range: Option<String>,
    },
    NoteAppend {
        id: String,
        content: String,
    },
    NoteSaveEditable {
        id: String,
        document: String,
    },
    NoteReplaceSection {
        id: String,
        section: String,
        content: String,
    },
    NoteRenameSection {
        id: String,
        section: String,
        name: String,
    },
    NoteInsert {
        id: String,
        section: String,
        position: InsertPosition,
        content: String,
    },
    NoteDeleteSection {
        id: String,
        section: String,
    },
    NoteModify(NoteModifyInput),
    NoteArchive {
        id: String,
    },
    NoteRestore {
        id: String,
    },
    NoteShare {
        id: String,
    },
    NoteUnshare {
        id: String,
    },
    NoteOpen {
        id: String,
    },
    ProjectList {
        include_archived: bool,
    },
    ProjectRecords {
        include_archived: bool,
    },
    ProjectGet {
        id: String,
    },
    ProjectAdd(ProjectAddInput),
    ProjectModify(ProjectModifyInput),
    ProjectArchive {
        id: String,
    },
    ProjectShare {
        id: String,
    },
    ProjectUnshare {
        id: String,
    },
    KeytermAdd {
        name: String,
        description: Option<String>,
        content: Option<String>,
    },
    KeytermList,
    KeytermGet {
        id: String,
    },
    KeytermModify {
        id: String,
        name: Option<String>,
        description: Option<String>,
        content: Option<String>,
    },
    KeytermDelete {
        id: String,
    },
    ExtractionValues {
        keys: Vec<String>,
        archived: bool,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "payload", rename_all = "snake_case")]
pub enum AppResponse {
    NoteSummary(NoteSummary),
    NoteSummaries(Vec<NoteSummary>),
    NoteCount { count: u64 },
    NoteDetail(NoteDetail),
    NoteRecord(Note),
    NoteSection(NoteSectionResult),
    NoteMutation(NoteMutationResult),
    EditableSave(EditableSaveResult),
    NoteArchive(NoteArchiveResult),
    Source(SourceResult),
    Share(ShareResult),
    Unshare(UnshareResult),
    Open(OpenResult),
    Projects(Vec<ProjectDto>),
    ProjectRecords(Vec<Project>),
    Project(ProjectDto),
    Keyterms(Vec<Keyterm>),
    Keyterm(Keyterm),
    Id { id: String },
    Values(Vec<String>),
    Unit,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WireError {
    pub code: String,
    pub message: String,
    pub retryable: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
}

impl WireError {
    pub fn from_service(error: ServiceError) -> Self {
        Self {
            code: error.code().to_string(),
            message: error.to_string(),
            retryable: error.retryable(),
            details: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "payload", rename_all = "snake_case")]
pub enum DaemonRequest {
    Health {
        protocol: u16,
    },
    App {
        protocol: u16,
        request: Box<AppRequest>,
    },
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "payload", rename_all = "snake_case")]
pub enum DaemonResponse {
    ServerInfo(ServerInfo),
    App(Box<AppResponse>),
    AppError(WireError),
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
    Other { message: String },
}

impl fmt::Display for DaemonError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unavailable { path, message } => {
                write!(f, "Sync daemon is not available at {path}: {message}")
            }
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

    pub async fn health(&self) -> Result<ServerInfo, ServiceError> {
        match self
            .request(DaemonRequest::Health {
                protocol: PROTOCOL_VERSION,
            })
            .await?
        {
            DaemonResponse::ServerInfo(info) if info.protocol == PROTOCOL_VERSION => Ok(info),
            DaemonResponse::AppError(error) => Err(Self::remote_error(error)),
            _ => Err(Self::protocol_mismatch()),
        }
    }

    pub async fn app(&self, request: AppRequest) -> Result<AppResponse, ServiceError> {
        match self
            .request(DaemonRequest::App {
                protocol: PROTOCOL_VERSION,
                request: Box::new(request),
            })
            .await?
        {
            DaemonResponse::App(response) => Ok(*response),
            DaemonResponse::AppError(error) => Err(Self::remote_error(error)),
            _ => Err(Self::protocol_mismatch()),
        }
    }

    fn remote_error(error: WireError) -> ServiceError {
        ServiceError::Remote {
            code: error.code,
            message: error.message,
            retryable: error.retryable,
            details: error.details,
        }
    }

    fn protocol_mismatch() -> ServiceError {
        ServiceError::Remote {
            code: "daemon_protocol_mismatch".to_string(),
            message: "The running sync daemon uses an incompatible protocol. Restart it with `flicknote sync stop && flicknote sync start`.".to_string(),
            retryable: false,
            details: None,
        }
    }
}

#[async_trait]
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

#[async_trait]
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

pub async fn serve_app_once(
    listener: UnixListener,
    app: std::sync::Arc<Application>,
    info: ServerInfo,
) -> Result<(), DaemonError> {
    let (mut stream, _) = listener
        .accept()
        .await
        .map_err(|error| DaemonError::Other {
            message: format!("Failed to accept daemon request: {error}"),
        })?;
    serve_app_stream(&mut stream, &app, &info).await
}

pub async fn serve_app(
    listener: UnixListener,
    app: std::sync::Arc<Application>,
    info: ServerInfo,
) -> Result<(), DaemonError> {
    loop {
        let (mut stream, _) = listener
            .accept()
            .await
            .map_err(|error| DaemonError::Other {
                message: format!("Failed to accept daemon request: {error}"),
            })?;
        let app = std::sync::Arc::clone(&app);
        let info = info.clone();
        tokio::spawn(async move {
            if let Err(error) = serve_app_stream(&mut stream, &app, &info).await {
                log::warn!("application IPC request failed: {error}");
            }
        });
    }
}

async fn serve_app_stream(
    stream: &mut UnixStream,
    app: &Application,
    info: &ServerInfo,
) -> Result<(), DaemonError> {
    let response = match read_request(stream).await? {
        DaemonRequest::Health { protocol } if protocol == PROTOCOL_VERSION => {
            DaemonResponse::ServerInfo(info.clone())
        }
        DaemonRequest::App { protocol, request } if protocol == PROTOCOL_VERSION => {
            match app.handle(*request).await {
                Ok(response) => DaemonResponse::App(Box::new(response)),
                Err(error) => DaemonResponse::AppError(error),
            }
        }
        DaemonRequest::Health { protocol } | DaemonRequest::App { protocol, .. } => {
            DaemonResponse::AppError(WireError {
                code: "daemon_protocol_mismatch".to_string(),
                message: format!(
                    "daemon protocol {PROTOCOL_VERSION} does not support client protocol {protocol}"
                ),
                retryable: false,
                details: None,
            })
        }
        _ => DaemonResponse::AppError(WireError {
            code: "daemon_protocol_mismatch".to_string(),
            message: "legacy application request is not supported by this daemon".to_string(),
            retryable: false,
            details: None,
        }),
    };
    write_response(stream, &response).await
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
    fn versioned_health_and_app_requests_have_stable_contracts() {
        let health = DaemonRequest::Health {
            protocol: PROTOCOL_VERSION,
        };
        assert_eq!(
            serde_json::to_value(health).unwrap(),
            json!({
                "type": "health",
                "payload": { "protocol": PROTOCOL_VERSION }
            })
        );

        let request = DaemonRequest::App {
            protocol: PROTOCOL_VERSION,
            request: Box::new(AppRequest::NoteList(NoteListInput {
                note_type: None,
                project: None,
                archived: false,
                limit: 20,
            })),
        };
        let value = serde_json::to_value(request).unwrap();
        assert_eq!(value["type"], "app");
        assert_eq!(value["payload"]["protocol"], PROTOCOL_VERSION);
        assert_eq!(value["payload"]["request"]["type"], "note_list");
    }

    #[test]
    fn server_info_reports_backend_mode_and_capabilities() {
        let info = ServerInfo::local();
        assert_eq!(info.protocol, PROTOCOL_VERSION);
        assert_eq!(info.backend, BackendMode::Local);
        assert!(info.capabilities.contains(&Capability::NoteAdd));
        assert!(info.capabilities.contains(&Capability::Share));
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
            attachment_path: None,
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

    #[tokio::test]
    async fn daemon_client_preserves_versioned_app_results_and_errors() {
        let directory = tempfile::tempdir().unwrap();
        let config = test_config(directory.path());
        let server = serve_response(
            &config,
            DaemonResponse::App(Box::new(AppResponse::NoteCount { count: 7 })),
        )
        .await;
        let response = DaemonClient::new(&config)
            .app(AppRequest::NoteCount(NoteCountInput {
                keywords: Vec::new(),
                project: None,
                note_type: None,
                archived: false,
            }))
            .await
            .unwrap();
        assert!(matches!(response, AppResponse::NoteCount { count: 7 }));
        assert!(matches!(server.await.unwrap(), DaemonRequest::App { .. }));

        let directory = tempfile::tempdir().unwrap();
        let config = test_config(directory.path());
        let server = serve_response(
            &config,
            DaemonResponse::AppError(WireError {
                code: "note_not_found".to_string(),
                message: "missing".to_string(),
                retryable: false,
                details: Some(json!({ "id": "42" })),
            }),
        )
        .await;
        let error = DaemonClient::new(&config)
            .app(AppRequest::NoteGet {
                id: "42".to_string(),
                archived: false,
            })
            .await
            .unwrap_err();
        assert_eq!(error.code(), "note_not_found");
        assert_eq!(error.to_string(), "missing");
        server.await.unwrap();
    }

    #[tokio::test]
    async fn health_rejects_legacy_or_unexpected_daemon_responses() {
        let directory = tempfile::tempdir().unwrap();
        let config = test_config(directory.path());
        let server = serve_response(&config, DaemonResponse::ShareRevoked).await;
        let error = DaemonClient::new(&config).health().await.unwrap_err();
        assert_eq!(error.code(), "daemon_protocol_mismatch");
        assert!(error.to_string().contains("sync stop"));
        server.await.unwrap();
    }
}
