use serde::{Deserialize, Serialize};

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
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    pub id: String,
    pub user_id: String,
    pub name: String,
    pub color: Option<String>,
    pub is_archived: Option<i64>,
    pub created_at: Option<String>,
}
