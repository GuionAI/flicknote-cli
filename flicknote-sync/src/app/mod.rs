use std::sync::Arc;

use flicknote_core::backend::NoteDb;
use flicknote_core::services::error::ServiceError;
use flicknote_core::services::ports::{NoteCreator, ShareGateway};

use crate::ipc::{AppRequest, AppRequestKind, AppResponse, WireError};

mod note;
mod project;

pub struct Application {
    db: Arc<dyn NoteDb>,
    creator: Arc<dyn NoteCreator>,
    share_gateway: Arc<dyn ShareGateway>,
    web_url: Option<String>,
    write_signal: Option<tokio::sync::mpsc::Sender<()>>,
}

impl Application {
    pub fn new(
        db: Arc<dyn NoteDb>,
        creator: Arc<dyn NoteCreator>,
        share_gateway: Arc<dyn ShareGateway>,
    ) -> Self {
        Self {
            db,
            creator,
            share_gateway,
            web_url: None,
            write_signal: None,
        }
    }

    pub fn with_web_url(mut self, web_url: Option<String>) -> Self {
        self.web_url = web_url;
        self
    }

    pub fn with_write_signal(mut self, signal: tokio::sync::mpsc::Sender<()>) -> Self {
        self.write_signal = Some(signal);
        self
    }

    pub async fn handle(&self, request: AppRequest) -> Result<AppResponse, WireError> {
        let may_write = request.may_write();
        let result = self.handle_inner(request).await;
        if may_write
            && let Some(signal) = &self.write_signal
            && signal.try_send(()).is_err()
        {
            log::debug!(
                "Upload trigger channel full or closed; startup/next write will drain CRUD"
            );
        }
        result
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
        WireError::from_service(ServiceError::from(error))
    }
}
