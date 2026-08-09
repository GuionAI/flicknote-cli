use std::sync::Arc;

use async_trait::async_trait;
use flicknote_core::backend::{InsertNoteReq, InsertedNote, NoteDb, SqliteBackend};
use flicknote_core::config::{Config, ConfigPaths};
use flicknote_core::db::Database;
use flicknote_core::services::dto::{NoteAddInput, NoteListInput, ProjectAddInput};
use flicknote_core::services::error::ServiceError;
use flicknote_core::services::ports::{CreateNote, CreatedNote, NoteCreator};
use flicknote_sync::app::Application;
use flicknote_sync::ipc::{
    AppRequest, AppResponse, BackendMode, DaemonClient, ServerInfo, serve_app_once,
};

fn test_config(directory: &std::path::Path) -> Config {
    Config {
        supabase_url: String::new(),
        supabase_anon_key: String::new(),
        powersync_url: String::new(),
        api_url: String::new(),
        web_url: None,
        paths: ConfigPaths {
            config_dir: directory.to_path_buf(),
            data_dir: directory.to_path_buf(),
            config_file: directory.join("config.json"),
            session_file: directory.join("session.json"),
            db_file: directory.join("flicknote.db"),
            log_file: directory.join("sync.log"),
        },
    }
}

#[test]
fn application_is_safe_to_share_between_daemon_request_tasks() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<Application>();
}

#[tokio::test]
async fn application_signals_every_may_write_request_even_when_it_fails() {
    let directory = tempfile::tempdir().unwrap();
    let config = test_config(directory.path());
    let backend = Arc::new(SqliteBackend {
        db: Database::open_local(&config).await.unwrap(),
        user_id: "user-1".to_string(),
    });
    let (signal, mut receiver) = tokio::sync::mpsc::channel(4);
    let app = Application::new(backend, BackendMode::Local).with_write_signal(signal);

    app.handle(AppRequest::NoteList(NoteListInput {
        note_type: None,
        project: None,
        archived: false,
        limit: 20,
    }))
    .await
    .unwrap();
    assert!(receiver.try_recv().is_err());

    let error = app
        .handle(AppRequest::KeytermModify {
            id: "missing".to_string(),
            name: None,
            description: None,
            content: None,
        })
        .await
        .unwrap_err();
    assert_eq!(error.code, "nothing_to_modify");
    receiver.try_recv().unwrap();
}

struct RecordingCreator {
    db: Arc<dyn NoteDb>,
    request: std::sync::Mutex<Option<CreateNote>>,
}

#[async_trait]
impl NoteCreator for RecordingCreator {
    async fn create(&self, request: CreateNote) -> Result<CreatedNote, ServiceError> {
        let inserted = self.db.insert_note(&request.as_insert_request()).await?;
        *self.request.lock().unwrap() = Some(request);
        Ok(CreatedNote {
            inserted,
            confirmed_extraction_ids: Vec::new(),
        })
    }
}

struct DetachedCreator;

#[async_trait]
impl NoteCreator for DetachedCreator {
    async fn create(&self, request: CreateNote) -> Result<CreatedNote, ServiceError> {
        Ok(CreatedNote {
            inserted: InsertedNote {
                uuid: request.id,
                short_id: Some(91),
            },
            confirmed_extraction_ids: Vec::new(),
        })
    }
}

#[tokio::test]
async fn app_preserves_created_identity_when_editor_or_attachment_summary_fails() {
    let directory = tempfile::tempdir().unwrap();
    let config = test_config(directory.path());
    let attachment = directory.path().join("report.pdf");
    std::fs::write(&attachment, b"pdf").unwrap();
    let backend = Arc::new(SqliteBackend {
        db: Database::open_local(&config).await.unwrap(),
        user_id: "user-1".to_string(),
    });
    let app = Application::new(backend, BackendMode::Local).with_creator(Arc::new(DetachedCreator));

    for request in [
        AppRequest::NoteAddEditable {
            document: "# editor-created".to_string(),
            project: None,
        },
        AppRequest::NoteUpload {
            path: attachment.to_string_lossy().into_owned(),
            project: None,
            created_at: None,
        },
    ] {
        let error = app.handle(request).await.unwrap_err();
        assert_eq!(error.code, "note_create_partial");
        let details = error.details.unwrap();
        assert_eq!(details["created"], true);
        assert_eq!(details["short_id"], 91);
        assert!(details["note_id"].as_str().is_some());
    }
}

#[tokio::test]
async fn app_routes_note_list_and_append_through_services() {
    const NOTE_ID: &str = "550e8400-e29b-41d4-a716-446655440000";
    let directory = tempfile::tempdir().unwrap();
    let config = test_config(directory.path());
    let backend = Arc::new(SqliteBackend {
        db: Database::open_local(&config).await.unwrap(),
        user_id: "user-1".to_string(),
    });
    backend
        .insert_note(&InsertNoteReq {
            id: NOTE_ID,
            note_type: "normal",
            status: "ready",
            title: Some("Title"),
            content: Some("Body"),
            metadata: None,
            project_id: None,
            now: "2026-08-09T00:00:00Z",
        })
        .await
        .unwrap();
    let app = Application::new(backend.clone(), BackendMode::Local);

    let listed = app
        .handle(AppRequest::NoteList(NoteListInput {
            note_type: None,
            project: None,
            archived: false,
            limit: 20,
        }))
        .await
        .unwrap();
    let AppResponse::NoteSummaries(notes) = listed else {
        panic!("unexpected list response")
    };
    assert_eq!(notes.len(), 1);
    assert_eq!(notes[0].uuid, NOTE_ID);

    let raw = app
        .handle(AppRequest::NoteRecord {
            id: NOTE_ID.to_string(),
            archived: false,
        })
        .await
        .unwrap();
    let AppResponse::NoteRecord(raw) = raw else {
        panic!("unexpected raw note response")
    };
    assert_eq!(raw.content.as_deref(), Some("Body"));

    let appended = app
        .handle(AppRequest::NoteAppend {
            id: NOTE_ID.to_string(),
            content: "More".to_string(),
        })
        .await
        .unwrap();
    let AppResponse::NoteMutation(result) = appended else {
        panic!("unexpected append response")
    };
    assert_eq!(result.note.uuid, NOTE_ID);
    assert_eq!(
        backend.find_note_content(NOTE_ID).await.unwrap(),
        Some("Body\n\nMore".to_string())
    );
}

#[tokio::test]
async fn app_owns_project_keyterm_and_catalog_domain_operations() {
    let directory = tempfile::tempdir().unwrap();
    let config = test_config(directory.path());
    let backend = Arc::new(SqliteBackend {
        db: Database::open_local(&config).await.unwrap(),
        user_id: "user-1".to_string(),
    });
    let app = Application::new(backend, BackendMode::Local);

    let keyterm = app
        .handle(AppRequest::KeytermAdd {
            name: "Rust".to_string(),
            description: Some("Language".to_string()),
            content: Some("ownership".to_string()),
        })
        .await
        .unwrap();
    let AppResponse::Keyterm(keyterm) = keyterm else {
        panic!("unexpected keyterm response")
    };
    assert_eq!(keyterm.name, "Rust");

    let project = app
        .handle(AppRequest::ProjectAdd(ProjectAddInput {
            name: "work".to_string(),
            keyterm: Some(keyterm.id.clone()),
            color: Some("#123456".to_string()),
        }))
        .await
        .unwrap();
    let AppResponse::Project(project) = project else {
        panic!("unexpected project response")
    };
    assert_eq!(project.name, "work");
    assert_eq!(project.keyterm_id.as_deref(), Some(keyterm.id.as_str()));

    let values = app
        .handle(AppRequest::ExtractionValues {
            keys: vec!["::topic".to_string()],
            archived: false,
        })
        .await
        .unwrap();
    let AppResponse::Values(values) = values else {
        panic!("unexpected catalog response")
    };
    assert!(values.is_empty());
}

#[tokio::test]
async fn versioned_socket_routes_client_requests_through_application() {
    let directory = tempfile::tempdir().unwrap();
    let config = test_config(directory.path());
    let backend = Arc::new(SqliteBackend {
        db: Database::open_local(&config).await.unwrap(),
        user_id: "user-1".to_string(),
    });
    let app = Arc::new(Application::new(backend, BackendMode::Local));
    let listener =
        tokio::net::UnixListener::bind(flicknote_sync::ipc::socket_path(&config)).unwrap();
    let server = tokio::spawn(serve_app_once(listener, app, ServerInfo::local()));

    let client = DaemonClient::new(&config);
    let info = client.health().await.unwrap();
    assert_eq!(info.backend, BackendMode::Local);
    server.await.unwrap().unwrap();

    std::fs::remove_file(flicknote_sync::ipc::socket_path(&config)).unwrap();
    let listener =
        tokio::net::UnixListener::bind(flicknote_sync::ipc::socket_path(&config)).unwrap();
    let directory2 = tempfile::tempdir().unwrap();
    let config2 = test_config(directory2.path());
    let backend = Arc::new(SqliteBackend {
        db: Database::open_local(&config2).await.unwrap(),
        user_id: "user-1".to_string(),
    });
    let app = Arc::new(Application::new(backend, BackendMode::Local));
    let server = tokio::spawn(serve_app_once(listener, app, ServerInfo::local()));
    let response = client
        .app(AppRequest::NoteList(NoteListInput {
            note_type: None,
            project: None,
            archived: false,
            limit: 20,
        }))
        .await
        .unwrap();
    assert!(matches!(response, AppResponse::NoteSummaries(notes) if notes.is_empty()));
    server.await.unwrap().unwrap();
}

#[tokio::test]
async fn managed_app_adds_note_and_topics_through_the_backend() {
    let directory = tempfile::tempdir().unwrap();
    let config = test_config(directory.path());
    let backend = Arc::new(SqliteBackend {
        db: Database::open_local(&config).await.unwrap(),
        user_id: "user-1".to_string(),
    });
    let app = Application::new(backend.clone(), BackendMode::Managed);

    let response = app
        .handle(AppRequest::NoteAdd(NoteAddInput {
            content: "# Title\n\nBody".to_string(),
            project: None,
            interpret_as_url: false,
            topics: vec!["rust".to_string()],
            created_at: Some("2026-01-02T03:04:05Z".to_string()),
        }))
        .await
        .unwrap();
    let AppResponse::NoteSummary(note) = response else {
        panic!("unexpected add response")
    };
    assert_eq!(note.title.as_deref(), Some("Title"));
    assert_eq!(note.created_at.as_deref(), Some("2026-01-02T03:04:05Z"));
    assert_eq!(
        backend
            .list_note_topics(&[note.uuid.as_str()])
            .await
            .unwrap()
            .get(&note.uuid)
            .cloned()
            .unwrap_or_default(),
        vec!["rust".to_string()]
    );
}

#[tokio::test]
async fn managed_app_rejects_local_only_workflows() {
    let directory = tempfile::tempdir().unwrap();
    let config = test_config(directory.path());
    let upload = directory.path().join("note.md");
    std::fs::write(&upload, "# imported").unwrap();
    let backend = Arc::new(SqliteBackend {
        db: Database::open_local(&config).await.unwrap(),
        user_id: "user-1".to_string(),
    });
    let app = Application::new(backend, BackendMode::Managed);

    for request in [
        AppRequest::NoteAddEditable {
            document: "# editor-created".to_string(),
            project: None,
        },
        AppRequest::NoteUpload {
            path: upload.to_string_lossy().into_owned(),
            project: None,
            created_at: None,
        },
        AppRequest::NoteLoadEditable {
            id: "missing".to_string(),
        },
        AppRequest::NoteOpen {
            id: "missing".to_string(),
        },
        AppRequest::NoteShare {
            id: "missing".to_string(),
        },
    ] {
        let error = app.handle(request).await.unwrap_err();
        assert_eq!(error.code, "unsupported_capability");
    }
}

#[tokio::test]
async fn local_app_owns_attachment_normalization_and_creator_call() {
    let directory = tempfile::tempdir().unwrap();
    let config = test_config(directory.path());
    let path = directory.path().join("report.pdf");
    std::fs::write(&path, b"pdf").unwrap();
    let backend = Arc::new(SqliteBackend {
        db: Database::open_local(&config).await.unwrap(),
        user_id: "user-1".to_string(),
    });
    let creator = Arc::new(RecordingCreator {
        db: backend.clone(),
        request: std::sync::Mutex::new(None),
    });
    let app = Application::new(backend, BackendMode::Local).with_creator(creator.clone());

    let response = app
        .handle(AppRequest::NoteUpload {
            path: path.to_string_lossy().into_owned(),
            project: None,
            created_at: None,
        })
        .await
        .unwrap();
    assert!(matches!(response, AppResponse::NoteSummary(_)));
    let request = creator.request.lock().unwrap();
    let request = request.as_ref().unwrap();
    assert_eq!(request.note_type, "file");
    assert_eq!(request.attachment_path.as_deref(), path.to_str());
    assert!(request.metadata.as_deref().unwrap().contains("report.pdf"));
}

#[tokio::test]
async fn app_owns_editable_document_parsing_and_persistence() {
    let directory = tempfile::tempdir().unwrap();
    let config = test_config(directory.path());
    let backend = Arc::new(SqliteBackend {
        db: Database::open_local(&config).await.unwrap(),
        user_id: "user-1".to_string(),
    });
    let creator = Arc::new(RecordingCreator {
        db: backend.clone(),
        request: std::sync::Mutex::new(None),
    });
    let app = Application::new(backend.clone(), BackendMode::Local).with_creator(creator);

    let created = app
        .handle(AppRequest::NoteAddEditable {
            document: "---\ntitle: First\ntopics: [rust]\n---\n\nBody".to_string(),
            project: None,
        })
        .await
        .unwrap();
    let AppResponse::NoteSummary(created) = created else {
        panic!("unexpected editable create response")
    };
    assert_eq!(created.title.as_deref(), Some("First"));

    let saved = app
        .handle(AppRequest::NoteSaveEditable {
            id: created.uuid.clone(),
            document: "---\ntitle: Second\ntopics: [rust, daemon]\n---\n\nChanged".to_string(),
        })
        .await
        .unwrap();
    let AppResponse::EditableSave(saved) = saved else {
        panic!("unexpected editable save response")
    };
    assert!(saved.title_changed);
    assert!(saved.content_changed);
    assert_eq!(
        backend
            .find_note(&created.uuid)
            .await
            .unwrap()
            .title
            .as_deref(),
        Some("Second")
    );
}
