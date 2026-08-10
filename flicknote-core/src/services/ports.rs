//! Narrow ports for network and operating-system side effects.

use async_trait::async_trait;

use crate::backend::{InsertNoteReq, InsertedNote};

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
    async fn create(&self, request: CreateNote) -> Result<CreatedNote, ServiceError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreatedNote {
    pub inserted: InsertedNote,
    pub confirmed_extraction_ids: Vec<String>,
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

pub trait BrowserOpener: Send + Sync {
    fn open(&self, url: &str) -> Result<(), ServiceError>;
}

#[cfg(test)]
mod tests {
    use super::BrowserOpener;

    fn assert_send_sync<T: Send + Sync + ?Sized>() {}

    #[test]
    fn browser_opener_port_is_send_and_sync() {
        assert_send_sync::<dyn BrowserOpener>();
    }
}
