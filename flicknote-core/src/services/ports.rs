//! Narrow ports for network and operating-system side effects.

use async_trait::async_trait;

use crate::backend::{InsertNoteReq, InsertedNote, NoteDb};

use super::error::ServiceError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateNote {
    pub id: String,
    pub note_type: String,
    pub status: String,
    pub title: Option<String>,
    pub content: Option<String>,
    pub metadata: Option<String>,
    pub project_id: Option<String>,
    pub now: String,
    pub topics: Vec<String>,
    pub attachment_path: Option<String>,
}

impl CreateNote {
    pub fn as_insert_request(&self) -> InsertNoteReq<'_> {
        InsertNoteReq {
            id: &self.id,
            note_type: &self.note_type,
            status: &self.status,
            title: self.title.as_deref(),
            content: self.content.as_deref(),
            metadata: self.metadata.as_deref(),
            project_id: self.project_id.as_deref(),
            now: &self.now,
        }
    }
}

#[async_trait]
pub trait NoteCreator: Send + Sync {
    async fn create(&self, request: CreateNote) -> Result<InsertedNote, ServiceError>;
}

pub struct DirectNoteCreator<'a> {
    db: &'a dyn NoteDb,
}

impl<'a> DirectNoteCreator<'a> {
    pub fn new(db: &'a dyn NoteDb) -> Self {
        Self { db }
    }
}

#[async_trait]
impl NoteCreator for DirectNoteCreator<'_> {
    async fn create(&self, request: CreateNote) -> Result<InsertedNote, ServiceError> {
        self.db
            .insert_note_with_extractions(
                &request.as_insert_request(),
                crate::TOPIC_EXTRACTION_KEY,
                &request.topics,
            )
            .await
            .map_err(ServiceError::from)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShareResource {
    Note,
    Project,
}

#[async_trait]
pub trait ShareGateway: Send + Sync {
    async fn share(&self, resource: ShareResource, id: &str) -> Result<String, ServiceError>;
    async fn unshare(&self, resource: ShareResource, id: &str) -> Result<(), ServiceError>;
}

pub trait BrowserOpener {
    fn open(&self, url: &str) -> Result<(), ServiceError>;
}

#[cfg(all(test, feature = "powersync"))]
mod tests {
    use super::*;
    use crate::backend::NoteDb;
    use crate::services::test_support::make_backend;

    #[tokio::test]
    async fn direct_create_rolls_back_note_when_topic_persistence_fails() {
        let backend = make_backend().await;
        sqlx::query(
            "CREATE TRIGGER reject_bad_topic INSTEAD OF INSERT ON note_extractions \
             WHEN NEW.value = 'fail' BEGIN SELECT RAISE(ABORT, 'topic failure'); END",
        )
        .execute(&backend.db.pool)
        .await
        .unwrap();
        let id = uuid::Uuid::new_v4().to_string();

        let error = DirectNoteCreator::new(&backend)
            .create(CreateNote {
                id: id.clone(),
                note_type: "normal".to_string(),
                status: "ai_queued".to_string(),
                title: Some("Title".to_string()),
                content: Some("Body".to_string()),
                metadata: None,
                project_id: None,
                now: chrono::Utc::now().to_rfc3339(),
                topics: vec!["fail".to_string()],
                attachment_path: None,
            })
            .await
            .unwrap_err();

        assert!(error.to_string().contains("topic failure"));
        assert!(backend.find_note(&id).await.is_err());
    }
}
