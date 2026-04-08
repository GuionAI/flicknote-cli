//! Postgres wire-native row types.
//!
//! These structs mirror the actual postgres column types, giving the compiler
//! full type-checked visibility into every column.  No string aliases,
//! no cast helpers, no forgotten mappings.

use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::error::CliError;

// ─── Note ────────────────────────────────────────────────────────────────────

pub(super) struct NotePgRow {
    pub id: Uuid,
    pub user_id: Uuid,
    pub r#type: String,
    pub status: String,
    pub title: Option<String>,
    pub content: Option<String>,
    pub summary: Option<String>,
    pub is_flagged: Option<bool>,
    pub project_id: Option<Uuid>,
    pub metadata: Option<serde_json::Value>,
    pub source: Option<serde_json::Value>,
    pub external_id: Option<serde_json::Value>,
    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
    pub deleted_at: Option<DateTime<Utc>>,
}

impl NotePgRow {
    pub(super) fn from_pg_row(row: &postgres::Row) -> Result<Self, CliError> {
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
                .try_get("is_flagged")
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
}

// ─── Project ──────────────────────────────────────────────────────────────────

pub(super) struct ProjectPgRow {
    pub id: Uuid,
    pub user_id: Uuid,
    pub name: String,
    pub color: Option<String>,
    pub prompt_id: Option<Uuid>,
    pub keyterm_id: Option<Uuid>,
    pub is_archived: Option<bool>,
    pub created_at: Option<DateTime<Utc>>,
}

impl ProjectPgRow {
    pub(super) fn from_pg_row(row: &postgres::Row) -> Result<Self, CliError> {
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
                .try_get("is_archived")
                .map_err(|e| CliError::Database(e.to_string()))?,
            created_at: row
                .try_get("created_at")
                .map_err(|e| CliError::Database(e.to_string()))?,
        })
    }
}

// ─── Prompt ───────────────────────────────────────────────────────────────────

pub(super) struct PromptPgRow {
    pub id: Uuid,
    pub user_id: Uuid,
    pub title: String,
    pub description: Option<String>,
    pub prompt: String,
    pub created_at: Option<DateTime<Utc>>,
}

impl PromptPgRow {
    pub(super) fn from_pg_row(row: &postgres::Row) -> Result<Self, CliError> {
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

// ─── Keyterm ─────────────────────────────────────────────────────────────────

pub(super) struct KeytermPgRow {
    pub id: Uuid,
    pub user_id: Uuid,
    pub name: String,
    pub description: Option<String>,
    pub content: Option<String>,
    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
}

impl KeytermPgRow {
    pub(super) fn from_pg_row(row: &postgres::Row) -> Result<Self, CliError> {
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
