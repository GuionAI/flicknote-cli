//! Synthetic creation at the existing port; never used by the production host.
use async_trait::async_trait;
use flicknote_core::backend::InsertedNote;
use flicknote_core::services::{
    error::ServiceError,
    ports::{CreateNote, CreatedNote, NoteCreator},
};
use powersync::PowerSyncDatabase;
use rusqlite::params;
use std::time::Duration;

pub(super) const USER: &str = "embedded-gpui-synthetic-user";

pub(super) struct FixtureCreator {
    pub db: PowerSyncDatabase,
    pub delay: Duration,
}

#[async_trait]
impl NoteCreator for FixtureCreator {
    async fn create(&self, request: CreateNote) -> Result<CreatedNote, ServiceError> {
        tokio::time::sleep(self.delay).await;
        if request.content.as_deref() == Some("[fixture-fail]") {
            return Err(ServiceError::Internal(
                "Synthetic creation failure (no write)".into(),
            ));
        }
        if request.attachment_path.is_some() || !request.topics.is_empty() {
            return Err(ServiceError::InvalidArgument(
                "Spike supports text fixtures without attachments or topics".into(),
            ));
        }
        let id = insert(&self.db, &request)
            .await
            .map_err(ServiceError::Internal)?;
        Ok(CreatedNote {
            inserted: InsertedNote {
                uuid: request.id,
                short_id: Some(id),
            },
            confirmed_extraction_ids: vec![],
        })
    }
}

async fn insert(db: &PowerSyncDatabase, request: &CreateNote) -> Result<i64, String> {
    let mut writer = db.writer().await.map_err(|e| e.to_string())?;
    let tx = writer.transaction().map_err(|e| e.to_string())?;
    let id: i64 = tx
        .query_row(
            "SELECT coalesce(max(short_id), 0) + 1 FROM notes WHERE user_id = ?",
            [USER],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    tx.execute("INSERT INTO notes (id, short_id, user_id, type, status, title, content, metadata, project_id, is_flagged, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, 0, ?, ?)",
        params![request.id, id, USER, request.note_type, request.status, request.title, request.content, request.metadata, request.project_id, request.now, request.now]).map_err(|e| e.to_string())?;
    tx.commit().map_err(|e| e.to_string())?;
    Ok(id)
}

pub(super) async fn seed(db: &PowerSyncDatabase, count: u32) -> Result<(), String> {
    let mut writer = db.writer().await.map_err(|e| e.to_string())?;
    let tx = writer.transaction().map_err(|e| e.to_string())?;
    let existing: i64 = tx
        .query_row("SELECT count(*) FROM notes", [], |r| r.get(0))
        .map_err(|e| e.to_string())?;
    if existing == 0 {
        let now = chrono::Utc::now().to_rfc3339();
        for id in 1..=count {
            tx.execute("INSERT INTO notes (id, short_id, user_id, type, status, title, content, is_flagged, created_at, updated_at) VALUES (?, ?, ?, 'normal', 'ready', ?, ?, 0, ?, ?)", params![format!("fixture-{id}"), id, USER, format!("Synthetic note {id}"), format!("Synthetic note {id}\nPlain detail — 合成笔记. This is fixture-owned content."), now, now]).map_err(|e| e.to_string())?;
        }
    }
    tx.commit().map_err(|e| e.to_string())
}
