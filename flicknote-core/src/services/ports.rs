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

#[async_trait(?Send)]
pub trait NoteCreator {
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

#[async_trait(?Send)]
impl NoteCreator for DirectNoteCreator<'_> {
    async fn create(&self, request: CreateNote) -> Result<InsertedNote, ServiceError> {
        Ok(self.db.insert_note(&request.as_insert_request()).await?)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShareResource {
    Note,
    Project,
}

#[async_trait(?Send)]
pub trait ShareGateway {
    async fn share(&self, resource: ShareResource, id: &str) -> Result<String, ServiceError>;
    async fn unshare(&self, resource: ShareResource, id: &str) -> Result<(), ServiceError>;
}

pub trait BrowserOpener {
    fn open(&self, url: &str) -> Result<(), ServiceError>;
}
