use std::sync::Arc;

use flicknote_core::backend::NoteDb;
use flicknote_core::services::dto::NoteAddInput;
use flicknote_core::services::error::ServiceError;
use flicknote_core::services::note::{NoteService, confirmed_create_followup_error};
use flicknote_core::services::ports::{CreateNote, DirectNoteCreator, NoteCreator, ShareGateway};
use flicknote_core::services::project::ProjectService;
use flicknote_core::services::upload::{self, UploadKind};

use crate::ipc::{AppRequest, AppResponse, BackendMode, WireError};

pub struct Application {
    db: Arc<dyn NoteDb>,
    mode: BackendMode,
    creator: Option<Arc<dyn NoteCreator>>,
    share_gateway: Option<Arc<dyn ShareGateway>>,
    web_url: Option<String>,
    write_signal: Option<tokio::sync::mpsc::Sender<()>>,
}

impl Application {
    pub fn new(db: Arc<dyn NoteDb>, mode: BackendMode) -> Self {
        Self {
            db,
            mode,
            creator: None,
            share_gateway: None,
            web_url: None,
            write_signal: None,
        }
    }

    pub fn with_creator(mut self, creator: Arc<dyn NoteCreator>) -> Self {
        self.creator = Some(creator);
        self
    }

    pub fn with_share_gateway(mut self, gateway: Arc<dyn ShareGateway>) -> Self {
        self.share_gateway = Some(gateway);
        self
    }

    pub fn with_web_url(mut self, web_url: Option<String>) -> Self {
        self.web_url = web_url;
        self
    }

    pub fn with_write_signal(mut self, signal: tokio::sync::mpsc::Sender<()>) -> Self {
        self.write_signal = Some(signal);
        self
    }

    pub fn mode(&self) -> BackendMode {
        self.mode
    }

    pub async fn handle(&self, request: AppRequest) -> Result<AppResponse, WireError> {
        let required = request.required_capability();
        if !self.mode.supports(required) {
            return Err(Self::unsupported(
                request.operation_name(),
                required,
                self.mode,
            ));
        }
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
        let notes = NoteService::new(self.db.as_ref());
        let projects = ProjectService::new(self.db.as_ref());
        match request {
            AppRequest::NoteAdd(input) => {
                if let Some(creator) = self.creator.as_deref() {
                    return notes
                        .add(creator, input)
                        .await
                        .map(AppResponse::NoteSummary)
                        .map_err(WireError::from_service);
                }
                if self.mode == BackendMode::Managed {
                    return notes
                        .add(&DirectNoteCreator::new(self.db.as_ref()), input)
                        .await
                        .map(AppResponse::NoteSummary)
                        .map_err(WireError::from_service);
                }
                Err(Self::unsupported(
                    "note_add",
                    crate::ipc::Capability::NoteAdd,
                    self.mode,
                ))
            }
            AppRequest::NoteAddEditable { document, project } => {
                let parsed =
                    flicknote_core::services::editable_document::parse_editable_note(&document)
                        .map_err(Self::db_error)?;
                let project_id = match project.as_deref() {
                    Some(name) => Some(
                        self.db
                            .find_project_by_name(name)
                            .await
                            .map_err(Self::db_error)?
                            .ok_or_else(|| {
                                WireError::from_service(ServiceError::ProjectNotFound(
                                    name.to_string(),
                                ))
                            })?,
                    ),
                    None => None,
                };
                let request = CreateNote {
                    id: uuid::Uuid::new_v4().to_string(),
                    note_type: "normal".to_string(),
                    status: "ai_queued".to_string(),
                    title: Some(parsed.title),
                    content: Some(parsed.stored_content),
                    metadata: None,
                    project_id,
                    now: chrono::Utc::now().to_rfc3339(),
                    topics: parsed.topics,
                    attachment_path: None,
                };
                let created = if let Some(creator) = self.creator.as_deref() {
                    creator.create(request).await
                } else if self.mode == BackendMode::Managed {
                    DirectNoteCreator::new(self.db.as_ref())
                        .create(request)
                        .await
                } else {
                    return Err(Self::unsupported(
                        "note_add_editable",
                        crate::ipc::Capability::Editor,
                        self.mode,
                    ));
                }
                .map_err(WireError::from_service)?;
                notes
                    .get(&created.inserted.uuid, false)
                    .await
                    .map(|detail| AppResponse::NoteSummary(detail.note))
                    .map_err(|error| {
                        WireError::from_service(confirmed_create_followup_error(&created, &error))
                    })
            }
            AppRequest::NoteUpload {
                path,
                project,
                created_at,
            } => {
                let path = std::path::PathBuf::from(path);
                match upload::classify(&path).map_err(Self::db_error)? {
                    UploadKind::Text => {
                        let content = std::fs::read_to_string(&path)
                            .map_err(|error| WireError::from_service(ServiceError::Io(error)))?;
                        if content.trim().is_empty() {
                            return Err(WireError::from_service(ServiceError::InvalidArgument(
                                "content must not be empty".to_string(),
                            )));
                        }
                        let input = NoteAddInput {
                            content: content.trim_end().to_string(),
                            project,
                            interpret_as_url: false,
                            topics: Vec::new(),
                            created_at,
                        };
                        if let Some(creator) = self.creator.as_deref() {
                            notes.add(creator, input).await
                        } else if self.mode == BackendMode::Managed {
                            notes
                                .add(&DirectNoteCreator::new(self.db.as_ref()), input)
                                .await
                        } else {
                            return Err(Self::unsupported(
                                "note_upload",
                                crate::ipc::Capability::Attachment,
                                self.mode,
                            ));
                        }
                        .map(AppResponse::NoteSummary)
                        .map_err(WireError::from_service)
                    }
                    UploadKind::Attachment {
                        note_type,
                        metadata,
                    } => {
                        let creator = self.creator.as_deref().ok_or_else(|| {
                            Self::unsupported(
                                "attachment",
                                crate::ipc::Capability::Attachment,
                                self.mode,
                            )
                        })?;
                        let project_id = match project.as_deref() {
                            Some(name) => Some(
                                self.db
                                    .find_project_by_name(name)
                                    .await
                                    .map_err(Self::db_error)?
                                    .ok_or_else(|| {
                                        WireError::from_service(ServiceError::ProjectNotFound(
                                            name.to_string(),
                                        ))
                                    })?,
                            ),
                            None => None,
                        };
                        let created = creator
                            .create(CreateNote {
                                id: uuid::Uuid::new_v4().to_string(),
                                note_type: note_type.to_string(),
                                status: "source_queued".to_string(),
                                title: None,
                                content: None,
                                metadata: Some(metadata),
                                project_id,
                                now: created_at.unwrap_or_else(|| chrono::Utc::now().to_rfc3339()),
                                topics: Vec::new(),
                                attachment_path: Some(path.to_string_lossy().into_owned()),
                            })
                            .await
                            .map_err(WireError::from_service)?;
                        notes
                            .get(&created.inserted.uuid, false)
                            .await
                            .map(|detail| AppResponse::NoteSummary(detail.note))
                            .map_err(|error| {
                                WireError::from_service(confirmed_create_followup_error(
                                    &created, &error,
                                ))
                            })
                    }
                }
            }
            AppRequest::NoteList(input) => notes
                .list(input)
                .await
                .map(AppResponse::NoteSummaries)
                .map_err(WireError::from_service),
            AppRequest::NoteAppend { id, content } => notes
                .append(&id, &content)
                .await
                .map(AppResponse::NoteMutation)
                .map_err(WireError::from_service),
            AppRequest::NoteSaveEditable { id, document } => {
                let id = self.db.resolve_note_id(&id).await.map_err(Self::db_error)?;
                flicknote_core::services::editable_document::save_editable_note(
                    self.db.as_ref(),
                    &id,
                    &document,
                )
                .await
                .map(AppResponse::EditableSave)
                .map_err(Self::db_error)
            }
            AppRequest::NoteFind(input) => notes
                .find(input)
                .await
                .map(AppResponse::NoteSummaries)
                .map_err(WireError::from_service),
            AppRequest::NoteCount(input) => notes
                .count(input)
                .await
                .map(|count| AppResponse::NoteCount { count })
                .map_err(WireError::from_service),
            AppRequest::NoteGet { id, archived } => notes
                .get(&id, archived)
                .await
                .map(AppResponse::NoteDetail)
                .map_err(WireError::from_service),
            AppRequest::NoteLoadEditable { id } => {
                let id = self.db.resolve_note_id(&id).await.map_err(Self::db_error)?;
                flicknote_core::services::editable_document::load_editable_note(
                    self.db.as_ref(),
                    &id,
                )
                .await
                .map(|document| {
                    AppResponse::EditableDocument(crate::ipc::EditableDocument { document })
                })
                .map_err(Self::db_error)
            }
            AppRequest::NoteRecord { id, archived } => {
                let id = if archived {
                    self.db.resolve_archived_note_id(&id).await
                } else {
                    self.db.resolve_note_id(&id).await
                }
                .map_err(Self::db_error)?;
                let note = if archived {
                    self.db.find_archived_note(&id).await
                } else {
                    self.db.find_note(&id).await
                }
                .map_err(Self::db_error)?;
                Ok(AppResponse::NoteRecord(note))
            }
            AppRequest::NoteGetSection { id, section } => notes
                .get_section(&id, &section)
                .await
                .map(AppResponse::NoteSection)
                .map_err(WireError::from_service),
            AppRequest::NoteSource {
                id,
                archived,
                view,
                range,
            } => notes
                .source(&id, archived, view, range.as_deref())
                .await
                .map(AppResponse::Source)
                .map_err(WireError::from_service),
            AppRequest::NoteReplaceSection {
                id,
                section,
                content,
            } => notes
                .replace_section(&id, &section, &content)
                .await
                .map(AppResponse::NoteMutation)
                .map_err(WireError::from_service),
            AppRequest::NoteRenameSection { id, section, name } => notes
                .rename_section(&id, &section, &name)
                .await
                .map(AppResponse::NoteMutation)
                .map_err(WireError::from_service),
            AppRequest::NoteInsert {
                id,
                section,
                position,
                content,
            } => notes
                .insert(&id, &section, position, &content)
                .await
                .map(AppResponse::NoteMutation)
                .map_err(WireError::from_service),
            AppRequest::NoteDeleteSection { id, section } => notes
                .delete_section(&id, &section)
                .await
                .map(AppResponse::NoteMutation)
                .map_err(WireError::from_service),
            AppRequest::NoteModify(input) => notes
                .modify(input)
                .await
                .map(AppResponse::NoteMutation)
                .map_err(WireError::from_service),
            AppRequest::NoteArchive { id } => notes
                .archive(&id)
                .await
                .map(AppResponse::NoteArchive)
                .map_err(WireError::from_service),
            AppRequest::NoteRestore { id } => notes
                .restore(&id)
                .await
                .map(AppResponse::NoteArchive)
                .map_err(WireError::from_service),
            AppRequest::NoteShare { id } => {
                let gateway = self.share_gateway.as_deref().ok_or_else(|| {
                    Self::unsupported("note_share", crate::ipc::Capability::Share, self.mode)
                })?;
                notes
                    .share(gateway, &id)
                    .await
                    .map(AppResponse::Share)
                    .map_err(WireError::from_service)
            }
            AppRequest::NoteUnshare { id } => {
                let gateway = self.share_gateway.as_deref().ok_or_else(|| {
                    Self::unsupported("note_unshare", crate::ipc::Capability::Share, self.mode)
                })?;
                notes
                    .unshare(gateway, &id)
                    .await
                    .map(AppResponse::Unshare)
                    .map_err(WireError::from_service)
            }
            AppRequest::NoteOpen { id } => {
                let web_url = self.web_url.as_deref().ok_or_else(|| {
                    WireError::from_service(ServiceError::ConfigMissing("webUrl".to_string()))
                })?;
                let full_id = self.db.resolve_note_id(&id).await.map_err(Self::db_error)?;
                let note = self.db.find_note(&full_id).await.map_err(Self::db_error)?;
                let url_id = note.short_id.map_or(full_id, |value| value.to_string());
                Ok(AppResponse::Open(
                    flicknote_core::services::dto::OpenResult {
                        url: format!("{}/notes/{url_id}", web_url.trim_end_matches('/')),
                        opened: false,
                    },
                ))
            }
            AppRequest::ProjectList { include_archived } => projects
                .list(include_archived)
                .await
                .map(AppResponse::Projects)
                .map_err(WireError::from_service),
            AppRequest::ProjectRecords { include_archived } => {
                let mut records = self.db.list_projects(false).await.map_err(Self::db_error)?;
                if include_archived {
                    records.extend(self.db.list_projects(true).await.map_err(Self::db_error)?);
                    records.sort_by(|left, right| right.created_at.cmp(&left.created_at));
                }
                Ok(AppResponse::ProjectRecords(records))
            }
            AppRequest::ProjectGet { id } => projects
                .get(&id)
                .await
                .map(AppResponse::Project)
                .map_err(WireError::from_service),
            AppRequest::ProjectGetByName { name } => {
                let id = self
                    .db
                    .find_project_by_name(&name)
                    .await
                    .map_err(Self::db_error)?
                    .ok_or_else(|| {
                        WireError::from_service(ServiceError::ProjectNotFound(name.clone()))
                    })?;
                projects
                    .get(&id)
                    .await
                    .map(AppResponse::Project)
                    .map_err(WireError::from_service)
            }
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
            AppRequest::ProjectShare { id } => {
                let gateway = self.share_gateway.as_deref().ok_or_else(|| {
                    Self::unsupported("project_share", crate::ipc::Capability::Share, self.mode)
                })?;
                projects
                    .share(gateway, &id)
                    .await
                    .map(AppResponse::Share)
                    .map_err(WireError::from_service)
            }
            AppRequest::ProjectUnshare { id } => {
                let gateway = self.share_gateway.as_deref().ok_or_else(|| {
                    Self::unsupported("project_unshare", crate::ipc::Capability::Share, self.mode)
                })?;
                projects
                    .unshare(gateway, &id)
                    .await
                    .map(AppResponse::Unshare)
                    .map_err(WireError::from_service)
            }
            AppRequest::ExtractionValues { keys, archived } => {
                let refs = keys.iter().map(String::as_str).collect::<Vec<_>>();
                self.db
                    .list_extraction_values(&refs, archived)
                    .await
                    .map(AppResponse::Values)
                    .map_err(Self::db_error)
            }
        }
    }

    fn unsupported(
        operation: &str,
        capability: crate::ipc::Capability,
        mode: BackendMode,
    ) -> WireError {
        WireError::from_service(crate::ipc::unsupported_capability(
            mode, capability, operation,
        ))
    }

    fn db_error(error: flicknote_core::error::CliError) -> WireError {
        WireError::from_service(ServiceError::from(error))
    }
}
