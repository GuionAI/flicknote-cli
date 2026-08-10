use crate::backend::{InsertNoteReq, LocalPowerSyncBackend, NoteDb};
use std::ops::Deref;

pub(crate) struct BackendFixture {
    backend: LocalPowerSyncBackend,
    _database: powersync::PowerSyncDatabase,
    _directory: tempfile::TempDir,
}

impl Deref for BackendFixture {
    type Target = LocalPowerSyncBackend;

    fn deref(&self) -> &Self::Target {
        &self.backend
    }
}

pub(crate) async fn make_backend() -> BackendFixture {
    struct NoHttp;

    #[async_trait::async_trait]
    impl powersync::http::HttpClient for NoHttp {
        async fn send(
            &self,
            _request: powersync::http::Request,
        ) -> Result<powersync::http::Response, powersync::error::PowerSyncError> {
            panic!("local service tests must not make HTTP requests")
        }
    }

    use powersync::{ConnectionPool, PowerSyncDatabase, env::PowerSyncEnvironment};

    PowerSyncEnvironment::powersync_auto_extension().unwrap();
    let directory = tempfile::tempdir().unwrap();
    let pool = ConnectionPool::open(directory.path().join("test.db")).unwrap();
    let environment =
        PowerSyncEnvironment::custom(NoHttp, pool, PowerSyncEnvironment::tokio_timer());
    let db = PowerSyncDatabase::new(environment, crate::schema::app_schema());
    db.writer().await.unwrap();
    let backend = LocalPowerSyncBackend::new(db.clone(), "test-user-id".to_string());
    BackendFixture {
        backend,
        _database: db,
        _directory: directory,
    }
}

pub(crate) async fn insert_normal_note(
    backend: &LocalPowerSyncBackend,
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
