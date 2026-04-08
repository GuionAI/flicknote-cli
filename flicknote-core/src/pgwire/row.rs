//! Postgres wire-native row types.
//!
//! These structs mirror the actual postgres column types, giving the compiler
//! full type-checked visibility into every column.  No string aliases,
//! no cast helpers, no forgotten mappings.
//!
//! ## Why explicit `try_get::<_, T>` type hints?
//!
//! `postgres::Row::try_get<I, T>(&self, idx: I) -> Result<T, _>` requires `T` to
//! implement `FromSql`.  When called without a turbofish — `try_get("col_name")` —
//! the compiler must infer `T` from context.  In a struct field assignment like:
//!
//! ```ignore
//! id: row.try_get("id")?
//! ```
//!
//! the assignment target (`Uuid`) *should* determine `T`, but the postgres crate's
//! inference doesn't propagate it backwards through the method chain reliably.
//! Without hints the compiler defaults `T` to `bool` (the first `FromSql` impl in
//! scope), producing type mismatches on every non-bool column.  The explicit
//! `::<_, Uuid>` annotation pins `T` before the call resolves, making
//! deserialization deterministic regardless of the column's reported type OID.
//!
//! String/text columns (`String`, `Option<String>`) don't need hints because
//! `FromSql<&str>` accepts any text-like type, so inference works implicitly.

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
            id: row.try_get::<_, Uuid>("id")?,
            user_id: row.try_get::<_, Uuid>("user_id")?,
            r#type: row.try_get::<_, String>("type")?,
            status: row.try_get::<_, String>("status")?,
            title: row.try_get::<_, Option<String>>("title")?,
            content: row.try_get::<_, Option<String>>("content")?,
            summary: row.try_get::<_, Option<String>>("summary")?,
            is_flagged: row.try_get::<_, Option<bool>>("is_flagged")?,
            project_id: row.try_get::<_, Option<Uuid>>("project_id")?,
            metadata: row.try_get::<_, Option<serde_json::Value>>("metadata")?,
            source: row.try_get::<_, Option<serde_json::Value>>("source")?,
            external_id: row.try_get::<_, Option<serde_json::Value>>("external_id")?,
            created_at: row.try_get::<_, Option<DateTime<Utc>>>("created_at")?,
            updated_at: row.try_get::<_, Option<DateTime<Utc>>>("updated_at")?,
            deleted_at: row.try_get::<_, Option<DateTime<Utc>>>("deleted_at")?,
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
            id: row.try_get::<_, Uuid>("id")?,
            user_id: row.try_get::<_, Uuid>("user_id")?,
            name: row.try_get::<_, String>("name")?,
            color: row.try_get::<_, Option<String>>("color")?,
            prompt_id: row.try_get::<_, Option<Uuid>>("prompt_id")?,
            keyterm_id: row.try_get::<_, Option<Uuid>>("keyterm_id")?,
            is_archived: row.try_get::<_, Option<bool>>("is_archived")?,
            created_at: row.try_get::<_, Option<DateTime<Utc>>>("created_at")?,
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
            id: row.try_get::<_, Uuid>("id")?,
            user_id: row.try_get::<_, Uuid>("user_id")?,
            title: row.try_get::<_, String>("title")?,
            description: row.try_get::<_, Option<String>>("description")?,
            prompt: row.try_get::<_, String>("prompt")?,
            created_at: row.try_get::<_, Option<DateTime<Utc>>>("created_at")?,
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
            id: row.try_get::<_, Uuid>("id")?,
            user_id: row.try_get::<_, Uuid>("user_id")?,
            name: row.try_get::<_, String>("name")?,
            description: row.try_get::<_, Option<String>>("description")?,
            content: row.try_get::<_, Option<String>>("content")?,
            created_at: row.try_get::<_, Option<DateTime<Utc>>>("created_at")?,
            updated_at: row.try_get::<_, Option<DateTime<Utc>>>("updated_at")?,
        })
    }
}
