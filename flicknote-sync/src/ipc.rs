use std::fmt;
use std::path::PathBuf;

use flicknote_core::config::Config;
use flicknote_core::services::dto::{
    InsertPosition, NoteAddInput, NoteArchiveResult, NoteCountInput, NoteDetail, NoteFindInput,
    NoteListInput, NoteModifyInput, NoteMutationResult, NoteSectionResult, NoteSummary, OpenResult,
    ProjectAddInput, ProjectDto, ProjectModifyInput, ShareResult, UnshareResult,
};
use flicknote_core::services::editable_document::EditableSaveResult;
use flicknote_core::services::error::ServiceError;
use flicknote_core::services::source::{SourceResult, SourceView};
use flicknote_core::types::{Note, Project};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixListener;
use tokio::net::UnixStream;

use crate::app::Application;

pub const PROTOCOL_VERSION: u16 = 1;
const IPC_CONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(1);
const IPC_WRITE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(1);
const IPC_HEALTH_RESPONSE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(1);
const IPC_APP_RESPONSE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(300);

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BackendMode {
    Local,
    Managed,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ClientSurface {
    #[default]
    Cli,
    Mcp,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Capability {
    Data,
    NoteAdd,
    Attachment,
    Editor,
    Browser,
    Mcp,
    Share,
    LocalSync,
}

const LOCAL_CAPABILITIES: &[Capability] = &[
    Capability::Data,
    Capability::NoteAdd,
    Capability::Attachment,
    Capability::Editor,
    Capability::Browser,
    Capability::Mcp,
    Capability::Share,
    Capability::LocalSync,
];
const MANAGED_CAPABILITIES: &[Capability] = &[Capability::Data, Capability::NoteAdd];

impl BackendMode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Managed => "managed",
        }
    }

    pub const fn capabilities(self) -> &'static [Capability] {
        match self {
            Self::Local => LOCAL_CAPABILITIES,
            Self::Managed => MANAGED_CAPABILITIES,
        }
    }

    pub fn supports(self, capability: Capability) -> bool {
        self.capabilities().contains(&capability)
    }
}

pub fn unsupported_capability(
    mode: BackendMode,
    capability: Capability,
    operation: &str,
) -> ServiceError {
    ServiceError::Remote {
        code: "unsupported_capability".to_string(),
        message: format!(
            "{operation} is not available in {} daemon mode",
            mode.as_str()
        ),
        retryable: false,
        details: Some(serde_json::json!({
            "operation": operation,
            "backend": mode,
            "required_capability": capability,
        })),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerInfo {
    pub protocol: u16,
    pub version: String,
    pub backend: BackendMode,
    pub capabilities: Vec<Capability>,
}

impl ServerInfo {
    pub fn local() -> Self {
        Self {
            protocol: PROTOCOL_VERSION,
            version: env!("CARGO_PKG_VERSION").to_string(),
            backend: BackendMode::Local,
            capabilities: BackendMode::Local.capabilities().to_vec(),
        }
    }

    pub fn managed() -> Self {
        Self {
            protocol: PROTOCOL_VERSION,
            version: env!("CARGO_PKG_VERSION").to_string(),
            backend: BackendMode::Managed,
            capabilities: BackendMode::Managed.capabilities().to_vec(),
        }
    }

    pub fn require(&self, capability: Capability, operation: &str) -> Result<(), ServiceError> {
        if self.capabilities.contains(&capability) {
            return Ok(());
        }
        Err(unsupported_capability(self.backend, capability, operation))
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
    NoteLoadEditable {
        id: String,
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
    ProjectGetByName {
        name: String,
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
    ExtractionValues {
        keys: Vec<String>,
        archived: bool,
    },
}

impl AppRequest {
    pub fn may_write(&self) -> bool {
        !matches!(
            self,
            Self::NoteList(_)
                | Self::NoteFind(_)
                | Self::NoteCount(_)
                | Self::NoteGet { .. }
                | Self::NoteLoadEditable { .. }
                | Self::NoteRecord { .. }
                | Self::NoteGetSection { .. }
                | Self::NoteSource { .. }
                | Self::NoteOpen { .. }
                | Self::ProjectList { .. }
                | Self::ProjectRecords { .. }
                | Self::ProjectGet { .. }
                | Self::ProjectGetByName { .. }
                | Self::ExtractionValues { .. }
        )
    }

    pub fn required_capability(&self) -> Capability {
        match self {
            Self::NoteAdd(_) => Capability::NoteAdd,
            Self::NoteAddEditable { .. }
            | Self::NoteLoadEditable { .. }
            | Self::NoteSaveEditable { .. } => Capability::Editor,
            Self::NoteUpload { .. } => Capability::Attachment,
            Self::NoteOpen { .. } => Capability::Browser,
            Self::NoteShare { .. }
            | Self::NoteUnshare { .. }
            | Self::ProjectShare { .. }
            | Self::ProjectUnshare { .. } => Capability::Share,
            _ => Capability::Data,
        }
    }

    pub fn operation_name(&self) -> &'static str {
        match self {
            Self::NoteAdd(_) => "note_add",
            Self::NoteAddEditable { .. } => "note_add_editable",
            Self::NoteUpload { .. } => "note_upload",
            Self::NoteLoadEditable { .. } => "note_load_editable",
            Self::NoteSaveEditable { .. } => "note_save_editable",
            Self::NoteOpen { .. } => "note_open",
            Self::NoteShare { .. } => "note_share",
            Self::NoteUnshare { .. } => "note_unshare",
            Self::ProjectShare { .. } => "project_share",
            Self::ProjectUnshare { .. } => "project_unshare",
            _ => "data",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "payload", rename_all = "snake_case")]
pub enum AppResponse {
    NoteSummary(NoteSummary),
    NoteSummaries(Vec<NoteSummary>),
    NoteCount { count: u64 },
    NoteDetail(NoteDetail),
    EditableDocument(EditableDocument),
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
    Id { id: String },
    Values(Vec<String>),
    Unit,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EditableDocument {
    pub document: String,
}

pub trait AppResult: Sized {
    fn from_response(response: AppResponse) -> Option<Self>;
}

macro_rules! app_result {
    ($type:ty, $variant:path) => {
        impl AppResult for $type {
            fn from_response(response: AppResponse) -> Option<Self> {
                match response {
                    $variant(value) => Some(value),
                    _ => None,
                }
            }
        }
    };
}

app_result!(NoteSummary, AppResponse::NoteSummary);
app_result!(Vec<NoteSummary>, AppResponse::NoteSummaries);
app_result!(NoteDetail, AppResponse::NoteDetail);
app_result!(EditableDocument, AppResponse::EditableDocument);
app_result!(Note, AppResponse::NoteRecord);
app_result!(NoteSectionResult, AppResponse::NoteSection);
app_result!(NoteMutationResult, AppResponse::NoteMutation);
app_result!(EditableSaveResult, AppResponse::EditableSave);
app_result!(NoteArchiveResult, AppResponse::NoteArchive);
app_result!(SourceResult, AppResponse::Source);
app_result!(ShareResult, AppResponse::Share);
app_result!(UnshareResult, AppResponse::Unshare);
app_result!(OpenResult, AppResponse::Open);
app_result!(Vec<ProjectDto>, AppResponse::Projects);
app_result!(Vec<Project>, AppResponse::ProjectRecords);
app_result!(ProjectDto, AppResponse::Project);
app_result!(Vec<String>, AppResponse::Values);

impl AppResult for u64 {
    fn from_response(response: AppResponse) -> Option<Self> {
        match response {
            AppResponse::NoteCount { count } => Some(count),
            _ => None,
        }
    }
}

impl AppResult for String {
    fn from_response(response: AppResponse) -> Option<Self> {
        match response {
            AppResponse::Id { id } => Some(id),
            _ => None,
        }
    }
}

impl AppResult for () {
    fn from_response(response: AppResponse) -> Option<Self> {
        match response {
            AppResponse::Unit => Some(()),
            _ => None,
        }
    }
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
        match error {
            ServiceError::Remote {
                code,
                message,
                retryable,
                details,
            } => Self {
                code,
                message,
                retryable,
                details,
            },
            error => Self {
                code: error.code().to_string(),
                message: error.to_string(),
                retryable: error.retryable(),
                details: None,
            },
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
        #[serde(default)]
        surface: ClientSurface,
        request: Box<AppRequest>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "payload", rename_all = "snake_case")]
pub enum DaemonResponse {
    ServerInfo(ServerInfo),
    App(Box<AppResponse>),
    AppError(WireError),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "code", rename_all = "snake_case")]
pub enum DaemonError {
    Unavailable {
        path: String,
        message: String,
    },
    PartialCreate {
        message: String,
        note_id: String,
        short_id: Option<i64>,
        confirmed_extraction_ids: Vec<String>,
        pending_extraction_ids: Vec<String>,
    },
    AmbiguousCreate {
        message: String,
        note_id: String,
        pending_extraction_ids: Vec<String>,
    },
    InvalidResponse {
        message: String,
    },
    IncompleteResponse {
        message: String,
    },
    MalformedResponse {
        message: String,
    },
    PostConnectTransport {
        message: String,
    },
    Other {
        message: String,
    },
}

impl fmt::Display for DaemonError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unavailable { path, message } => {
                write!(f, "Sync daemon is not available at {path}: {message}")
            }
            Self::PartialCreate { message, .. }
            | Self::AmbiguousCreate { message, .. }
            | Self::InvalidResponse { message }
            | Self::IncompleteResponse { message }
            | Self::MalformedResponse { message }
            | Self::PostConnectTransport { message }
            | Self::Other { message } => f.write_str(message),
        }
    }
}

impl std::error::Error for DaemonError {}

pub fn socket_path(config: &Config) -> PathBuf {
    config.paths.data_dir.join("sync.sock")
}

fn unavailable(path: &std::path::Path, stage: &str) -> DaemonError {
    DaemonError::Unavailable {
        path: path.display().to_string(),
        message: format!("timed out while {stage}"),
    }
}

fn request_timeout_error(
    request: &DaemonRequest,
    path: &std::path::Path,
    stage: &str,
) -> DaemonError {
    if !is_mutating_app_request(request) {
        return unavailable(path, stage);
    }
    DaemonError::Other {
        message: format!(
            "Timed out while {stage} from the sync daemon at {}; the application request outcome is unknown. Do not retry it automatically.",
            path.display()
        ),
    }
}

fn is_mutating_app_request(request: &DaemonRequest) -> bool {
    matches!(request, DaemonRequest::App { request, .. } if request.may_write())
}

fn response_timeout_for(request: &DaemonRequest) -> Option<std::time::Duration> {
    match request {
        DaemonRequest::Health { .. } => Some(IPC_HEALTH_RESPONSE_TIMEOUT),
        // Once a write request may have reached the daemon, a transport timeout cannot tell
        // whether it committed. Keep waiting for the authoritative response until the protocol
        // has durable operation IDs and status reconciliation (tracked as FlickNote #1785).
        DaemonRequest::App { request, .. } if request.may_write() => None,
        DaemonRequest::App { .. } => Some(IPC_APP_RESPONSE_TIMEOUT),
    }
}

pub async fn send_request(
    config: &Config,
    request: &DaemonRequest,
) -> Result<DaemonResponse, DaemonError> {
    let path = socket_path(config);
    let request_bytes = serde_json::to_vec(request).map_err(|e| DaemonError::Other {
        message: format!("Failed to serialize daemon request: {e}"),
    })?;
    let mut stream = tokio::time::timeout(IPC_CONNECT_TIMEOUT, UnixStream::connect(&path))
        .await
        .map_err(|_| unavailable(&path, "connecting"))?
        .map_err(|error| DaemonError::Unavailable {
            path: path.display().to_string(),
            message: error.to_string(),
        })?;
    let write_request = async {
        stream.write_all(&request_bytes).await?;
        stream.shutdown().await
    };
    if is_mutating_app_request(request) {
        write_request
            .await
            .map_err(|error| DaemonError::PostConnectTransport {
                message: format!("Failed to send daemon request: {error}"),
            })?;
    } else {
        tokio::time::timeout(IPC_WRITE_TIMEOUT, write_request)
            .await
            .map_err(|_| request_timeout_error(request, &path, "sending a request"))?
            .map_err(|error| DaemonError::PostConnectTransport {
                message: format!("Failed to send daemon request: {error}"),
            })?;
    }
    let mut buf = Vec::new();
    match response_timeout_for(request) {
        Some(response_timeout) => {
            tokio::time::timeout(response_timeout, stream.read_to_end(&mut buf))
                .await
                .map_err(|_| request_timeout_error(request, &path, "waiting for a response"))?
        }
        None => stream.read_to_end(&mut buf).await,
    }
    .map_err(|e| DaemonError::PostConnectTransport {
        message: format!("Failed to read daemon response: {e}"),
    })?;
    serde_json::from_slice(&buf).map_err(|e| {
        if e.is_eof() {
            return DaemonError::IncompleteResponse {
                message: format!("Daemon closed the connection before a complete response: {e}"),
            };
        }
        match serde_json::from_slice::<serde_json::Value>(&buf) {
            Ok(_) => DaemonError::InvalidResponse {
                message: format!("Daemon returned an incompatible response: {e}"),
            },
            Err(raw_error) => DaemonError::MalformedResponse {
                message: format!("Daemon returned a malformed response: {raw_error}"),
            },
        }
    })
}

pub struct DaemonClient<'a> {
    config: &'a Config,
    surface: ClientSurface,
}

impl<'a> DaemonClient<'a> {
    pub fn new(config: &'a Config) -> Self {
        Self {
            config,
            surface: ClientSurface::Cli,
        }
    }

    pub fn for_mcp(config: &'a Config) -> Self {
        Self {
            config,
            surface: ClientSurface::Mcp,
        }
    }

    async fn request(&self, request: DaemonRequest) -> Result<DaemonResponse, ServiceError> {
        let is_mutating = is_mutating_app_request(&request);
        send_request(self.config, &request)
            .await
            .map_err(|error| match error {
                DaemonError::Unavailable { .. } => ServiceError::DaemonUnavailable(format!(
                    "{error}. Start it with `flicknote sync start`."
                )),
                DaemonError::IncompleteResponse { .. }
                | DaemonError::MalformedResponse { .. }
                | DaemonError::PostConnectTransport { .. }
                    if !is_mutating =>
                {
                    ServiceError::DaemonUnavailable(format!(
                        "Sync daemon is not ready: {error}. Start it with `flicknote sync start`."
                    ))
                }
                DaemonError::IncompleteResponse { message }
                | DaemonError::MalformedResponse { message }
                | DaemonError::PostConnectTransport { message }
                | DaemonError::InvalidResponse { message }
                    if is_mutating =>
                {
                    Self::outcome_unknown(message)
                }
                DaemonError::InvalidResponse { .. } => Self::protocol_mismatch(),
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
        let may_write = request.may_write();
        match self
            .request(DaemonRequest::App {
                protocol: PROTOCOL_VERSION,
                surface: self.surface,
                request: Box::new(request),
            })
            .await?
        {
            DaemonResponse::App(response) => Ok(*response),
            DaemonResponse::AppError(error) => Err(Self::remote_error(error)),
            _ if may_write => Err(Self::outcome_unknown(
                "The daemon returned an unexpected envelope after a mutating request; the operation outcome is unknown."
                    .to_string(),
            )),
            _ => Err(Self::protocol_mismatch()),
        }
    }

    pub async fn call<T: AppResult>(&self, request: AppRequest) -> Result<T, ServiceError> {
        let may_write = request.may_write();
        let response = self.app(request).await?;
        T::from_response(response).ok_or_else(|| {
            if may_write {
                Self::outcome_unknown(
                    "The daemon returned an unexpected response after a mutating request; the operation outcome is unknown.".to_string(),
                )
            } else {
                Self::protocol_mismatch()
            }
        })
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

    fn outcome_unknown(message: String) -> ServiceError {
        ServiceError::Remote {
            code: "daemon_request_outcome_unknown".to_string(),
            message,
            retryable: false,
            details: None,
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
        DaemonRequest::App {
            protocol,
            surface,
            request,
        } if protocol == PROTOCOL_VERSION => {
            if surface == ClientSurface::Mcp && !info.backend.supports(Capability::Mcp) {
                DaemonResponse::AppError(WireError::from_service(unsupported_capability(
                    info.backend,
                    Capability::Mcp,
                    "mcp",
                )))
            } else {
                match app.handle(*request).await {
                    Ok(response) => DaemonResponse::App(Box::new(response)),
                    Err(error) => DaemonResponse::AppError(error),
                }
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
            surface: ClientSurface::Cli,
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
        assert_eq!(value["payload"]["surface"], "cli");
        assert_eq!(value["payload"]["request"]["type"], "note_list");
    }

    #[test]
    fn server_info_reports_backend_mode_and_capabilities() {
        let info = ServerInfo::local();
        assert_eq!(info.protocol, PROTOCOL_VERSION);
        assert!(!info.version.is_empty());
        assert_eq!(info.backend, BackendMode::Local);
        assert!(info.capabilities.contains(&Capability::NoteAdd));
        assert!(info.capabilities.contains(&Capability::Share));
        assert!(
            serde_json::to_value(&info).unwrap()["capabilities"]
                .as_array()
                .unwrap()
                .contains(&json!("mcp"))
        );

        let error = ServerInfo::managed()
            .require(Capability::Mcp, "mcp")
            .unwrap_err();
        assert_eq!(error.code(), "unsupported_capability");
    }

    #[test]
    fn wire_error_preserves_partial_success_details() {
        let details = json!({"created": true, "short_id": 80});
        let wire = WireError::from_service(ServiceError::Remote {
            code: "note_create_partial".to_string(),
            message: "note created; topics pending".to_string(),
            retryable: false,
            details: Some(details.clone()),
        });

        assert_eq!(wire.code, "note_create_partial");
        assert_eq!(wire.details, Some(details));
    }

    #[tokio::test]
    async fn daemon_client_maps_missing_socket_to_retryable_unavailable() {
        let directory = tempfile::tempdir().unwrap();
        let config = test_config(directory.path());

        let error = DaemonClient::new(&config).health().await.unwrap_err();

        assert_eq!(error.code(), "daemon_unavailable");
        assert!(error.retryable());
        assert!(error.to_string().contains("flicknote sync start"));
    }

    #[tokio::test]
    async fn health_request_has_a_bounded_response_wait() {
        let directory = tempfile::tempdir().unwrap();
        let config = test_config(directory.path());
        let listener = UnixListener::bind(socket_path(&config)).unwrap();
        let server = tokio::spawn(async move {
            let (_stream, _) = listener.accept().await.unwrap();
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        });

        let result = tokio::time::timeout(
            std::time::Duration::from_millis(1_200),
            send_request(
                &config,
                &DaemonRequest::Health {
                    protocol: PROTOCOL_VERSION,
                },
            ),
        )
        .await;
        server.abort();

        let response = result.expect("IPC must enforce its own response timeout");
        assert!(matches!(response, Err(DaemonError::Unavailable { .. })));
    }

    #[test]
    fn mutating_application_requests_do_not_have_an_automatic_response_timeout() {
        let request = DaemonRequest::App {
            protocol: PROTOCOL_VERSION,
            surface: ClientSurface::Cli,
            request: Box::new(AppRequest::NoteArchive {
                id: "note-1".to_string(),
            }),
        };

        assert_eq!(response_timeout_for(&request), None);
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
    async fn health_rejects_unexpected_daemon_responses() {
        let directory = tempfile::tempdir().unwrap();
        let config = test_config(directory.path());
        let server = serve_response(
            &config,
            DaemonResponse::App(Box::new(AppResponse::NoteCount { count: 0 })),
        )
        .await;
        let error = DaemonClient::new(&config).health().await.unwrap_err();
        assert_eq!(error.code(), "daemon_protocol_mismatch");
        assert!(error.to_string().contains("sync stop"));
        server.await.unwrap();
    }

    #[tokio::test]
    async fn application_maps_unknown_envelope_to_protocol_mismatch() {
        let directory = tempfile::tempdir().unwrap();
        let config = test_config(directory.path());
        let listener = UnixListener::bind(socket_path(&config)).unwrap();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let _request = read_request(&mut stream).await.unwrap();
            write_json(&mut stream, &json!({"type":"legacy_result","payload":{}}))
                .await
                .unwrap();
        });

        let error = DaemonClient::new(&config)
            .app(AppRequest::NoteCount(NoteCountInput {
                keywords: Vec::new(),
                project: None,
                note_type: None,
                archived: false,
            }))
            .await
            .unwrap_err();

        assert_eq!(error.code(), "daemon_protocol_mismatch");
        assert!(!error.retryable());
        server.await.unwrap();
    }

    #[tokio::test]
    async fn mutating_application_maps_incomplete_response_to_unknown_outcome() {
        let directory = tempfile::tempdir().unwrap();
        let config = test_config(directory.path());
        let listener = UnixListener::bind(socket_path(&config)).unwrap();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let _request = read_request(&mut stream).await.unwrap();
            stream.write_all(br#"{"type":"app""#).await.unwrap();
            stream.shutdown().await.unwrap();
        });

        let error = DaemonClient::new(&config)
            .app(AppRequest::NoteArchive {
                id: "note-1".to_string(),
            })
            .await
            .unwrap_err();

        assert_eq!(error.code(), "daemon_request_outcome_unknown");
        assert!(!error.retryable());
        server.await.unwrap();
    }

    #[tokio::test]
    async fn malformed_transport_responses_are_classified_by_mutation_safety() {
        for (request, expected_code, retryable) in [
            (
                AppRequest::NoteArchive {
                    id: "note-1".to_string(),
                },
                "daemon_request_outcome_unknown",
                false,
            ),
            (
                AppRequest::NoteCount(NoteCountInput {
                    keywords: Vec::new(),
                    project: None,
                    note_type: None,
                    archived: false,
                }),
                "daemon_unavailable",
                true,
            ),
        ] {
            let directory = tempfile::tempdir().unwrap();
            let config = test_config(directory.path());
            let listener = UnixListener::bind(socket_path(&config)).unwrap();
            let server = tokio::spawn(async move {
                let (mut stream, _) = listener.accept().await.unwrap();
                let _request = read_request(&mut stream).await.unwrap();
                stream.write_all(b"not-json").await.unwrap();
                stream.shutdown().await.unwrap();
            });

            let error = DaemonClient::new(&config).app(request).await.unwrap_err();

            assert_eq!(error.code(), expected_code);
            assert_eq!(error.retryable(), retryable);
            server.await.unwrap();
        }
    }

    #[tokio::test]
    async fn unexpected_typed_responses_are_classified_by_mutation_safety() {
        let directory = tempfile::tempdir().unwrap();
        let config = test_config(directory.path());
        let server =
            serve_response(&config, DaemonResponse::App(Box::new(AppResponse::Unit))).await;
        let error = DaemonClient::new(&config)
            .call::<u64>(AppRequest::NoteCount(NoteCountInput {
                keywords: Vec::new(),
                project: None,
                note_type: None,
                archived: false,
            }))
            .await
            .unwrap_err();
        assert_eq!(error.code(), "daemon_protocol_mismatch");
        server.await.unwrap();

        let directory = tempfile::tempdir().unwrap();
        let config = test_config(directory.path());
        let server =
            serve_response(&config, DaemonResponse::App(Box::new(AppResponse::Unit))).await;
        let error = DaemonClient::new(&config)
            .call::<NoteArchiveResult>(AppRequest::NoteArchive {
                id: "note-1".to_string(),
            })
            .await
            .unwrap_err();
        assert_eq!(error.code(), "daemon_request_outcome_unknown");
        server.await.unwrap();
    }

    #[tokio::test]
    async fn unexpected_outer_responses_are_classified_by_mutation_safety() {
        let directory = tempfile::tempdir().unwrap();
        let config = test_config(directory.path());
        let server = serve_response(&config, DaemonResponse::ServerInfo(ServerInfo::local())).await;

        let error = DaemonClient::new(&config)
            .app(AppRequest::NoteArchive {
                id: "note-1".to_string(),
            })
            .await
            .unwrap_err();

        assert_eq!(error.code(), "daemon_request_outcome_unknown");
        assert!(!error.retryable());
        server.await.unwrap();
    }

    #[tokio::test]
    async fn health_maps_legacy_daemon_error_to_protocol_mismatch() {
        let directory = tempfile::tempdir().unwrap();
        let config = test_config(directory.path());
        let listener = UnixListener::bind(socket_path(&config)).unwrap();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let request = read_request(&mut stream).await.unwrap();
            let response = json!({
                "type": "error",
                "payload": {
                    "code": "other",
                    "message": "Failed to parse daemon request: unknown variant `health`"
                }
            });
            write_json(&mut stream, &response).await.unwrap();
            request
        });

        let error = DaemonClient::new(&config).health().await.unwrap_err();

        assert_eq!(error.code(), "daemon_protocol_mismatch");
        assert!(!error.retryable());
        assert!(error.to_string().contains("sync stop"));
        assert!(matches!(
            server.await.unwrap(),
            DaemonRequest::Health { .. }
        ));
    }

    #[tokio::test]
    async fn health_maps_empty_startup_response_to_retryable_unavailable() {
        let directory = tempfile::tempdir().unwrap();
        let config = test_config(directory.path());
        let listener = UnixListener::bind(socket_path(&config)).unwrap();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let _request = read_request(&mut stream).await.unwrap();
            drop(stream);
        });

        let error = DaemonClient::new(&config).health().await.unwrap_err();

        assert_eq!(error.code(), "daemon_unavailable");
        assert!(error.retryable());
        server.await.unwrap();
    }
}
