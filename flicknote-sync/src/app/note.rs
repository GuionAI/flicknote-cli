use std::path::PathBuf;

use flicknote_core::services::dto::NoteAddInput;
use flicknote_core::services::editable_document;
use flicknote_core::services::error::ServiceError;
use flicknote_core::services::note::{NoteService, confirmed_create_followup_error};
use flicknote_core::services::ports::{CreateNote, CreatedNote};
use flicknote_core::services::upload::{self, UploadKind};

use super::Application;
use crate::ipc::{AppRequest, AppResponse, EditableDocument, WireError};

pub(super) async fn handle_read(
    app: &Application,
    request: AppRequest,
) -> Result<AppResponse, WireError> {
    let notes = NoteService::new(app.db.as_ref());
    match request {
        AppRequest::NoteList(input) => {
            service_result(notes.list(input).await, AppResponse::NoteSummaries)
        }
        AppRequest::NoteFind(input) => {
            service_result(notes.find(input).await, AppResponse::NoteSummaries)
        }
        AppRequest::NoteCount(input) => notes
            .count(input)
            .await
            .map(|count| AppResponse::NoteCount { count })
            .map_err(WireError::from_service),
        AppRequest::NoteGet { id, archived } => {
            service_result(notes.get(&id, archived).await, AppResponse::NoteDetail)
        }
        AppRequest::NoteLoadEditable { id } => load_editable(app, &id).await,
        AppRequest::NoteRecord { id, archived } => note_record(app, &id, archived).await,
        AppRequest::NoteGetSection { id, section } => service_result(
            notes.get_section(&id, &section).await,
            AppResponse::NoteSection,
        ),
        AppRequest::NoteSource {
            id,
            archived,
            view,
            range,
        } => service_result(
            notes.source(&id, archived, view, range.as_deref()).await,
            AppResponse::Source,
        ),
        AppRequest::NoteOpen { id } => open_note(app, &id).await,
        _ => unreachable!("request kind guarantees a read-only note request"),
    }
}

pub(super) async fn handle_write(
    app: &Application,
    request: AppRequest,
) -> Result<AppResponse, WireError> {
    let notes = NoteService::new(app.db.as_ref());
    match request {
        AppRequest::NoteAdd(input) => service_result(
            notes.add(app.creator.as_ref(), input).await,
            AppResponse::NoteSummary,
        ),
        AppRequest::NoteAddEditable { document, project } => {
            add_editable(app, &document, project.as_deref()).await
        }
        AppRequest::NoteUpload {
            path,
            project,
            created_at,
        } => upload_note(app, PathBuf::from(path), project, created_at).await,
        AppRequest::NoteAppend { id, content } => {
            service_result(notes.append(&id, &content).await, AppResponse::NoteMutation)
        }
        AppRequest::NoteSaveEditable { id, document } => save_editable(app, &id, &document).await,
        AppRequest::NoteReplaceSection {
            id,
            section,
            content,
        } => service_result(
            notes.replace_section(&id, &section, &content).await,
            AppResponse::NoteMutation,
        ),
        AppRequest::NoteRenameSection { id, section, name } => service_result(
            notes.rename_section(&id, &section, &name).await,
            AppResponse::NoteMutation,
        ),
        AppRequest::NoteInsert {
            id,
            section,
            position,
            content,
        } => service_result(
            notes.insert(&id, &section, position, &content).await,
            AppResponse::NoteMutation,
        ),
        AppRequest::NoteDeleteSection { id, section } => service_result(
            notes.delete_section(&id, &section).await,
            AppResponse::NoteMutation,
        ),
        AppRequest::NoteModify(input) => {
            service_result(notes.modify(input).await, AppResponse::NoteMutation)
        }
        AppRequest::NoteArchive { id } => {
            service_result(notes.archive(&id).await, AppResponse::NoteArchive)
        }
        AppRequest::NoteRestore { id } => {
            service_result(notes.restore(&id).await, AppResponse::NoteArchive)
        }
        AppRequest::NoteShare { id } => service_result(
            notes.share(app.share_gateway.as_ref(), &id).await,
            AppResponse::Share,
        ),
        AppRequest::NoteUnshare { id } => service_result(
            notes.unshare(app.share_gateway.as_ref(), &id).await,
            AppResponse::Unshare,
        ),
        _ => unreachable!("request kind guarantees a mutating note request"),
    }
}

fn service_result<T>(
    result: Result<T, ServiceError>,
    response: impl FnOnce(T) -> AppResponse,
) -> Result<AppResponse, WireError> {
    result.map(response).map_err(WireError::from_service)
}

async fn load_editable(app: &Application, id: &str) -> Result<AppResponse, WireError> {
    let id = app
        .db
        .resolve_note_id(id)
        .await
        .map_err(Application::db_error)?;
    editable_document::load_editable_note(app.db.as_ref(), &id)
        .await
        .map(|document| AppResponse::EditableDocument(EditableDocument { document }))
        .map_err(Application::db_error)
}

async fn note_record(
    app: &Application,
    id: &str,
    archived: bool,
) -> Result<AppResponse, WireError> {
    let id = if archived {
        app.db.resolve_archived_note_id(id).await
    } else {
        app.db.resolve_note_id(id).await
    }
    .map_err(Application::db_error)?;
    let note = if archived {
        app.db.find_archived_note(&id).await
    } else {
        app.db.find_note(&id).await
    }
    .map_err(Application::db_error)?;
    Ok(AppResponse::NoteRecord(note.into()))
}

async fn open_note(app: &Application, id: &str) -> Result<AppResponse, WireError> {
    let web_url = app.web_url.as_deref().ok_or_else(|| {
        WireError::from_service(ServiceError::ConfigMissing("webUrl".to_string()))
    })?;
    let full_id = app
        .db
        .resolve_note_id(id)
        .await
        .map_err(Application::db_error)?;
    let note = app
        .db
        .find_note(&full_id)
        .await
        .map_err(Application::db_error)?;
    let url_id = note.short_id.map_or(full_id, |value| value.to_string());
    Ok(AppResponse::Open(
        flicknote_core::services::dto::OpenResult {
            url: format!("{}/notes/{url_id}", web_url.trim_end_matches('/')),
            opened: false,
        },
    ))
}

async fn add_editable(
    app: &Application,
    document: &str,
    project: Option<&str>,
) -> Result<AppResponse, WireError> {
    let parsed = editable_document::parse_editable_note(document).map_err(Application::db_error)?;
    let created = app
        .creator
        .create(CreateNote {
            id: uuid::Uuid::new_v4().to_string(),
            note_type: "normal".to_string(),
            status: "ai_queued".to_string(),
            title: Some(parsed.title),
            content: Some(parsed.stored_content),
            metadata: None,
            project_id: resolve_project_id(app, project).await?,
            now: chrono::Utc::now().to_rfc3339(),
            topics: parsed.topics,
            attachment_path: None,
        })
        .await
        .map_err(WireError::from_service)?;
    confirmed_summary(app, created).await
}

async fn upload_note(
    app: &Application,
    path: PathBuf,
    project: Option<String>,
    created_at: Option<String>,
) -> Result<AppResponse, WireError> {
    match upload::classify(&path).map_err(Application::db_error)? {
        UploadKind::Text => upload_text(app, &path, project, created_at).await,
        UploadKind::Attachment {
            note_type,
            metadata,
        } => {
            upload_attachment(
                app,
                path,
                project.as_deref(),
                created_at,
                note_type,
                metadata,
            )
            .await
        }
    }
}

async fn upload_text(
    app: &Application,
    path: &PathBuf,
    project: Option<String>,
    created_at: Option<String>,
) -> Result<AppResponse, WireError> {
    let content = tokio::fs::read_to_string(path)
        .await
        .map_err(|error| WireError::from_service(ServiceError::Io(error)))?;
    if content.trim().is_empty() {
        return Err(WireError::from_service(ServiceError::InvalidArgument(
            "content must not be empty".to_string(),
        )));
    }
    let notes = NoteService::new(app.db.as_ref());
    service_result(
        notes
            .add(
                app.creator.as_ref(),
                NoteAddInput {
                    content: content.trim_end().to_string(),
                    project,
                    interpret_as_url: false,
                    topics: Vec::new(),
                    created_at,
                },
            )
            .await,
        AppResponse::NoteSummary,
    )
}

async fn upload_attachment(
    app: &Application,
    path: PathBuf,
    project: Option<&str>,
    created_at: Option<String>,
    note_type: &'static str,
    metadata: String,
) -> Result<AppResponse, WireError> {
    let created = app
        .creator
        .create(CreateNote {
            id: uuid::Uuid::new_v4().to_string(),
            note_type: note_type.to_string(),
            status: "source_queued".to_string(),
            title: None,
            content: None,
            metadata: Some(metadata),
            project_id: resolve_project_id(app, project).await?,
            now: created_at.unwrap_or_else(|| chrono::Utc::now().to_rfc3339()),
            topics: Vec::new(),
            attachment_path: Some(path.to_string_lossy().into_owned()),
        })
        .await
        .map_err(WireError::from_service)?;
    confirmed_summary(app, created).await
}

async fn resolve_project_id(
    app: &Application,
    project: Option<&str>,
) -> Result<Option<String>, WireError> {
    let Some(name) = project else {
        return Ok(None);
    };
    app.db
        .find_project_by_name(name)
        .await
        .map_err(Application::db_error)?
        .map(Some)
        .ok_or_else(|| WireError::from_service(ServiceError::ProjectNotFound(name.to_string())))
}

async fn confirmed_summary(
    app: &Application,
    created: CreatedNote,
) -> Result<AppResponse, WireError> {
    NoteService::new(app.db.as_ref())
        .get(&created.inserted.uuid, false)
        .await
        .map(|detail| AppResponse::NoteSummary(detail.note))
        .map_err(|error| WireError::from_service(confirmed_create_followup_error(&created, &error)))
}

async fn save_editable(
    app: &Application,
    id: &str,
    document: &str,
) -> Result<AppResponse, WireError> {
    let id = app
        .db
        .resolve_note_id(id)
        .await
        .map_err(Application::db_error)?;
    editable_document::save_editable_note(app.db.as_ref(), &id, document)
        .await
        .map(AppResponse::EditableSave)
        .map_err(Application::db_error)
}
