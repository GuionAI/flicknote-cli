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

pub const PROJECTS: [(&str, &str, &str); 3] = [
    ("fixture-ideas", "Ideas", "05C7F7"),
    ("fixture-reading", "Reading", "A78BFA"),
    ("fixture-work", "Workspace", "F59E0B"),
];

pub const USER: &str = "embedded-gpui-synthetic-user";

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
        for (key, name, color) in PROJECTS {
            tx.execute("INSERT OR IGNORE INTO projects (id, user_id, name, color, is_archived, created_at) VALUES (?, ?, ?, ?, 0, ?)", params![key, USER, name, color, now]).map_err(|e| e.to_string())?;
        }
        for id in 1..=count {
            let (note_type, content) = match id % 5 {
                0 => ("normal", "A quiet place to collect ideas".to_string()),
                1 => ("normal", "今天的想法：把复杂的事情写清楚。\n下一步，做一个小实验。".to_string()),
                2 => ("link", "https://example.com/synthetic-reading".to_string()),
                3 => ("normal", "Review the workspace spacing and typography with a long line that reaches the edge of the timeline and truncates cleanly.".repeat(3)),
                _ => ("normal", format!("Planning note {id}\nFirst, write down the idea.\nThen choose one useful next step.")),
            };
            let project = PROJECTS[(id as usize) % PROJECTS.len()].0;
            tx.execute("INSERT INTO notes (id, short_id, user_id, type, status, title, content, project_id, is_flagged, created_at, updated_at) VALUES (?, ?, ?, ?, 'ready', ?, ?, ?, 0, ?, ?)", params![format!("fixture-{id}"), id, USER, note_type, format!("Synthetic note {id}"), content, project, now, now]).map_err(|e| e.to_string())?;
        }
    }
    tx.commit().map_err(|e| e.to_string())
}
