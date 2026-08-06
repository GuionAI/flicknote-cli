use clap::Args;
use flicknote_core::backend::{InsertNoteReq, InsertedNote, NoteDb};
use flicknote_core::config::Config;
use flicknote_core::error::CliError;
use flicknote_core::services::dto::NoteAddInput;
use flicknote_core::services::note::NoteService;
use flicknote_core::services::ports::{DirectNoteCreator, NoteCreator};
use flicknote_sync::ipc::DaemonClient;
use flicknote_sync::ipc::{CreateNoteRequest, DaemonRequest, DaemonResponse};
use std::io::{IsTerminal, Read};

use super::util::{display_summary_id, resolve_project_arg};

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
    Daemon,
}

impl AddCreateMode {
    pub(crate) fn uses_daemon(self) -> bool {
        matches!(self, Self::Daemon)
    }
}

pub(crate) async fn run(
    db: &dyn NoteDb,
    config: &Config,
    args: &AddArgs,
    mode: AddCreateMode,
) -> Result<(), CliError> {
    let content = match &args.value {
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

    let project = resolve_project_arg(&args.project);
    let direct = DirectNoteCreator::new(db);
    let daemon = DaemonClient::new(config);
    let creator: &dyn NoteCreator = if mode.uses_daemon() { &daemon } else { &direct };
    let note = NoteService::new(db)
        .add(
            creator,
            NoteAddInput {
                content,
                project: project.clone(),
                interpret_as_url: args.value.is_some(),
            },
        )
        .await?;
    match project.as_deref() {
        Some(name) => println!(
            "Created note {} in project \"{name}\".",
            display_summary_id(&note)
        ),
        None => println!("Created note {}.", display_summary_id(&note)),
    }
    Ok(())
}

pub(crate) fn daemon_create_request(req: &InsertNoteReq<'_>) -> CreateNoteRequest {
    daemon_create_request_with_topics(req, &[])
}

pub(crate) fn daemon_create_request_with_topics(
    req: &InsertNoteReq<'_>,
    topics: &[String],
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
        attachment_path: None,
    }
}

pub(crate) async fn create_note_with_daemon(
    config: &Config,
    req: CreateNoteRequest,
) -> Result<InsertedNote, CliError> {
    match flicknote_sync::ipc::send_request(config, &DaemonRequest::CreateNote(Box::new(req)))
        .await
        .map_err(|e| CliError::Other(e.to_string()))?
    {
        DaemonResponse::NoteCreated(note) => Ok(InsertedNote {
            uuid: note.uuid,
            short_id: Some(note.short_id),
        }),
        DaemonResponse::Error(e) => Err(CliError::Other(e.to_string())),
        _ => Err(CliError::Other(
            "Sync daemon returned an unexpected response to the create request".into(),
        )),
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
    fn daemon_create_request_can_include_topics() {
        let topics = vec!["rust".to_string()];
        let req = daemon_create_request_with_topics(
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
        );

        assert_eq!(req.topics, topics);
    }
}
