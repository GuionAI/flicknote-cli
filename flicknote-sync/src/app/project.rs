use flicknote_core::services::error::ServiceError;
use flicknote_core::services::project::ProjectService;

use super::Application;
use crate::ipc::{AppRequest, AppResponse, WireError};

pub(super) async fn handle_read(
    app: &Application,
    request: AppRequest,
) -> Result<AppResponse, WireError> {
    let projects = ProjectService::new(app.db.as_ref());
    match request {
        AppRequest::ProjectList { include_archived } => projects
            .list(include_archived)
            .await
            .map(AppResponse::Projects)
            .map_err(WireError::from_service),
        AppRequest::ProjectGet { id } => projects
            .get(&id)
            .await
            .map(AppResponse::Project)
            .map_err(WireError::from_service),
        AppRequest::ProjectGetByName { name } => project_by_name(app, &name).await,
        _ => unreachable!("request kind guarantees a read-only project request"),
    }
}

pub(super) async fn handle_write(
    app: &Application,
    request: AppRequest,
) -> Result<AppResponse, WireError> {
    let projects = ProjectService::new(app.db.as_ref());
    match request {
        AppRequest::ProjectAdd(input) => projects
            .add(input)
            .await
            .map(AppResponse::Project)
            .map_err(WireError::from_service),
        AppRequest::ProjectModify(input) => projects
            .modify(input)
            .await
            .map(AppResponse::Project)
            .map_err(WireError::from_service),
        AppRequest::ProjectArchive { id } => projects
            .archive(&id)
            .await
            .map(AppResponse::Project)
            .map_err(WireError::from_service),
        AppRequest::ProjectShare { id } => projects
            .share(app.share_gateway.as_ref(), &id)
            .await
            .map(AppResponse::Share)
            .map_err(WireError::from_service),
        AppRequest::ProjectUnshare { id } => projects
            .unshare(app.share_gateway.as_ref(), &id)
            .await
            .map(AppResponse::Unshare)
            .map_err(WireError::from_service),
        _ => unreachable!("request kind guarantees a mutating project request"),
    }
}

async fn project_by_name(app: &Application, name: &str) -> Result<AppResponse, WireError> {
    let id = app
        .db
        .find_project_by_name(name)
        .await
        .map_err(Application::db_error)?
        .ok_or_else(|| WireError::from_service(ServiceError::ProjectNotFound(name.to_string())))?;
    ProjectService::new(app.db.as_ref())
        .get(&id)
        .await
        .map(AppResponse::Project)
        .map_err(WireError::from_service)
}
