#![allow(clippy::print_stderr)] // debug-only diagnostics

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

// ─── Helpers ──────────────────────────────────────────────────────────────────

/// Gate SQL + column metadata logging behind FN_DEBUG_SQL=1.
fn debug_sql_enabled() -> bool {
    std::env::var("FN_DEBUG_SQL").ok().as_deref() == Some("1")
}

/// Read a column from a postgres row, naming the column in the error message.
macro_rules! try_get_col {
    ($row:expr, $name:literal, $t:ty) => {
        $row.try_get::<_, $t>($name)
            .map_err(|e| CliError::Database(format!("decode(col={}): {e}", $name)))?
    };
}

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
        if debug_sql_enabled() {
            for (i, col) in row.columns().iter().enumerate() {
                eprintln!(
                    "[fn-sql] col[{i}] name={} type={:?}",
                    col.name(),
                    col.type_()
                );
            }
        }
        Ok(Self {
            id: try_get_col!(row, "id", Uuid),
            user_id: try_get_col!(row, "user_id", Uuid),
            r#type: try_get_col!(row, "type", String),
            status: try_get_col!(row, "status", String),
            title: try_get_col!(row, "title", Option<String>),
            content: try_get_col!(row, "content", Option<String>),
            summary: try_get_col!(row, "summary", Option<String>),
            is_flagged: try_get_col!(row, "is_flagged", Option<bool>),
            project_id: try_get_col!(row, "project_id", Option<Uuid>),
            metadata: try_get_col!(row, "metadata", Option<serde_json::Value>),
            source: try_get_col!(row, "source", Option<serde_json::Value>),
            external_id: try_get_col!(row, "external_id", Option<serde_json::Value>),
            created_at: try_get_col!(row, "created_at", Option<DateTime<Utc>>),
            updated_at: try_get_col!(row, "updated_at", Option<DateTime<Utc>>),
            deleted_at: try_get_col!(row, "deleted_at", Option<DateTime<Utc>>),
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
            id: try_get_col!(row, "id", Uuid),
            user_id: try_get_col!(row, "user_id", Uuid),
            name: try_get_col!(row, "name", String),
            color: try_get_col!(row, "color", Option<String>),
            prompt_id: try_get_col!(row, "prompt_id", Option<Uuid>),
            keyterm_id: try_get_col!(row, "keyterm_id", Option<Uuid>),
            is_archived: try_get_col!(row, "is_archived", Option<bool>),
            created_at: try_get_col!(row, "created_at", Option<DateTime<Utc>>),
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
            id: try_get_col!(row, "id", Uuid),
            user_id: try_get_col!(row, "user_id", Uuid),
            title: try_get_col!(row, "title", String),
            description: try_get_col!(row, "description", Option<String>),
            prompt: try_get_col!(row, "prompt", String),
            created_at: try_get_col!(row, "created_at", Option<DateTime<Utc>>),
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
            id: try_get_col!(row, "id", Uuid),
            user_id: try_get_col!(row, "user_id", Uuid),
            name: try_get_col!(row, "name", String),
            description: try_get_col!(row, "description", Option<String>),
            content: try_get_col!(row, "content", Option<String>),
            created_at: try_get_col!(row, "created_at", Option<DateTime<Utc>>),
            updated_at: try_get_col!(row, "updated_at", Option<DateTime<Utc>>),
        })
    }
}
