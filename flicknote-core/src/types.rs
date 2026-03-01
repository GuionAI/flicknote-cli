use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Note {
    pub id: String,
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
    pub external_id: Option<String>,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
    pub deleted_at: Option<String>,
}

impl Note {
    pub fn from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get("id")?,
            user_id: row.get("user_id")?,
            r#type: row.get("type")?,
            status: row.get("status")?,
            title: row.get("title")?,
            content: row.get("content")?,
            summary: row.get("summary")?,
            is_flagged: row.get("is_flagged")?,
            project_id: row.get("project_id")?,
            metadata: row.get("metadata")?,
            source: row.get("source")?,
            external_id: row.get("external_id")?,
            created_at: row.get("created_at")?,
            updated_at: row.get("updated_at")?,
            deleted_at: row.get("deleted_at")?,
        })
    }

    pub fn link_url(&self) -> Option<String> {
        let meta = self.metadata.as_ref()?;
        let v: serde_json::Value = serde_json::from_str(meta).ok()?;
        v.get("link")?
            .get("url")?
            .as_str()
            .map(std::string::ToString::to_string)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    pub id: String,
    pub user_id: String,
    pub name: String,
    pub color: Option<String>,
    pub is_archived: Option<i64>,
    pub created_at: Option<String>,
}

impl Project {
    pub fn from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get("id")?,
            user_id: row.get("user_id")?,
            name: row.get("name")?,
            color: row.get("color")?,
            is_archived: row.get("is_archived")?,
            created_at: row.get("created_at")?,
        })
    }
}
