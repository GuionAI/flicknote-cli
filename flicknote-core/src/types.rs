use serde::{Deserialize, Serialize};

#[cfg(feature = "storage-pgwire")]
use crate::error::CliError;

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
    #[cfg(feature = "powersync")]
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

    #[cfg(feature = "storage-pgwire")]
    pub fn from_pg_row(row: &postgres::Row) -> Result<Self, CliError> {
        Ok(Self {
            id: row
                .try_get("id")
                .map_err(|e| CliError::Database(e.to_string()))?,
            user_id: row
                .try_get("user_id")
                .map_err(|e| CliError::Database(e.to_string()))?,
            r#type: row
                .try_get("type")
                .map_err(|e| CliError::Database(e.to_string()))?,
            status: row
                .try_get("status")
                .map_err(|e| CliError::Database(e.to_string()))?,
            title: row
                .try_get("title")
                .map_err(|e| CliError::Database(e.to_string()))?,
            content: row
                .try_get("content")
                .map_err(|e| CliError::Database(e.to_string()))?,
            summary: row
                .try_get("summary")
                .map_err(|e| CliError::Database(e.to_string()))?,
            is_flagged: row
                .try_get::<_, Option<i64>>("is_flagged")
                .map_err(|e| CliError::Database(e.to_string()))?,
            project_id: row
                .try_get("project_id")
                .map_err(|e| CliError::Database(e.to_string()))?,
            metadata: row
                .try_get("metadata")
                .map_err(|e| CliError::Database(e.to_string()))?,
            source: row
                .try_get("source")
                .map_err(|e| CliError::Database(e.to_string()))?,
            external_id: row
                .try_get("external_id")
                .map_err(|e| CliError::Database(e.to_string()))?,
            created_at: row
                .try_get("created_at")
                .map_err(|e| CliError::Database(e.to_string()))?,
            updated_at: row
                .try_get("updated_at")
                .map_err(|e| CliError::Database(e.to_string()))?,
            deleted_at: row
                .try_get("deleted_at")
                .map_err(|e| CliError::Database(e.to_string()))?,
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
    pub prompt_id: Option<String>,
    pub keyterm_id: Option<String>,
    pub is_archived: Option<i64>,
    pub created_at: Option<String>,
}

impl Project {
    #[cfg(feature = "powersync")]
    pub fn from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get("id")?,
            user_id: row.get("user_id")?,
            name: row.get("name")?,
            color: row.get("color")?,
            prompt_id: row.get("prompt_id")?,
            keyterm_id: row.get("keyterm_id")?,
            is_archived: row.get("is_archived")?,
            created_at: row.get("created_at")?,
        })
    }

    #[cfg(feature = "storage-pgwire")]
    pub fn from_pg_row(row: &postgres::Row) -> Result<Self, CliError> {
        Ok(Self {
            id: row
                .try_get("id")
                .map_err(|e| CliError::Database(e.to_string()))?,
            user_id: row
                .try_get("user_id")
                .map_err(|e| CliError::Database(e.to_string()))?,
            name: row
                .try_get("name")
                .map_err(|e| CliError::Database(e.to_string()))?,
            color: row
                .try_get("color")
                .map_err(|e| CliError::Database(e.to_string()))?,
            prompt_id: row
                .try_get("prompt_id")
                .map_err(|e| CliError::Database(e.to_string()))?,
            keyterm_id: row
                .try_get("keyterm_id")
                .map_err(|e| CliError::Database(e.to_string()))?,
            is_archived: row
                .try_get::<_, Option<i64>>("is_archived")
                .map_err(|e| CliError::Database(e.to_string()))?,
            created_at: row
                .try_get("created_at")
                .map_err(|e| CliError::Database(e.to_string()))?,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Prompt {
    pub id: String,
    pub user_id: String,
    pub title: String,
    pub description: Option<String>,
    pub prompt: String,
    pub created_at: Option<String>,
}

#[cfg(feature = "powersync")]
impl Prompt {
    pub fn from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get("id")?,
            user_id: row.get("user_id")?,
            title: row.get("title")?,
            description: row.get("description")?,
            prompt: row.get("prompt")?,
            created_at: row.get("created_at")?,
        })
    }
}

#[cfg(feature = "storage-pgwire")]
impl Prompt {
    pub fn from_pg_row(row: &postgres::Row) -> Result<Self, CliError> {
        Ok(Self {
            id: row
                .try_get("id")
                .map_err(|e| CliError::Database(e.to_string()))?,
            user_id: row
                .try_get("user_id")
                .map_err(|e| CliError::Database(e.to_string()))?,
            title: row
                .try_get("title")
                .map_err(|e| CliError::Database(e.to_string()))?,
            description: row
                .try_get("description")
                .map_err(|e| CliError::Database(e.to_string()))?,
            prompt: row
                .try_get("prompt")
                .map_err(|e| CliError::Database(e.to_string()))?,
            created_at: row
                .try_get("created_at")
                .map_err(|e| CliError::Database(e.to_string()))?,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Keyterm {
    pub id: String,
    pub user_id: String,
    pub name: String,
    pub description: Option<String>,
    pub content: Option<String>,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
}

#[cfg(feature = "powersync")]
impl Keyterm {
    pub fn from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get("id")?,
            user_id: row.get("user_id")?,
            name: row.get("name")?,
            description: row.get("description")?,
            content: row.get("content")?,
            created_at: row.get("created_at")?,
            updated_at: row.get("updated_at")?,
        })
    }
}

#[cfg(feature = "storage-pgwire")]
impl Keyterm {
    pub fn from_pg_row(row: &postgres::Row) -> Result<Self, CliError> {
        Ok(Self {
            id: row
                .try_get("id")
                .map_err(|e| CliError::Database(e.to_string()))?,
            user_id: row
                .try_get("user_id")
                .map_err(|e| CliError::Database(e.to_string()))?,
            name: row
                .try_get("name")
                .map_err(|e| CliError::Database(e.to_string()))?,
            description: row
                .try_get("description")
                .map_err(|e| CliError::Database(e.to_string()))?,
            content: row
                .try_get("content")
                .map_err(|e| CliError::Database(e.to_string()))?,
            created_at: row
                .try_get("created_at")
                .map_err(|e| CliError::Database(e.to_string()))?,
            updated_at: row
                .try_get("updated_at")
                .map_err(|e| CliError::Database(e.to_string()))?,
        })
    }
}
