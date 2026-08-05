use crate::backend::{InsertNoteReq, NoteDb, SqliteBackend};
use crate::config::{Config, ConfigPaths};
use crate::db::Database;

pub(crate) async fn make_backend() -> SqliteBackend {
    let directory = tempfile::tempdir().unwrap();
    let config = Config {
        supabase_url: String::new(),
        supabase_anon_key: String::new(),
        powersync_url: String::new(),
        api_url: String::new(),
        web_url: None,
        paths: ConfigPaths {
            config_dir: directory.path().to_path_buf(),
            data_dir: directory.path().to_path_buf(),
            config_file: directory.path().join("config.json"),
            session_file: directory.path().join("session.json"),
            db_file: directory.path().join("test.db"),
            log_file: directory.path().join("test.log"),
        },
    };
    let db = Database::open_local(&config).await.unwrap();
    std::mem::forget(directory);
    SqliteBackend {
        db,
        user_id: "test-user-id".to_string(),
    }
}

pub(crate) async fn insert_normal_note(
    backend: &SqliteBackend,
    content: &str,
    status: &str,
) -> String {
    let id = uuid::Uuid::new_v4().to_string();
    backend
        .insert_note(&InsertNoteReq {
            id: &id,
            note_type: "normal",
            status,
            title: Some("Test note"),
            content: Some(content),
            metadata: None,
            project_id: None,
            now: "2026-08-05T00:00:00Z",
        })
        .await
        .unwrap();
    id
}
