use clap::Args;
use flicknote_core::backend::{InsertNoteReq, InsertedNote, NoteDb};
use flicknote_core::config::Config;
use flicknote_core::error::CliError;
use flicknote_sync::ipc::{CreateNoteRequest, DaemonRequest, DaemonResponse};
use std::io::{IsTerminal, Read};

use super::upload_util::{
    cleanup_uploaded_file, is_readable_text_file, is_uploadable_file, metadata_for_upload,
    note_type_for_extension, upload_file,
};
use super::util::{display_inserted_note_id, print_pending_short_id_hint, resolve_project_arg};

const ADD_HELP: &str = include_str!("../help/add.md");

#[derive(Args)]
#[command(after_help = ADD_HELP)]
pub(crate) struct AddArgs {
    /// Note content or URL. Reads from stdin if omitted.
    value: Option<String>,
    /// Assign to project by name
    #[arg(long)]
    project: Option<String>,
}

#[derive(Clone, Copy)]
pub(crate) enum AddCreateMode {
    Local,
    DaemonForNonFile,
}

pub(crate) async fn run(
    db: &dyn NoteDb,
    config: &Config,
    args: &AddArgs,
    mode: AddCreateMode,
) -> Result<(), CliError> {
    let id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();

    let mut content = match &args.value {
        Some(v) => v.to_owned(),
        None => {
            if std::io::stdin().is_terminal() {
                return Err(CliError::Other(
                    "No content provided. Pass a value or pipe from stdin.".into(),
                ));
            }
            let mut buf = String::new();
            std::io::stdin().read_to_string(&mut buf)?;
            let trimmed = buf.trim_end().to_string();
            if trimmed.is_empty() {
                return Err(CliError::Other("No content provided".into()));
            }
            trimmed
        }
    };

    let from_arg = args.value.is_some();
    let is_url_arg = content.starts_with("http://") || content.starts_with("https://");

    let path = std::path::Path::new(&content);
    let looks_like_file_path =
        from_arg && !is_url_arg && path.extension().is_some() && path.file_name().is_some();

    let is_text_file = from_arg && looks_like_file_path && is_readable_text_file(&content);

    if from_arg && looks_like_file_path && !is_uploadable_file(&content) && !is_text_file {
        return Err(CliError::Other(format!(
            "File not found or unsupported: {}",
            content
        )));
    }

    // Read text file content into `content` so the rest of the function treats it as normal text
    if is_text_file {
        let path = content.clone();
        content = std::fs::read_to_string(&path)
            .map_err(|e| CliError::Other(format!("Failed to read {}: {}", path, e)))?;
        content = content.trim_end().to_string();
    }

    let is_file = from_arg && !is_text_file && is_uploadable_file(&content);
    let is_url = !is_file && is_url_arg;

    let effective_project = resolve_project_arg(&args.project);
    let project_id = if let Some(ref name) = effective_project {
        Some(resolve_project(db, name).await?)
    } else {
        None
    };

    let inserted = if is_file {
        let file_path = std::path::PathBuf::from(&content);
        let filename = file_path
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| CliError::Other("Invalid filename".into()))?
            .to_string();

        upload_file(config, &id, &file_path).await?;

        let note_type = note_type_for_extension(&filename);
        let metadata = metadata_for_upload(&filename);

        match db
            .insert_note(&InsertNoteReq {
                id: &id,
                note_type,
                status: "source_queued",
                title: None,
                content: None,
                metadata: Some(&metadata),
                project_id: project_id.as_deref(),
                now: &now,
            })
            .await
        {
            Ok(inserted) => inserted,
            Err(e) => {
                #[allow(clippy::let_underscore_must_use, clippy::let_underscore_untyped)]
                let _ = cleanup_uploaded_file(config, &id).await;
                return Err(e);
            }
        }
    } else if is_url {
        let metadata = serde_json::json!({ "link": { "url": &content } }).to_string();
        if matches!(mode, AddCreateMode::DaemonForNonFile) {
            create_note_with_daemon(
                config,
                daemon_create_request(&InsertNoteReq {
                    id: &id,
                    note_type: "link",
                    status: "source_queued",
                    title: None,
                    content: None,
                    metadata: Some(&metadata),
                    project_id: project_id.as_deref(),
                    now: &now,
                }),
            )
            .await?
        } else {
            db.insert_note(&InsertNoteReq {
                id: &id,
                note_type: "link",
                status: "source_queued",
                title: None,
                content: None,
                metadata: Some(&metadata),
                project_id: project_id.as_deref(),
                now: &now,
            })
            .await?
        }
    } else {
        let (title, stripped_content) = crate::utils::extract_title_and_strip(&content);
        let title_ref = title.as_deref();
        if matches!(mode, AddCreateMode::DaemonForNonFile) {
            create_note_with_daemon(
                config,
                daemon_create_request(&InsertNoteReq {
                    id: &id,
                    note_type: "normal",
                    status: "ai_queued",
                    title: title_ref,
                    content: Some(&stripped_content),
                    metadata: None,
                    project_id: project_id.as_deref(),
                    now: &now,
                }),
            )
            .await?
        } else {
            db.insert_note(&InsertNoteReq {
                id: &id,
                note_type: "normal",
                status: "ai_queued",
                title: title_ref,
                content: Some(&stripped_content),
                metadata: None,
                project_id: project_id.as_deref(),
                now: &now,
            })
            .await?
        }
    };

    match effective_project.as_deref() {
        Some(name) => println!(
            "Created note {} in project \"{name}\".",
            display_inserted_note_id(&inserted)
        ),
        None => println!("Created note {}.", display_inserted_note_id(&inserted)),
    }
    if inserted.short_id.is_none() {
        print_pending_short_id_hint();
    }
    Ok(())
}

pub(crate) fn daemon_create_request(req: &InsertNoteReq<'_>) -> CreateNoteRequest {
    daemon_create_request_with_extractions(req, &[], &[])
}

pub(crate) fn daemon_create_request_with_extractions(
    req: &InsertNoteReq<'_>,
    topics: &[String],
    entities: &[String],
) -> CreateNoteRequest {
    CreateNoteRequest {
        id: req.id.to_string(),
        note_type: req.note_type.to_string(),
        status: req.status.to_string(),
        title: req.title.map(str::to_string),
        content: req.content.map(str::to_string),
        metadata: req.metadata.map(str::to_string),
        project_id: req.project_id.map(str::to_string),
        now: req.now.to_string(),
        topics: topics.to_vec(),
        entities: entities.to_vec(),
    }
}

pub(crate) async fn create_note_with_daemon(
    config: &Config,
    req: CreateNoteRequest,
) -> Result<InsertedNote, CliError> {
    match flicknote_sync::ipc::send_request(config, &DaemonRequest::CreateNote(req))
        .await
        .map_err(|e| CliError::Other(e.to_string()))?
    {
        DaemonResponse::NoteCreated(note) => Ok(InsertedNote {
            uuid: note.uuid,
            short_id: Some(note.short_id),
        }),
        DaemonResponse::Error(e) => Err(CliError::Other(e.to_string())),
    }
}

/// Resolve project by name. Returns an error with a hint if the project doesn't exist.
pub(crate) async fn resolve_project(db: &dyn NoteDb, name: &str) -> Result<String, CliError> {
    match db.find_project_by_name(name).await? {
        Some(id) => Ok(id),
        None => Err(CliError::ProjectNotFound {
            name: name.to_string(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn daemon_create_request_keeps_normal_note_fields() {
        let req = daemon_create_request(&InsertNoteReq {
            id: "note-id",
            note_type: "normal",
            status: "ai_queued",
            title: Some("Title"),
            content: Some("Body"),
            metadata: None,
            project_id: Some("project-id"),
            now: "2026-06-26T00:00:00Z",
        });

        assert_eq!(req.id, "note-id");
        assert_eq!(req.note_type, "normal");
        assert_eq!(req.status, "ai_queued");
        assert_eq!(req.title.as_deref(), Some("Title"));
        assert_eq!(req.content.as_deref(), Some("Body"));
        assert_eq!(req.metadata, None);
        assert_eq!(req.project_id.as_deref(), Some("project-id"));
        assert_eq!(req.now, "2026-06-26T00:00:00Z");
    }

    #[test]
    fn daemon_create_request_keeps_link_metadata() {
        let metadata = serde_json::json!({ "link": { "url": "https://example.com" } }).to_string();
        let req = daemon_create_request(&InsertNoteReq {
            id: "note-id",
            note_type: "link",
            status: "source_queued",
            title: None,
            content: None,
            metadata: Some(&metadata),
            project_id: None,
            now: "2026-06-26T00:00:00Z",
        });

        assert_eq!(req.note_type, "link");
        assert_eq!(req.status, "source_queued");
        assert_eq!(req.metadata.as_deref(), Some(metadata.as_str()));
    }

    #[test]
    fn daemon_create_request_can_include_extractions() {
        let topics = vec!["rust".to_string()];
        let entities = vec!["PowerSync".to_string()];
        let req = daemon_create_request_with_extractions(
            &InsertNoteReq {
                id: "note-id",
                note_type: "normal",
                status: "ai_queued",
                title: Some("Title"),
                content: Some("Body"),
                metadata: None,
                project_id: None,
                now: "2026-06-26T00:00:00Z",
            },
            &topics,
            &entities,
        );

        assert_eq!(req.topics, topics);
        assert_eq!(req.entities, entities);
    }
}
