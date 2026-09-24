use flicknote_core::services::daily::DailyService;

use super::Application;
use crate::ipc::{AppRequest, AppResponse, WireError};

pub(super) async fn handle_write(
    app: &Application,
    request: AppRequest,
) -> Result<AppResponse, WireError> {
    let daily = DailyService::new(app.db.as_ref());
    match request {
        AppRequest::DailyGetOrCreate => daily
            .get_or_create(app.creator.as_ref())
            .await
            .map(AppResponse::Daily)
            .map_err(WireError::from_service),
        AppRequest::Capture { text } => daily
            .capture(app.creator.as_ref(), &text)
            .await
            .map(AppResponse::Capture)
            .map_err(WireError::from_service),
        _ => unreachable!("request kind guarantees a Daily write"),
    }
}
