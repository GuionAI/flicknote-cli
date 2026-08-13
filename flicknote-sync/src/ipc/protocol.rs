use super::*;

pub const PROTOCOL_VERSION: u16 = 4;
pub const PROTOCOL_MISMATCH_CODE: &str = "daemon_protocol_mismatch";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SyncConnectionState {
    Connected,
    Connecting,
    Offline,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct PowerSyncErrors {
    pub download: Option<String>,
    pub upload: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerInfo {
    pub protocol: u16,
    pub version: String,
    pub executable: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sync: Option<SyncConnectionState>,
    #[serde(default)]
    pub sync_errors: PowerSyncErrors,
}

impl ServerInfo {
    pub fn current() -> Self {
        Self {
            protocol: PROTOCOL_VERSION,
            version: env!("CARGO_PKG_VERSION").to_string(),
            executable: current_executable(),
            sync: None,
            sync_errors: PowerSyncErrors::default(),
        }
    }

    pub fn with_sync_status(
        mut self,
        sync: SyncConnectionState,
        sync_errors: PowerSyncErrors,
    ) -> Self {
        self.sync = Some(sync);
        self.sync_errors = sync_errors;
        self
    }
}

fn current_executable() -> String {
    std::env::current_exe()
        .ok()
        .or_else(|| std::env::args_os().next().map(std::path::PathBuf::from))
        .map_or_else(
            || "unavailable".to_string(),
            |path| path.display().to_string(),
        )
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
    pub(crate) fn kind(&self) -> AppRequestKind {
        match self {
            Self::NoteList(_)
            | Self::NoteFind(_)
            | Self::NoteCount(_)
            | Self::NoteGet { .. }
            | Self::NoteLoadEditable { .. }
            | Self::NoteRecord { .. }
            | Self::NoteGetSection { .. }
            | Self::NoteSource { .. }
            | Self::NoteOpen { .. } => AppRequestKind::NoteRead,
            Self::NoteAdd(_)
            | Self::NoteAddEditable { .. }
            | Self::NoteUpload { .. }
            | Self::NoteAppend { .. }
            | Self::NoteSaveEditable { .. }
            | Self::NoteReplaceSection { .. }
            | Self::NoteRenameSection { .. }
            | Self::NoteInsert { .. }
            | Self::NoteDeleteSection { .. }
            | Self::NoteModify(_)
            | Self::NoteArchive { .. }
            | Self::NoteRestore { .. }
            | Self::NoteShare { .. }
            | Self::NoteUnshare { .. } => AppRequestKind::NoteWrite,
            Self::ProjectList { .. }
            | Self::ProjectRecords { .. }
            | Self::ProjectGet { .. }
            | Self::ProjectGetByName { .. } => AppRequestKind::ProjectRead,
            Self::ProjectAdd(_)
            | Self::ProjectModify(_)
            | Self::ProjectArchive { .. }
            | Self::ProjectShare { .. }
            | Self::ProjectUnshare { .. } => AppRequestKind::ProjectWrite,
            Self::ExtractionValues { .. } => AppRequestKind::ExtractionRead,
        }
    }

    pub fn may_write(&self) -> bool {
        matches!(
            self.kind(),
            AppRequestKind::NoteWrite | AppRequestKind::ProjectWrite
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AppRequestKind {
    NoteRead,
    NoteWrite,
    ProjectRead,
    ProjectWrite,
    ExtractionRead,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "payload", rename_all = "snake_case")]
pub enum AppResponse {
    NoteSummary(NoteSummary),
    NoteSummaries(Vec<NoteSummary>),
    NoteCount { count: u64 },
    NoteDetail(NoteDetail),
    EditableDocument(EditableDocument),
    NoteRecord(NoteRecord),
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
    Values(Vec<String>),
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
app_result!(NoteRecord, AppResponse::NoteRecord);
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

#[derive(Debug, Clone, PartialEq, Eq)]
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
                write!(f, "FlickNote daemon is not available at {path}: {message}")
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
