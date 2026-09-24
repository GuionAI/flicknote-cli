use flicknote_core::services::comment::CommentService;

use super::Application;
use crate::ipc::{AppRequest, AppResponse, WireError};

pub(super) async fn handle_read(
    app: &Application,
    request: AppRequest,
) -> Result<AppResponse, WireError> {
    let comments = CommentService::new(app.db.as_ref());
    match request {
        AppRequest::CommentList { note_id } => comments
            .list(&note_id)
            .await
            .map(AppResponse::Comments)
            .map_err(WireError::from_service),
        AppRequest::CommentPending(input) => comments
            .pending_routing(input.limit)
            .await
            .map(AppResponse::Comments)
            .map_err(WireError::from_service),
        _ => unreachable!("request kind guarantees a comment read"),
    }
}

pub(super) async fn handle_write(
    app: &Application,
    request: AppRequest,
) -> Result<AppResponse, WireError> {
    let comments = CommentService::new(app.db.as_ref());
    match request {
        AppRequest::CommentCreate(input) => comments
            .create(input)
            .await
            .map(AppResponse::Comment)
            .map_err(WireError::from_service),
        AppRequest::CommentModify(input) => comments
            .modify(input)
            .await
            .map(AppResponse::Comment)
            .map_err(WireError::from_service),
        _ => unreachable!("request kind guarantees a comment write"),
    }
}
