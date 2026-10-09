use std::sync::Arc;

use flicknote_core::backend::NoteDb;
use flicknote_core::services::error::ServiceError;
use flicknote_core::services::note::NoteService;
use flicknote_core::services::ports::{NoteCreator, ProjectAssignmentEventSink, ShareGateway};

use crate::search::NoteSearch;
use flicknote_client::{AppRequest, AppRequestKind, AppResponse, WireError};

mod note;
mod project;

pub struct Application {
    db: Arc<dyn NoteDb>,
    creator: Arc<dyn NoteCreator>,
    share_gateway: Option<Arc<dyn ShareGateway>>,
    assignment_events: Option<Arc<dyn ProjectAssignmentEventSink>>,
    web_url: Option<String>,
    search: Option<Arc<dyn NoteSearch>>,
}

impl Application {
    /// Desktop-only lifecycle action; no IPC/MCP or PostgreSQL status setter.
    #[cfg(feature = "experimental-spike")]
    pub async fn retry_local_processing(
        &self,
        local: &flicknote_core::backend::LocalPowerSyncBackend,
        uuid: &str,
        stage: flicknote_core::backend::FailedStage,
    ) -> Result<bool, WireError> {
        local
            .retry_processing(uuid, stage)
            .await
            .map_err(Self::db_error)
    }
    #[cfg(feature = "experimental-spike")]
    pub async fn observe_local_processing(
        &self,
        local: &flicknote_core::backend::LocalPowerSyncBackend,
        uuid: &str,
    ) -> Result<Option<flicknote_core::backend::FailedStage>, WireError> {
        local.observe_processing(uuid).await.map_err(Self::db_error)
    }
    /// Narrow local workspace seam for an immutable watched note/project identity.
    #[cfg(feature = "experimental-spike")]
    pub async fn classify_local_note(
        &self,
        local: &flicknote_core::backend::LocalPowerSyncBackend,
        uuid: &str,
        short_id: i64,
        project: &str,
    ) -> Result<(), WireError> {
        self.notes()
            .classify_local_note(local, uuid, short_id, project)
            .await
            .map_err(Into::into)
    }
    pub fn new(
        db: Arc<dyn NoteDb>,
        creator: Arc<dyn NoteCreator>,
        share_gateway: Arc<dyn ShareGateway>,
    ) -> Self {
        Self {
            db,
            creator,
            share_gateway: Some(share_gateway),
            assignment_events: None,
            web_url: None,
            search: None,
        }
    }

    pub fn new_private(db: Arc<dyn NoteDb>, creator: Arc<dyn NoteCreator>) -> Self {
        Self {
            db,
            creator,
            share_gateway: None,
            assignment_events: None,
            web_url: None,
            search: None,
        }
    }

    pub fn with_assignment_events(
        mut self,
        assignment_events: Arc<dyn ProjectAssignmentEventSink>,
    ) -> Self {
        self.assignment_events = Some(assignment_events);
        self
    }

    pub fn with_web_url(mut self, web_url: Option<String>) -> Self {
        self.web_url = web_url;
        self
    }

    pub fn with_search(mut self, search: impl NoteSearch + 'static) -> Self {
        self.search = Some(Arc::new(search));
        self
    }

    pub async fn handle(&self, request: AppRequest) -> Result<AppResponse, WireError> {
        self.handle_inner(request).await
    }

    #[cfg(feature = "experimental-spike")]
    pub(crate) async fn route_project_locally_guarded(
        &self,
        input: Vec<flicknote_client::dto::NoteRouteProjectInput>,
        local: &flicknote_core::backend::LocalPowerSyncBackend,
        check: impl FnOnce(&rusqlite::Connection) -> Result<bool, flicknote_core::error::CliError>
        + Send,
    ) -> Result<bool, WireError> {
        self.notes()
            .route_project_locally_guarded(input, local, check)
            .await
            .map_err(Into::into)
    }

    async fn handle_inner(&self, request: AppRequest) -> Result<AppResponse, WireError> {
        match request.kind() {
            AppRequestKind::NoteRead => note::handle_read(self, request).await,
            AppRequestKind::NoteWrite => note::handle_write(self, request).await,
            AppRequestKind::ProjectRead => project::handle_read(self, request).await,
            AppRequestKind::ProjectWrite => project::handle_write(self, request).await,
            AppRequestKind::ExtractionRead => self.handle_extraction(request).await,
        }
    }

    async fn handle_extraction(&self, request: AppRequest) -> Result<AppResponse, WireError> {
        let AppRequest::ExtractionValues { keys, archived } = request else {
            unreachable!("request kind guarantees an extraction request")
        };
        let refs = keys.iter().map(String::as_str).collect::<Vec<_>>();
        self.db
            .list_extraction_values(&refs, archived)
            .await
            .map(AppResponse::Values)
            .map_err(Self::db_error)
    }

    fn db_error(error: flicknote_core::error::CliError) -> WireError {
        WireError::from(ServiceError::from(error))
    }

    // GPUI unifies serde_json/preserve_order, enlarging JSON-backed boundary types.
    #[allow(clippy::result_large_err)]
    fn share_gateway(&self) -> Result<&dyn ShareGateway, WireError> {
        self.share_gateway.as_deref().ok_or_else(|| {
            WireError::from(ServiceError::InvalidArgument(
                "sharing is unavailable on this server".into(),
            ))
        })
    }

    fn notes(&self) -> NoteService<'_> {
        let notes = NoteService::new(self.db.as_ref());
        match self.assignment_events.as_deref() {
            Some(events) => notes.with_assignment_events(events),
            None => notes,
        }
    }
}
