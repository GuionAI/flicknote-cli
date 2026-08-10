use std::sync::Arc;

use async_trait::async_trait;
use flicknote_core::backend::{InsertNoteReq, InsertedNote, LocalPowerSyncBackend, NoteDb};
use flicknote_core::config::{Config, ConfigPaths};
use flicknote_core::schema::app_schema;
use flicknote_core::services::dto::{NoteListInput, Patch, ProjectAddInput, ProjectModifyInput};
use flicknote_core::services::error::ServiceError;
use flicknote_core::services::ports::{
    CreateNote, CreatedNote, NoteCreator, ShareGateway, ShareResource,
};
use flicknote_sync::app::Application;
use flicknote_sync::ipc::{
    AppRequest, AppResponse, DaemonClient, DaemonRequest, DaemonResponse, ServerInfo,
    serve_app_once, socket_path,
};
use powersync::{ConnectionPool, PowerSyncDatabase, env::PowerSyncEnvironment};

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

fn test_backend(config: &Config) -> Arc<LocalPowerSyncBackend> {
    PowerSyncEnvironment::powersync_auto_extension().unwrap();
    let pool = ConnectionPool::open(&config.paths.db_file).unwrap();
    let environment = PowerSyncEnvironment::custom(
        reqwest::Client::new(),
        pool,
        PowerSyncEnvironment::tokio_timer(),
    );
    let db = PowerSyncDatabase::new(environment, app_schema());
    Arc::new(LocalPowerSyncBackend::new(db, "user-1".to_string()))
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
    let backend = test_backend(&config);
    let (signal, mut receiver) = tokio::sync::mpsc::channel(4);
    let app = test_app(backend).with_write_signal(signal);

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
        .handle(AppRequest::ProjectModify(ProjectModifyInput {
            id: "missing".to_string(),
            color: Patch::Missing,
        }))
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

struct TestShareGateway;

#[async_trait]
impl ShareGateway for TestShareGateway {
    async fn share(&self, _resource: ShareResource, id: &str) -> Result<String, ServiceError> {
        Ok(format!("https://share.example/{id}"))
    }

    async fn unshare(&self, _resource: ShareResource, _id: &str) -> Result<(), ServiceError> {
        Ok(())
    }
}

fn app_with_creator(db: Arc<dyn NoteDb>, creator: Arc<dyn NoteCreator>) -> Application {
    Application::new(db, creator, Arc::new(TestShareGateway))
}

fn test_app(db: Arc<dyn NoteDb>) -> Application {
    app_with_creator(db, Arc::new(DetachedCreator))
}

#[tokio::test]
async fn app_preserves_created_identity_when_editor_or_attachment_summary_fails() {
    let directory = tempfile::tempdir().unwrap();
    let config = test_config(directory.path());
    let attachment = directory.path().join("report.pdf");
    std::fs::write(&attachment, b"pdf").unwrap();
    let backend = test_backend(&config);
    let app = app_with_creator(backend, Arc::new(DetachedCreator));

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
    let backend = test_backend(&config);
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
    let app = test_app(backend.clone());

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
async fn app_owns_project_and_catalog_domain_operations() {
    let directory = tempfile::tempdir().unwrap();
    let config = test_config(directory.path());
    let backend = test_backend(&config);
    let app = test_app(backend);

    let project = app
        .handle(AppRequest::ProjectAdd(ProjectAddInput {
            name: "work".to_string(),
            color: Some("#123456".to_string()),
        }))
        .await
        .unwrap();
    let AppResponse::Project(project) = project else {
        panic!("unexpected project response")
    };
    assert_eq!(project.name, "work");
    assert_eq!(project.color.as_deref(), Some("#123456"));

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
    let backend = test_backend(&config);
    let app = Arc::new(test_app(backend));
    let listener =
        tokio::net::UnixListener::bind(flicknote_sync::ipc::socket_path(&config)).unwrap();
    let server = tokio::spawn(serve_app_once(listener, app, ServerInfo::current()));

    let client = DaemonClient::new(&config);
    let info = client.health().await.unwrap();
    assert_eq!(info.protocol, flicknote_sync::ipc::PROTOCOL_VERSION);
    server.await.unwrap().unwrap();

    std::fs::remove_file(flicknote_sync::ipc::socket_path(&config)).unwrap();
    let listener =
        tokio::net::UnixListener::bind(flicknote_sync::ipc::socket_path(&config)).unwrap();
    let directory2 = tempfile::tempdir().unwrap();
    let config2 = test_config(directory2.path());
    let backend = test_backend(&config2);
    let app = Arc::new(test_app(backend));
    let server = tokio::spawn(serve_app_once(listener, app, ServerInfo::current()));
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
async fn protocol_v1_app_request_is_rejected_before_application_dispatch() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let directory = tempfile::tempdir().unwrap();
    let config = test_config(directory.path());
    let backend = test_backend(&config);
    let (signal, mut receiver) = tokio::sync::mpsc::channel(1);
    let app = Arc::new(test_app(backend).with_write_signal(signal));
    let listener = tokio::net::UnixListener::bind(socket_path(&config)).unwrap();
    let server = tokio::spawn(serve_app_once(listener, app, ServerInfo::current()));

    let mut stream = tokio::net::UnixStream::connect(socket_path(&config))
        .await
        .unwrap();
    let request = DaemonRequest::App {
        protocol: 1,
        request: Box::new(AppRequest::ProjectArchive {
            id: "project-1".to_string(),
        }),
    };
    stream
        .write_all(&serde_json::to_vec(&request).unwrap())
        .await
        .unwrap();
    stream.shutdown().await.unwrap();
    let mut response = Vec::new();
    stream.read_to_end(&mut response).await.unwrap();
    let response: DaemonResponse = serde_json::from_slice(&response).unwrap();

    let DaemonResponse::AppError(error) = response else {
        panic!("expected protocol mismatch")
    };
    assert_eq!(error.code, "daemon_protocol_mismatch");
    assert!(receiver.try_recv().is_err());
    server.await.unwrap().unwrap();
}

#[tokio::test]
async fn local_app_owns_attachment_normalization_and_creator_call() {
    let directory = tempfile::tempdir().unwrap();
    let config = test_config(directory.path());
    let path = directory.path().join("report.pdf");
    std::fs::write(&path, b"pdf").unwrap();
    let backend = test_backend(&config);
    let creator = Arc::new(RecordingCreator {
        db: backend.clone(),
        request: std::sync::Mutex::new(None),
    });
    let app = app_with_creator(backend, creator.clone());

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
    let backend = test_backend(&config);
    let creator = Arc::new(RecordingCreator {
        db: backend.clone(),
        request: std::sync::Mutex::new(None),
    });
    let app = app_with_creator(backend.clone(), creator);

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
