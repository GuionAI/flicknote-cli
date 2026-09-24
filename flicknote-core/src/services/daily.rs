use chrono::{DateTime, Datelike, Utc};
use chrono_tz::Tz;

use crate::backend::NoteDb;

use super::dto::{CaptureReceipt, DailyReceipt};
use super::error::ServiceError;
use super::ports::{CreateNote, NoteCreator};

const PENDING_ROUTE_CONTENT: &str =
    r#"{"kind":"project_route","destination":null,"probability":null}"#;

pub struct DailyService<'a> {
    db: &'a dyn NoteDb,
}

impl<'a> DailyService<'a> {
    pub fn new(db: &'a dyn NoteDb) -> Self {
        Self { db }
    }

    pub async fn get_or_create(
        &self,
        creator: &dyn NoteCreator,
    ) -> Result<DailyReceipt, ServiceError> {
        self.get_or_create_at(creator, Utc::now()).await
    }

    pub async fn get_or_create_at(
        &self,
        creator: &dyn NoteCreator,
        now: DateTime<Utc>,
    ) -> Result<DailyReceipt, ServiceError> {
        let tz_name = self
            .db
            .configured_iana_tz()
            .await?
            .unwrap_or_else(|| "UTC".to_string());
        let timezone: Tz = tz_name.parse().map_err(|_| {
            ServiceError::InvalidArgument(format!("invalid configured IANA timezone: {tz_name}"))
        })?;
        let local = now.with_timezone(&timezone);
        let date = local.format("%Y-%m-%d").to_string();
        if let Some(note) = self.db.find_daily(&date).await? {
            return Ok(receipt(note, date));
        }

        let title = format!(
            "{}, {} {}",
            local.format("%A"),
            local.format("%b"),
            local.day()
        );
        let id = uuid::Uuid::new_v4().to_string();
        let created = creator
            .create(CreateNote {
                id,
                note_type: "normal".to_string(),
                status: "ready".to_string(),
                title: Some(title),
                content: Some(String::new()),
                metadata: Some(serde_json::json!({ "daily": { "date": date } }).to_string()),
                project_id: None,
                now: now.to_rfc3339(),
                topics: Vec::new(),
                attachment_path: None,
            })
            .await?;
        let note = self.db.find_note(&created.inserted.uuid).await?;
        Ok(receipt(note, date))
    }

    pub async fn capture(
        &self,
        creator: &dyn NoteCreator,
        text: &str,
    ) -> Result<CaptureReceipt, ServiceError> {
        if text.trim().is_empty() {
            return Err(ServiceError::InvalidArgument(
                "capture text must not be empty".to_string(),
            ));
        }
        let daily = self.get_or_create(creator).await?;
        let routing_comment_uuid = uuid::Uuid::new_v4().to_string();
        self.db
            .capture_into_daily(
                &daily.uuid,
                text,
                &routing_comment_uuid,
                PENDING_ROUTE_CONTENT,
            )
            .await?;
        Ok(CaptureReceipt {
            daily_uuid: daily.uuid,
            daily_short_id: daily.short_id,
            routing_comment_uuid,
        })
    }
}

fn receipt(note: crate::types::Note, date: String) -> DailyReceipt {
    DailyReceipt {
        uuid: note.id,
        short_id: note.short_id,
        date,
        title: note.title.unwrap_or_default(),
    }
}

#[cfg(all(test, feature = "powersync"))]
mod tests {
    use async_trait::async_trait;
    use chrono::TimeZone;
    use rusqlite::params;

    use crate::backend::NoteDb;
    use crate::services::ports::{CreatedNote, NoteCreator};
    use crate::services::test_support::make_backend;

    use super::*;

    struct DbCreator<'a>(&'a dyn NoteDb);

    #[async_trait]
    impl NoteCreator for DbCreator<'_> {
        async fn create(&self, request: CreateNote) -> Result<CreatedNote, ServiceError> {
            let inserted = self.0.insert_note(&request.as_insert_request()).await?;
            Ok(CreatedNote {
                inserted,
                confirmed_extraction_ids: Vec::new(),
            })
        }
    }

    #[tokio::test]
    async fn daily_uses_configured_local_date_and_is_idempotent() {
        let backend = make_backend().await;
        {
            let writer = backend.database().writer().await.unwrap();
            writer
                .execute(
                    "INSERT INTO settings (id, iana_tz) VALUES (?, ?)",
                    params!["settings-1", "Asia/Taipei"],
                )
                .unwrap();
        }
        let service = DailyService::new(&*backend);
        let creator = DbCreator(&*backend);
        let instant = Utc.with_ymd_and_hms(2026, 9, 24, 16, 30, 0).unwrap();

        let first = service.get_or_create_at(&creator, instant).await.unwrap();
        let second = service.get_or_create_at(&creator, instant).await.unwrap();

        assert_eq!(first.uuid, second.uuid);
        assert_eq!(first.date, "2026-09-25");
        assert_eq!(first.title, "Friday, Sep 25");
        let note = backend.find_note(&first.uuid).await.unwrap();
        assert_eq!(note.r#type, "normal");
        assert_eq!(note.project_id, None);
        assert_eq!(
            note.metadata.as_deref(),
            Some(r#"{"daily":{"date":"2026-09-25"}}"#)
        );
    }

    #[tokio::test]
    async fn capture_keeps_multiline_and_duplicate_submissions_as_distinct_comments() {
        let backend = make_backend().await;
        let service = DailyService::new(&*backend);
        let creator = DbCreator(&*backend);
        let text = "pricing:\n\n500 basic\n900 VAT";

        let first = service.capture(&creator, text).await.unwrap();
        let second = service.capture(&creator, text).await.unwrap();

        assert_eq!(first.daily_uuid, second.daily_uuid);
        assert_ne!(first.routing_comment_uuid, second.routing_comment_uuid);
        let comments = backend.list_comments(&first.daily_uuid).await.unwrap();
        assert_eq!(comments.len(), 2);
        assert!(comments.iter().all(|comment| comment.block_text == text));
        assert_eq!(
            backend
                .find_note_content(&first.daily_uuid)
                .await
                .unwrap()
                .as_deref(),
            Some("pricing:\n\n500 basic\n900 VAT\n\npricing:\n\n500 basic\n900 VAT")
        );
    }

    #[tokio::test]
    async fn blank_capture_is_rejected_before_daily_creation() {
        let backend = make_backend().await;
        let service = DailyService::new(&*backend);
        let creator = DbCreator(&*backend);

        let error = service.capture(&creator, " \n ").await.unwrap_err();
        assert_eq!(error.code(), "invalid_argument");
        assert!(
            backend
                .find_daily(&Utc::now().format("%Y-%m-%d").to_string())
                .await
                .unwrap()
                .is_none()
        );
    }
}
