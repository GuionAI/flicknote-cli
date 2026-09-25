use std::path::PathBuf;

use async_trait::async_trait;
use flicknote_core::services::error::ServiceError;
use flicknote_core::services::ports::{ProjectAssignmentEvent, ProjectAssignmentEventSink};
use tokio::io::AsyncWriteExt;
use tokio::sync::Mutex;

pub(crate) struct JsonlProjectAssignmentEventSink {
    path: PathBuf,
    append_lock: Mutex<()>,
}

impl JsonlProjectAssignmentEventSink {
    pub(crate) fn new(path: PathBuf) -> Self {
        Self {
            path,
            append_lock: Mutex::new(()),
        }
    }
}

#[async_trait]
impl ProjectAssignmentEventSink for JsonlProjectAssignmentEventSink {
    async fn append(&self, events: &[ProjectAssignmentEvent]) -> Result<(), ServiceError> {
        if events.is_empty() {
            return Ok(());
        }
        let mut payload = Vec::new();
        for event in events {
            serde_json::to_writer(&mut payload, event)
                .map_err(|error| ServiceError::Internal(error.to_string()))?;
            payload.push(b'\n');
        }

        let _guard = self.append_lock.lock().await;
        let parent = self.path.parent().ok_or_else(|| {
            ServiceError::Internal("project assignment event path has no parent".to_string())
        })?;
        tokio::fs::create_dir_all(parent).await?;
        let mut file = tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .await?;
        file.write_all(&payload).await?;
        file.flush().await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use flicknote_core::services::ports::{
        ProjectAssignmentEvent, ProjectAssignmentEventSink, ProjectAssignmentSource,
    };

    use super::JsonlProjectAssignmentEventSink;

    #[tokio::test]
    async fn lazily_appends_one_json_object_per_line() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("events/project_assignment.jsonl");
        let sink = JsonlProjectAssignmentEventSink::new(path.clone());
        let events = [ProjectAssignmentEvent {
            v: 1,
            event: "project_assignment",
            note_id: "note-1".to_string(),
            from_project_id: None,
            to_project_id: Some("project-1".to_string()),
            source: ProjectAssignmentSource::Manual,
            probability: None,
            created_at: "2026-09-25T00:00:00Z".to_string(),
        }];

        sink.append(&events).await.unwrap();
        sink.append(&events).await.unwrap();

        let contents = tokio::fs::read_to_string(path).await.unwrap();
        let lines = contents.lines().collect::<Vec<_>>();
        assert_eq!(lines.len(), 2);
        let value: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(value["v"], 1);
        assert_eq!(value["event"], "project_assignment");
        assert_eq!(value["source"], "manual");
        assert_eq!(value["probability"], serde_json::Value::Null);
    }
}
