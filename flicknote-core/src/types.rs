use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Closed production lifecycle set accepted by public note-status filters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum NoteStatus {
    Draft,
    AiQueued,
    SourceQueued,
    Ready,
}

impl NoteStatus {
    pub const ALL: [Self; 4] = [Self::Draft, Self::AiQueued, Self::SourceQueued, Self::Ready];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Draft => "draft",
            Self::AiQueued => "ai_queued",
            Self::SourceQueued => "source_queued",
            Self::Ready => "ready",
        }
    }
}

impl std::fmt::Display for NoteStatus {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl std::str::FromStr for NoteStatus {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .into_iter()
            .find(|status| status.as_str() == value)
            .ok_or_else(|| format!("unknown note status: {value}"))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Note {
    pub id: String,
    pub short_id: Option<i64>,
    pub user_id: String,
    pub r#type: String,
    pub status: String,
    pub title: Option<String>,
    pub content: Option<String>,
    pub summary: Option<String>,
    pub is_flagged: Option<i64>,
    pub project_id: Option<String>,
    pub metadata: Option<String>,
    pub source: Option<String>,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
    pub deleted_at: Option<String>,
}

impl Note {
    pub fn link_url(&self) -> Option<String> {
        let meta = self.metadata.as_ref()?;
        let v: serde_json::Value = serde_json::from_str(meta).ok()?;
        v.get("link")?
            .get("url")?
            .as_str()
            .map(std::string::ToString::to_string)
    }
}

#[cfg(test)]
mod tests {
    use super::NoteStatus;

    #[test]
    fn canonical_note_statuses_round_trip_and_reject_unknown_values() {
        let values = ["draft", "ai_queued", "source_queued", "ready"];
        assert_eq!(NoteStatus::ALL.map(NoteStatus::as_str), values);
        for value in values {
            let status = value.parse::<NoteStatus>().unwrap();
            assert_eq!(status.as_str(), value);
        }
        assert!("synced".parse::<NoteStatus>().is_err());
        assert!("reday".parse::<NoteStatus>().is_err());
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    pub id: String,
    pub user_id: String,
    pub name: String,
    pub color: Option<String>,
    pub metadata: Option<String>,
    pub is_archived: Option<i64>,
    pub created_at: Option<String>,
}
