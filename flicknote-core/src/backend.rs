use async_trait::async_trait;
#[cfg(feature = "powersync")]
use sqlx::SqlitePool;

#[cfg(feature = "powersync")]
use crate::db::Database;
use crate::error::CliError;
use crate::types::{Keyterm, Note, Project, Prompt};

// ─── Filter / request types ──────────────────────────────────────────────────

pub struct NoteFilter<'a> {
    pub project_id: Option<&'a str>,
    pub note_type: Option<&'a str>,
    pub archived: bool,
    pub limit: u32,
}

pub struct InsertNoteReq<'a> {
    pub id: &'a str,
    pub note_type: &'a str,
    pub status: &'a str,
    pub title: Option<&'a str>,
    pub content: Option<&'a str>,
    pub metadata: Option<&'a str>,
    pub project_id: Option<&'a str>,
    pub now: &'a str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InsertedNote {
    pub uuid: String,
    pub short_id: Option<i64>,
}

pub(crate) enum NoteLookup<'a> {
    ShortId(i64),
    Uuid(&'a str),
}

// ─── Shared helpers ──────────────────────────────────────────────────────────

pub(crate) fn parse_note_lookup(input: &str) -> Result<NoteLookup<'_>, CliError> {
    if input.chars().all(|c| c.is_ascii_digit()) {
        let short_id = input.parse::<i64>().map_err(|_| CliError::NoteNotFound {
            id: input.to_string(),
        })?;
        return Ok(NoteLookup::ShortId(short_id));
    }
    if uuid::Uuid::parse_str(input).is_ok() {
        return Ok(NoteLookup::Uuid(input));
    }
    Err(CliError::NoteNotFound {
        id: input.to_string(),
    })
}

/// Validate that an ID prefix contains only hex digits and hyphens.
/// Returns `NoteNotFound` for invalid characters so the error message is consistent.
pub(crate) fn validate_id_prefix(prefix: &str) -> Result<(), CliError> {
    if prefix.chars().all(|c| c.is_ascii_hexdigit() || c == '-') {
        Ok(())
    } else {
        Err(CliError::NoteNotFound {
            id: prefix.to_string(),
        })
    }
}

// ─── NoteDb trait ────────────────────────────────────────────────────────────

#[async_trait(?Send)]
pub trait NoteDb {
    fn user_id(&self) -> &str;

    // Note resolution
    async fn resolve_note_id(&self, prefix: &str) -> Result<String, CliError>;
    async fn resolve_archived_note_id(&self, prefix: &str) -> Result<String, CliError>;

    // Note reads
    async fn find_note(&self, id: &str) -> Result<Note, CliError>;
    async fn find_archived_note(&self, id: &str) -> Result<Note, CliError>;
    async fn find_note_content(&self, id: &str) -> Result<Option<String>, CliError>;
    async fn list_notes(&self, filter: &NoteFilter<'_>) -> Result<Vec<Note>, CliError>;
    async fn search_notes(
        &self,
        keywords: &[String],
        filter: &NoteFilter<'_>,
    ) -> Result<Vec<Note>, CliError>;

    // Note writes
    async fn insert_note(&self, req: &InsertNoteReq<'_>) -> Result<InsertedNote, CliError>;
    /// Update content. When `requeue` is true, also sets status = 'ai_queued'.
    async fn update_note_content(
        &self,
        id: &str,
        content: &str,
        requeue: bool,
    ) -> Result<(), CliError>;
    /// Set deleted_at to the given timestamp, or NULL when `deleted_at` is None.
    /// `now` is used for the `updated_at` column and must match the timestamp
    /// used in the hook payload so subscribers see consistent values.
    async fn set_note_deleted_at(
        &self,
        id: &str,
        deleted_at: Option<&str>,
        now: &str,
    ) -> Result<(), CliError>;

    /// Restore the most recently deleted note (sets deleted_at = NULL).
    /// Returns `Ok(())` for both "note restored" and "nothing to undo" — callers
    /// cannot distinguish the two cases.
    async fn undo_last_delete(&self) -> Result<(), CliError>;

    // Project reads
    async fn find_project_by_name(&self, name: &str) -> Result<Option<String>, CliError>;
    async fn find_project_name_by_id(&self, project_id: &str) -> Result<Option<String>, CliError>;
    async fn list_projects(&self, archived: bool) -> Result<Vec<Project>, CliError>;
    async fn find_project(&self, id: &str) -> Result<Project, CliError>;
    async fn resolve_project_id(&self, prefix: &str) -> Result<String, CliError>;

    // Project writes
    async fn create_project(&self, name: &str) -> Result<String, CliError>;

    /// Move a note to a different project. Returns the deleted project name if the old
    /// project is now empty. Returns `NoteNotFound` if no such note exists.
    async fn move_note_to_project(
        &self,
        note_id: &str,
        new_project_id: &str,
        old_project_id: Option<&str>,
    ) -> Result<Option<String>, CliError>;

    /// Update project metadata. `None` = don't change, `Some(None)` = clear, `Some(Some(v))` = set.
    async fn update_project(
        &self,
        id: &str,
        prompt_id: Option<Option<&str>>,
        keyterm_id: Option<Option<&str>>,
        color: Option<Option<&str>>,
    ) -> Result<(), CliError>;

    /// Delete (archive) a project by ID. Returns `ProjectNotFound` if no such project exists.
    async fn delete_project(&self, id: &str) -> Result<(), CliError>;

    // Note metadata writes
    /// Update a note's title. Returns `NoteNotFound` if no such note exists.
    async fn update_note_title(&self, id: &str, title: &str) -> Result<(), CliError>;
    /// Update a note's flagged status. Returns `NoteNotFound` if no such note exists.
    async fn update_note_flagged(&self, id: &str, flagged: bool) -> Result<(), CliError>;

    // Note reads (extended)
    async fn count_notes(&self, filter: &NoteFilter<'_>) -> Result<u64, CliError>;
    async fn list_note_topics(
        &self,
        note_ids: &[&str],
    ) -> Result<std::collections::HashMap<String, Vec<String>>, CliError>;
    /// Read extraction rows for one or more notes. Returns a map of note_id -> Vec<(type, value)>.
    /// `extraction_types` filters which types to read (e.g. `topic`, `entity`).
    /// Results are ordered by type then value for deterministic rendering.
    async fn list_note_extractions(
        &self,
        note_ids: &[&str],
        extraction_types: &[&str],
    ) -> Result<std::collections::HashMap<String, Vec<(String, String)>>, CliError>;
    /// Replace all extraction rows for one note and one managed type in a single operation.
    /// `values` replaces all rows of the given type for the note.
    /// An empty vec clears all rows for that type.
    async fn set_note_extractions(
        &self,
        note_id: &str,
        extraction_type: &str,
        values: &[String],
    ) -> Result<(), CliError>;

    // Prompt operations
    async fn resolve_prompt_id(&self, prefix: &str) -> Result<String, CliError>;
    async fn insert_prompt(
        &self,
        id: &str,
        title: &str,
        description: Option<&str>,
        prompt: &str,
        now: &str,
    ) -> Result<(), CliError>;
    async fn find_prompt(&self, id: &str) -> Result<Prompt, CliError>;
    async fn list_prompts(&self) -> Result<Vec<Prompt>, CliError>;
    async fn update_prompt(
        &self,
        id: &str,
        title: Option<&str>,
        description: Option<&str>,
        prompt: Option<&str>,
    ) -> Result<(), CliError>;
    async fn delete_prompt(&self, id: &str) -> Result<(), CliError>;

    // Keyterm operations
    async fn resolve_keyterm_id(&self, prefix: &str) -> Result<String, CliError>;
    async fn insert_keyterm(
        &self,
        id: &str,
        name: &str,
        description: Option<&str>,
        content: Option<&str>,
        now: &str,
    ) -> Result<(), CliError>;
    async fn find_keyterm(&self, id: &str) -> Result<Keyterm, CliError>;
    async fn list_keyterms(&self) -> Result<Vec<Keyterm>, CliError>;
    async fn update_keyterm(
        &self,
        id: &str,
        name: Option<&str>,
        description: Option<&str>,
        content: Option<&str>,
    ) -> Result<(), CliError>;
    async fn delete_keyterm(&self, id: &str) -> Result<(), CliError>;
}

// ─── SqliteBackend ───────────────────────────────────────────────────────────

#[cfg(feature = "powersync")]
pub struct SqliteBackend {
    pub db: Database,
    pub user_id: String,
}

// SQLite SQL constants — all scope by user_id.
// id column is TEXT in SQLite schema, so LIKE works directly.

#[cfg(feature = "powersync")]
const SQ_RESOLVE_UUID: &str =
    "SELECT id FROM notes WHERE user_id = ? AND id = ? AND deleted_at IS NULL LIMIT 1";
#[cfg(feature = "powersync")]
const SQ_RESOLVE_SHORT_ID: &str =
    "SELECT id FROM notes WHERE user_id = ? AND short_id = ? AND deleted_at IS NULL LIMIT 1";
#[cfg(feature = "powersync")]
const SQ_RESOLVE_ARCHIVED_UUID: &str =
    "SELECT id FROM notes WHERE user_id = ? AND id = ? AND deleted_at IS NOT NULL LIMIT 1";
#[cfg(feature = "powersync")]
const SQ_RESOLVE_ARCHIVED_SHORT_ID: &str =
    "SELECT id FROM notes WHERE user_id = ? AND short_id = ? AND deleted_at IS NOT NULL LIMIT 1";
#[cfg(feature = "powersync")]
const SQ_FIND: &str = "SELECT id, short_id, user_id, type, status, title, content, summary, is_flagged, \
     project_id, metadata, source, created_at, updated_at, deleted_at \
     FROM notes WHERE user_id = ? AND id = ? AND deleted_at IS NULL LIMIT 1";
#[cfg(feature = "powersync")]
const SQ_FIND_ARCHIVED: &str = "SELECT id, short_id, user_id, type, status, title, content, summary, is_flagged, \
     project_id, metadata, source, created_at, updated_at, deleted_at \
     FROM notes WHERE user_id = ? AND id = ? AND deleted_at IS NOT NULL LIMIT 1";
#[cfg(feature = "powersync")]
const SQ_FIND_CONTENT: &str =
    "SELECT content FROM notes WHERE user_id = ? AND id = ? AND deleted_at IS NULL LIMIT 1";
#[cfg(feature = "powersync")]
const SQ_INSERT: &str = "INSERT INTO notes \
     (id, user_id, type, status, title, content, metadata, project_id, created_at, updated_at) \
     VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)";
#[cfg(feature = "powersync")]
const SQ_UPDATE_CONTENT: &str = "UPDATE notes \
     SET content = ?, status = CASE WHEN ? THEN 'ai_queued' ELSE status END, updated_at = ? \
     WHERE user_id = ? AND id = ?";
#[cfg(feature = "powersync")]
const SQ_SET_DELETED_AT: &str =
    "UPDATE notes SET deleted_at = ?, updated_at = ? WHERE user_id = ? AND id = ?";
#[cfg(feature = "powersync")]
const SQ_SET_DELETED_AT_NULL: &str =
    "UPDATE notes SET deleted_at = NULL, updated_at = ? WHERE user_id = ? AND id = ?";
#[cfg(feature = "powersync")]
const SQ_UPDATE_PROJECT: &str =
    "UPDATE notes SET project_id = ?, updated_at = ? WHERE user_id = ? AND id = ?";

#[cfg(feature = "powersync")]
const SQ_FIND_PROJECT: &str = "SELECT id FROM projects WHERE user_id = ? AND name = ? \
     AND (is_archived = 0 OR is_archived IS NULL) LIMIT 1";
#[cfg(feature = "powersync")]
const SQ_FIND_PROJECT_NAME: &str = "SELECT name FROM projects WHERE user_id = ? AND id = ? LIMIT 1";
#[cfg(feature = "powersync")]
const SQ_LIST_PROJECTS_ACTIVE: &str = "SELECT id, user_id, name, color, prompt_id, keyterm_id, is_archived, created_at FROM projects \
     WHERE user_id = ? AND (is_archived = 0 OR is_archived IS NULL) ORDER BY name";
#[cfg(feature = "powersync")]
const SQ_LIST_PROJECTS_ARCHIVED: &str = "SELECT id, user_id, name, color, prompt_id, keyterm_id, is_archived, created_at FROM projects \
     WHERE user_id = ? AND is_archived = 1 ORDER BY name";
#[cfg(feature = "powersync")]
const SQ_CREATE_PROJECT: &str =
    "INSERT INTO projects (id, user_id, name, is_archived, created_at) VALUES (?, ?, ?, 0, ?)";
#[cfg(feature = "powersync")]
const SQ_COUNT_PROJECT_NOTES: &str =
    "SELECT COUNT(*) FROM notes WHERE user_id = ? AND project_id = ? AND deleted_at IS NULL";
#[cfg(feature = "powersync")]
const SQ_DELETE_PROJECT: &str = "DELETE FROM projects WHERE user_id = ? AND id = ?";

#[cfg(feature = "powersync")]
const SQ_UNDO_DELETE: &str = "UPDATE notes SET deleted_at = NULL, updated_at = ? \
     WHERE id = (SELECT id FROM notes WHERE deleted_at IS NOT NULL AND user_id = ? \
     ORDER BY deleted_at DESC LIMIT 1)";

#[cfg(feature = "powersync")]
const SQ_UPDATE_TITLE: &str =
    "UPDATE notes SET title = ?, updated_at = ? WHERE user_id = ? AND id = ?";
#[cfg(feature = "powersync")]
const SQ_UPDATE_FLAGGED: &str =
    "UPDATE notes SET is_flagged = ?, updated_at = ? WHERE user_id = ? AND id = ?";
#[cfg(feature = "powersync")]
#[cfg(feature = "powersync")]
const SQ_LIST_EXTRACTIONS: &str = "SELECT note_id, type, value FROM note_extractions \
     WHERE user_id = ? AND type IN (SELECT value FROM json_each(?)) \
     AND note_id IN (SELECT value FROM json_each(?)) \
     ORDER BY type, value";
#[cfg(feature = "powersync")]
const SQ_CLEAR_EXTRACTIONS: &str = "DELETE FROM note_extractions \
     WHERE user_id = ? AND note_id = ? AND type = ?";
#[cfg(feature = "powersync")]
const SQ_INSERT_EXTRACTION: &str = "INSERT INTO note_extractions (note_id, user_id, type, value) \
     VALUES (?, ?, ?, ?)";

#[cfg(feature = "powersync")]
const SQ_FIND_PROJECT_BY_ID: &str = "SELECT id, user_id, name, color, prompt_id, keyterm_id, is_archived, created_at FROM projects WHERE user_id = ? AND id = ? LIMIT 1";
#[cfg(feature = "powersync")]
const SQ_RESOLVE_PROJECT: &str = "SELECT id FROM projects WHERE user_id = ? AND id LIKE ? LIMIT 2";
#[cfg(feature = "powersync")]
const SQ_ARCHIVE_PROJECT: &str = "UPDATE projects SET is_archived = 1 WHERE user_id = ? AND id = ?";
#[cfg(feature = "powersync")]
const SQ_RESOLVE_PROMPT: &str = "SELECT id FROM prompts WHERE user_id = ? AND id LIKE ? LIMIT 2";
#[cfg(feature = "powersync")]
const SQ_INSERT_PROMPT: &str = "INSERT INTO prompts (id, user_id, title, description, prompt, created_at) VALUES (?, ?, ?, ?, ?, ?)";
#[cfg(feature = "powersync")]
const SQ_FIND_PROMPT: &str = "SELECT id, user_id, title, description, prompt, created_at FROM prompts WHERE user_id = ? AND id = ? LIMIT 1";
#[cfg(feature = "powersync")]
const SQ_LIST_PROMPTS: &str = "SELECT id, user_id, title, description, prompt, created_at FROM prompts WHERE user_id = ? ORDER BY created_at DESC";
#[cfg(feature = "powersync")]
const SQ_DELETE_PROMPT: &str = "DELETE FROM prompts WHERE user_id = ? AND id = ?";
#[cfg(feature = "powersync")]
const SQ_RESOLVE_KEYTERM: &str = "SELECT id FROM keyterms WHERE user_id = ? AND id LIKE ? LIMIT 2";
#[cfg(feature = "powersync")]
const SQ_INSERT_KEYTERM: &str = "INSERT INTO keyterms (id, user_id, name, description, content, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?)";
#[cfg(feature = "powersync")]
const SQ_FIND_KEYTERM: &str = "SELECT id, user_id, name, description, content, created_at, updated_at FROM keyterms WHERE user_id = ? AND id = ? LIMIT 1";
#[cfg(feature = "powersync")]
const SQ_LIST_KEYTERMS: &str = "SELECT id, user_id, name, description, content, created_at, updated_at FROM keyterms WHERE user_id = ? ORDER BY name";
#[cfg(feature = "powersync")]
const SQ_DELETE_KEYTERM: &str = "DELETE FROM keyterms WHERE user_id = ? AND id = ?";
#[cfg(feature = "powersync")]
async fn resolve_sqlite_id(
    pool: &SqlitePool,
    sql: &str,
    user_id: &str,
    prefix: &str,
    ambiguous: &str,
    missing: impl FnOnce() -> CliError,
) -> Result<String, CliError> {
    let rows = sqlx::query_scalar::<_, String>(sql)
        .bind(user_id)
        .bind(format!("{prefix}%"))
        .fetch_all(pool)
        .await?;

    match rows.as_slice() {
        [_, _, ..] => Err(CliError::Other(format!(
            "Ambiguous {ambiguous} prefix: {prefix}"
        ))),
        [id] => Ok(id.clone()),
        [] => Err(missing()),
    }
}

#[cfg(feature = "powersync")]
async fn resolve_sqlite_note_id(
    pool: &SqlitePool,
    user_id: &str,
    input: &str,
    uuid_sql: &str,
    short_id_sql: &str,
) -> Result<String, CliError> {
    match parse_note_lookup(input)? {
        NoteLookup::ShortId(short_id) => sqlx::query_scalar::<_, String>(short_id_sql)
            .bind(user_id)
            .bind(short_id)
            .fetch_optional(pool)
            .await?
            .ok_or_else(|| CliError::NoteNotFound {
                id: input.to_string(),
            }),
        NoteLookup::Uuid(uuid) => sqlx::query_scalar::<_, String>(uuid_sql)
            .bind(user_id)
            .bind(uuid)
            .fetch_optional(pool)
            .await?
            .ok_or_else(|| CliError::NoteNotFound {
                id: input.to_string(),
            }),
    }
}

#[cfg(feature = "powersync")]
async fn sqlite_exists(
    pool: &SqlitePool,
    sql: &str,
    user_id: &str,
    id: &str,
) -> Result<bool, CliError> {
    let exists = sqlx::query_scalar::<_, i64>(sql)
        .bind(user_id)
        .bind(id)
        .fetch_optional(pool)
        .await?;
    Ok(exists.is_some())
}

#[cfg(feature = "powersync")]
#[async_trait(?Send)]
impl NoteDb for SqliteBackend {
    fn user_id(&self) -> &str {
        &self.user_id
    }

    async fn resolve_note_id(&self, prefix: &str) -> Result<String, CliError> {
        resolve_sqlite_note_id(
            &self.db.pool,
            &self.user_id,
            prefix,
            SQ_RESOLVE_UUID,
            SQ_RESOLVE_SHORT_ID,
        )
        .await
    }

    async fn resolve_archived_note_id(&self, prefix: &str) -> Result<String, CliError> {
        resolve_sqlite_note_id(
            &self.db.pool,
            &self.user_id,
            prefix,
            SQ_RESOLVE_ARCHIVED_UUID,
            SQ_RESOLVE_ARCHIVED_SHORT_ID,
        )
        .await
    }

    async fn find_note(&self, id: &str) -> Result<Note, CliError> {
        sqlx::query_as::<_, Note>(SQ_FIND)
            .bind(&self.user_id)
            .bind(id)
            .fetch_optional(&self.db.pool)
            .await?
            .ok_or_else(|| CliError::NoteNotFound { id: id.to_string() })
    }

    async fn find_archived_note(&self, id: &str) -> Result<Note, CliError> {
        sqlx::query_as::<_, Note>(SQ_FIND_ARCHIVED)
            .bind(&self.user_id)
            .bind(id)
            .fetch_optional(&self.db.pool)
            .await?
            .ok_or_else(|| CliError::NoteNotFound { id: id.to_string() })
    }

    async fn find_note_content(&self, id: &str) -> Result<Option<String>, CliError> {
        sqlx::query_scalar::<_, Option<String>>(SQ_FIND_CONTENT)
            .bind(&self.user_id)
            .bind(id)
            .fetch_optional(&self.db.pool)
            .await?
            .ok_or_else(|| CliError::NoteNotFound { id: id.to_string() })
    }

    async fn list_notes(&self, filter: &NoteFilter<'_>) -> Result<Vec<Note>, CliError> {
        let limit = i64::from(filter.limit);
        Ok(sqlx::query_as!(
            Note,
            r#"
            SELECT
                id as "id!",
                short_id,
                user_id as "user_id!",
                type as "type!",
                status as "status!",
                title,
                content,
                summary,
                is_flagged,
                project_id,
                metadata,
                source,
                created_at,
                updated_at,
                deleted_at
            FROM notes
            WHERE user_id = ?
              AND (deleted_at IS NOT NULL) = ?
              AND (? IS NULL OR type = ?)
              AND (? IS NULL OR project_id = ?)
            ORDER BY created_at DESC
            LIMIT ?
            "#,
            self.user_id,
            filter.archived,
            filter.note_type,
            filter.note_type,
            filter.project_id,
            filter.project_id,
            limit,
        )
        .fetch_all(&self.db.pool)
        .await?)
    }

    async fn search_notes(
        &self,
        keywords: &[String],
        filter: &NoteFilter<'_>,
    ) -> Result<Vec<Note>, CliError> {
        if keywords.is_empty() {
            return Err(CliError::Other(
                "search_notes requires at least one keyword".into(),
            ));
        }
        let limit = i64::from(filter.limit);
        let keywords_json = serde_json::to_string(keywords)?;
        Ok(sqlx::query_as!(
            Note,
            r#"
            SELECT
                id as "id!",
                short_id,
                user_id as "user_id!",
                type as "type!",
                status as "status!",
                title,
                content,
                summary,
                is_flagged,
                project_id,
                metadata,
                source,
                created_at,
                updated_at,
                deleted_at
            FROM notes
            WHERE user_id = ?
              AND (deleted_at IS NOT NULL) = ?
              AND (? IS NULL OR type = ?)
              AND (? IS NULL OR project_id = ?)
              AND EXISTS (
                SELECT 1 FROM json_each(?) AS kw
                WHERE title LIKE '%' || kw.value || '%'
                   OR content LIKE '%' || kw.value || '%'
                   OR summary LIKE '%' || kw.value || '%'
              )
            ORDER BY updated_at DESC
            LIMIT ?
            "#,
            self.user_id,
            filter.archived,
            filter.note_type,
            filter.note_type,
            filter.project_id,
            filter.project_id,
            keywords_json,
            limit,
        )
        .fetch_all(&self.db.pool)
        .await?)
    }

    async fn insert_note(&self, req: &InsertNoteReq<'_>) -> Result<InsertedNote, CliError> {
        sqlx::query(SQ_INSERT)
            .bind(req.id)
            .bind(&self.user_id)
            .bind(req.note_type)
            .bind(req.status)
            .bind(req.title)
            .bind(req.content)
            .bind(req.metadata)
            .bind(req.project_id)
            .bind(req.now)
            .bind(req.now)
            .execute(&self.db.pool)
            .await?;
        Ok(InsertedNote {
            uuid: req.id.to_string(),
            short_id: None,
        })
    }

    async fn update_note_content(
        &self,
        id: &str,
        content: &str,
        requeue: bool,
    ) -> Result<(), CliError> {
        let now = chrono::Utc::now().to_rfc3339();
        sqlx::query(SQ_UPDATE_CONTENT)
            .bind(content)
            .bind(requeue)
            .bind(now)
            .bind(&self.user_id)
            .bind(id)
            .execute(&self.db.pool)
            .await?;
        Ok(())
    }

    async fn set_note_deleted_at(
        &self,
        id: &str,
        deleted_at: Option<&str>,
        now: &str,
    ) -> Result<(), CliError> {
        if let Some(ts) = deleted_at {
            sqlx::query(SQ_SET_DELETED_AT)
                .bind(ts)
                .bind(now)
                .bind(&self.user_id)
                .bind(id)
                .execute(&self.db.pool)
                .await?;
        } else {
            sqlx::query(SQ_SET_DELETED_AT_NULL)
                .bind(now)
                .bind(&self.user_id)
                .bind(id)
                .execute(&self.db.pool)
                .await?;
        }
        Ok(())
    }

    async fn undo_last_delete(&self) -> Result<(), CliError> {
        let now = chrono::Utc::now().to_rfc3339();
        sqlx::query(SQ_UNDO_DELETE)
            .bind(now)
            .bind(&self.user_id)
            .execute(&self.db.pool)
            .await?;
        Ok(())
    }

    async fn find_project_by_name(&self, name: &str) -> Result<Option<String>, CliError> {
        Ok(sqlx::query_scalar::<_, String>(SQ_FIND_PROJECT)
            .bind(&self.user_id)
            .bind(name)
            .fetch_optional(&self.db.pool)
            .await?)
    }

    async fn find_project_name_by_id(&self, project_id: &str) -> Result<Option<String>, CliError> {
        Ok(sqlx::query_scalar::<_, String>(SQ_FIND_PROJECT_NAME)
            .bind(&self.user_id)
            .bind(project_id)
            .fetch_optional(&self.db.pool)
            .await?)
    }

    async fn list_projects(&self, archived: bool) -> Result<Vec<Project>, CliError> {
        let sql = if archived {
            SQ_LIST_PROJECTS_ARCHIVED
        } else {
            SQ_LIST_PROJECTS_ACTIVE
        };
        Ok(sqlx::query_as::<_, Project>(sql)
            .bind(&self.user_id)
            .fetch_all(&self.db.pool)
            .await?)
    }

    async fn create_project(&self, name: &str) -> Result<String, CliError> {
        let id = uuid::Uuid::new_v4().to_string();
        let now = chrono::Utc::now().to_rfc3339();
        sqlx::query(SQ_CREATE_PROJECT)
            .bind(&id)
            .bind(&self.user_id)
            .bind(name)
            .bind(now)
            .execute(&self.db.pool)
            .await?;
        Ok(id)
    }

    async fn move_note_to_project(
        &self,
        note_id: &str,
        new_project_id: &str,
        old_project_id: Option<&str>,
    ) -> Result<Option<String>, CliError> {
        let now = chrono::Utc::now().to_rfc3339();
        let mut tx = self.db.pool.begin().await?;
        let exists = sqlx::query_scalar::<_, i64>(
            "SELECT 1 FROM notes WHERE user_id = ? AND id = ? AND deleted_at IS NULL LIMIT 1",
        )
        .bind(&self.user_id)
        .bind(note_id)
        .fetch_optional(&mut *tx)
        .await?
        .is_some();
        if !exists {
            return Err(CliError::NoteNotFound {
                id: note_id.to_string(),
            });
        }

        sqlx::query(SQ_UPDATE_PROJECT)
            .bind(new_project_id)
            .bind(now)
            .bind(&self.user_id)
            .bind(note_id)
            .execute(&mut *tx)
            .await?;

        let Some(old_pid) = old_project_id else {
            tx.commit().await?;
            return Ok(None);
        };

        let count = sqlx::query_scalar::<_, i64>(SQ_COUNT_PROJECT_NOTES)
            .bind(&self.user_id)
            .bind(old_pid)
            .fetch_one(&mut *tx)
            .await?;

        if count != 0 {
            tx.commit().await?;
            return Ok(None);
        }

        let old_name = sqlx::query_scalar::<_, String>(SQ_FIND_PROJECT_NAME)
            .bind(&self.user_id)
            .bind(old_pid)
            .fetch_optional(&mut *tx)
            .await?;
        sqlx::query(SQ_DELETE_PROJECT)
            .bind(&self.user_id)
            .bind(old_pid)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(old_name)
    }

    async fn find_project(&self, id: &str) -> Result<Project, CliError> {
        sqlx::query_as::<_, Project>(SQ_FIND_PROJECT_BY_ID)
            .bind(&self.user_id)
            .bind(id)
            .fetch_optional(&self.db.pool)
            .await?
            .ok_or_else(|| CliError::Other(format!("Project not found: {id}")))
    }

    async fn resolve_project_id(&self, prefix: &str) -> Result<String, CliError> {
        validate_id_prefix(prefix)?;
        resolve_sqlite_id(
            &self.db.pool,
            SQ_RESOLVE_PROJECT,
            &self.user_id,
            prefix,
            "project ID",
            || CliError::Other(format!("Project not found: {prefix}")),
        )
        .await
    }

    async fn update_project(
        &self,
        id: &str,
        prompt_id: Option<Option<&str>>,
        keyterm_id: Option<Option<&str>>,
        color: Option<Option<&str>>,
    ) -> Result<(), CliError> {
        let update_prompt = prompt_id.is_some();
        let update_keyterm = keyterm_id.is_some();
        let update_color = color.is_some();
        if !(update_prompt || update_keyterm || update_color) {
            return Ok(());
        }

        let prompt_value = prompt_id.flatten();
        let keyterm_value = keyterm_id.flatten();
        let color_value = color.flatten();
        sqlx::query!(
            r#"
            UPDATE projects SET
                prompt_id = CASE WHEN ? THEN ? ELSE prompt_id END,
                keyterm_id = CASE WHEN ? THEN ? ELSE keyterm_id END,
                color = CASE WHEN ? THEN ? ELSE color END
            WHERE user_id = ? AND id = ?
            "#,
            update_prompt,
            prompt_value,
            update_keyterm,
            keyterm_value,
            update_color,
            color_value,
            self.user_id,
            id,
        )
        .execute(&self.db.pool)
        .await?;
        Ok(())
    }

    async fn delete_project(&self, id: &str) -> Result<(), CliError> {
        if !sqlite_exists(
            &self.db.pool,
            "SELECT 1 FROM projects WHERE user_id = ? AND id = ? LIMIT 1",
            &self.user_id,
            id,
        )
        .await?
        {
            return Err(CliError::Other(format!("Project not found: {id}")));
        }
        sqlx::query(SQ_ARCHIVE_PROJECT)
            .bind(&self.user_id)
            .bind(id)
            .execute(&self.db.pool)
            .await?;
        Ok(())
    }

    async fn update_note_title(&self, id: &str, title: &str) -> Result<(), CliError> {
        let now = chrono::Utc::now().to_rfc3339();
        if !sqlite_exists(
            &self.db.pool,
            "SELECT 1 FROM notes WHERE user_id = ? AND id = ? AND deleted_at IS NULL LIMIT 1",
            &self.user_id,
            id,
        )
        .await?
        {
            return Err(CliError::NoteNotFound { id: id.to_string() });
        }
        sqlx::query(SQ_UPDATE_TITLE)
            .bind(title)
            .bind(now)
            .bind(&self.user_id)
            .bind(id)
            .execute(&self.db.pool)
            .await?;
        Ok(())
    }

    async fn update_note_flagged(&self, id: &str, flagged: bool) -> Result<(), CliError> {
        let now = chrono::Utc::now().to_rfc3339();
        let val: i64 = if flagged { 1 } else { 0 };
        if !sqlite_exists(
            &self.db.pool,
            "SELECT 1 FROM notes WHERE user_id = ? AND id = ? AND deleted_at IS NULL LIMIT 1",
            &self.user_id,
            id,
        )
        .await?
        {
            return Err(CliError::NoteNotFound { id: id.to_string() });
        }
        sqlx::query(SQ_UPDATE_FLAGGED)
            .bind(val)
            .bind(now)
            .bind(&self.user_id)
            .bind(id)
            .execute(&self.db.pool)
            .await?;
        Ok(())
    }

    async fn count_notes(&self, filter: &NoteFilter<'_>) -> Result<u64, CliError> {
        let count = sqlx::query_scalar!(
            r#"
            SELECT COUNT(*) as "count!: i64"
            FROM notes
            WHERE user_id = ?
              AND (deleted_at IS NOT NULL) = ?
              AND (? IS NULL OR type = ?)
              AND (? IS NULL OR project_id = ?)
            "#,
            self.user_id,
            filter.archived,
            filter.note_type,
            filter.note_type,
            filter.project_id,
            filter.project_id,
        )
        .fetch_one(&self.db.pool)
        .await?;
        count
            .try_into()
            .map_err(|_| CliError::Other(format!("unexpected negative count: {count}")))
    }

    async fn list_note_topics(
        &self,
        note_ids: &[&str],
    ) -> Result<std::collections::HashMap<String, Vec<String>>, CliError> {
        let extractions = self.list_note_extractions(note_ids, &["topic"]).await?;
        let mut map = std::collections::HashMap::new();
        for (note_id, pairs) in extractions {
            map.insert(note_id, pairs.into_iter().map(|(_, value)| value).collect());
        }
        Ok(map)
    }
    async fn list_note_extractions(
        &self,
        note_ids: &[&str],
        extraction_types: &[&str],
    ) -> Result<std::collections::HashMap<String, Vec<(String, String)>>, CliError> {
        if note_ids.is_empty() || extraction_types.is_empty() {
            return Ok(std::collections::HashMap::new());
        }
        let note_ids_json = serde_json::to_string(note_ids)?;
        let types_json = serde_json::to_string(extraction_types)?;
        let rows = sqlx::query_as::<_, (String, String, String)>(SQ_LIST_EXTRACTIONS)
            .bind(&self.user_id)
            .bind(types_json)
            .bind(note_ids_json)
            .fetch_all(&self.db.pool)
            .await?;
        let mut map: std::collections::HashMap<String, Vec<(String, String)>> =
            std::collections::HashMap::new();
        for (note_id, ext_type, value) in rows {
            map.entry(note_id).or_default().push((ext_type, value));
        }
        Ok(map)
    }
    async fn set_note_extractions(
        &self,
        note_id: &str,
        extraction_type: &str,
        values: &[String],
    ) -> Result<(), CliError> {
        // Delete all existing rows for this note + type
        sqlx::query(SQ_CLEAR_EXTRACTIONS)
            .bind(&self.user_id)
            .bind(note_id)
            .bind(extraction_type)
            .execute(&self.db.pool)
            .await?;
        // Insert new values
        for value in values {
            sqlx::query(SQ_INSERT_EXTRACTION)
                .bind(note_id)
                .bind(&self.user_id)
                .bind(extraction_type)
                .bind(value)
                .execute(&self.db.pool)
                .await?;
        }
        Ok(())
    }

    async fn resolve_prompt_id(&self, prefix: &str) -> Result<String, CliError> {
        validate_id_prefix(prefix)?;
        resolve_sqlite_id(
            &self.db.pool,
            SQ_RESOLVE_PROMPT,
            &self.user_id,
            prefix,
            "prompt ID",
            || CliError::Other(format!("Prompt not found: {prefix}")),
        )
        .await
    }

    async fn insert_prompt(
        &self,
        id: &str,
        title: &str,
        description: Option<&str>,
        prompt: &str,
        now: &str,
    ) -> Result<(), CliError> {
        sqlx::query(SQ_INSERT_PROMPT)
            .bind(id)
            .bind(&self.user_id)
            .bind(title)
            .bind(description)
            .bind(prompt)
            .bind(now)
            .execute(&self.db.pool)
            .await?;
        Ok(())
    }

    async fn find_prompt(&self, id: &str) -> Result<Prompt, CliError> {
        sqlx::query_as::<_, Prompt>(SQ_FIND_PROMPT)
            .bind(&self.user_id)
            .bind(id)
            .fetch_optional(&self.db.pool)
            .await?
            .ok_or_else(|| CliError::Other(format!("Prompt not found: {id}")))
    }

    async fn list_prompts(&self) -> Result<Vec<Prompt>, CliError> {
        Ok(sqlx::query_as::<_, Prompt>(SQ_LIST_PROMPTS)
            .bind(&self.user_id)
            .fetch_all(&self.db.pool)
            .await?)
    }

    async fn update_prompt(
        &self,
        id: &str,
        title: Option<&str>,
        description: Option<&str>,
        prompt: Option<&str>,
    ) -> Result<(), CliError> {
        let update_title = title.is_some();
        let update_description = description.is_some();
        let update_prompt = prompt.is_some();
        if !(update_title || update_description || update_prompt) {
            return Ok(());
        }

        sqlx::query!(
            r#"
            UPDATE prompts SET
                title = CASE WHEN ? THEN ? ELSE title END,
                description = CASE WHEN ? THEN ? ELSE description END,
                prompt = CASE WHEN ? THEN ? ELSE prompt END
            WHERE user_id = ? AND id = ?
            "#,
            update_title,
            title,
            update_description,
            description,
            update_prompt,
            prompt,
            self.user_id,
            id,
        )
        .execute(&self.db.pool)
        .await?;
        Ok(())
    }

    async fn delete_prompt(&self, id: &str) -> Result<(), CliError> {
        sqlx::query(SQ_DELETE_PROMPT)
            .bind(&self.user_id)
            .bind(id)
            .execute(&self.db.pool)
            .await?;
        Ok(())
    }

    async fn resolve_keyterm_id(&self, prefix: &str) -> Result<String, CliError> {
        validate_id_prefix(prefix)?;
        resolve_sqlite_id(
            &self.db.pool,
            SQ_RESOLVE_KEYTERM,
            &self.user_id,
            prefix,
            "keyterm ID",
            || CliError::Other(format!("Keyterm not found: {prefix}")),
        )
        .await
    }

    async fn insert_keyterm(
        &self,
        id: &str,
        name: &str,
        description: Option<&str>,
        content: Option<&str>,
        now: &str,
    ) -> Result<(), CliError> {
        sqlx::query(SQ_INSERT_KEYTERM)
            .bind(id)
            .bind(&self.user_id)
            .bind(name)
            .bind(description)
            .bind(content)
            .bind(now)
            .bind(now)
            .execute(&self.db.pool)
            .await?;
        Ok(())
    }

    async fn find_keyterm(&self, id: &str) -> Result<Keyterm, CliError> {
        sqlx::query_as::<_, Keyterm>(SQ_FIND_KEYTERM)
            .bind(&self.user_id)
            .bind(id)
            .fetch_optional(&self.db.pool)
            .await?
            .ok_or_else(|| CliError::Other(format!("Keyterm not found: {id}")))
    }

    async fn list_keyterms(&self) -> Result<Vec<Keyterm>, CliError> {
        Ok(sqlx::query_as::<_, Keyterm>(SQ_LIST_KEYTERMS)
            .bind(&self.user_id)
            .fetch_all(&self.db.pool)
            .await?)
    }

    async fn update_keyterm(
        &self,
        id: &str,
        name: Option<&str>,
        description: Option<&str>,
        content: Option<&str>,
    ) -> Result<(), CliError> {
        let now = chrono::Utc::now().to_rfc3339();
        let update_name = name.is_some();
        let update_description = description.is_some();
        let update_content = content.is_some();
        if !(update_name || update_description || update_content) {
            return Ok(());
        }

        sqlx::query!(
            r#"
            UPDATE keyterms SET
                name = CASE WHEN ? THEN ? ELSE name END,
                description = CASE WHEN ? THEN ? ELSE description END,
                content = CASE WHEN ? THEN ? ELSE content END,
                updated_at = CASE WHEN (? OR ? OR ?) THEN ? ELSE updated_at END
            WHERE user_id = ? AND id = ?
            "#,
            update_name,
            name,
            update_description,
            description,
            update_content,
            content,
            update_name,
            update_description,
            update_content,
            now,
            self.user_id,
            id,
        )
        .execute(&self.db.pool)
        .await?;
        Ok(())
    }

    async fn delete_keyterm(&self, id: &str) -> Result<(), CliError> {
        sqlx::query(SQ_DELETE_KEYTERM)
            .bind(&self.user_id)
            .bind(id)
            .execute(&self.db.pool)
            .await?;
        Ok(())
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
#[cfg(feature = "powersync")]
mod tests {
    use super::*;

    async fn make_backend() -> SqliteBackend {
        use crate::config::{Config, ConfigPaths};
        use tempfile::tempdir;

        let dir = tempdir().unwrap();
        let config = Config {
            supabase_url: String::new(),
            supabase_anon_key: String::new(),
            powersync_url: String::new(),
            api_url: String::new(),
            web_url: None,
            paths: ConfigPaths {
                config_dir: dir.path().to_path_buf(),
                data_dir: dir.path().to_path_buf(),
                config_file: dir.path().join("config.json"),
                session_file: dir.path().join("session.json"),
                db_file: dir.path().join("test.db"),
                log_file: dir.path().join("test.log"),
            },
        };

        let db = Database::open_local(&config).await.unwrap();
        let user_id = "test-user-id".to_string();

        // Keep dir alive by leaking it — acceptable in tests
        std::mem::forget(dir);

        SqliteBackend { db, user_id }
    }

    #[tokio::test]
    async fn test_sqlite_backend_insert_and_find() {
        let backend = make_backend().await;
        let id = uuid::Uuid::new_v4().to_string();
        let now = chrono::Utc::now().to_rfc3339();

        let inserted = backend
            .insert_note(&InsertNoteReq {
                id: &id,
                note_type: "normal",
                status: "ai_queued",
                title: Some("Hello world"),
                content: Some("# Hello world\n\nContent here."),
                metadata: None,
                project_id: None,
                now: &now,
            })
            .await
            .unwrap();

        assert_eq!(inserted.uuid, id);
        assert_eq!(inserted.short_id, None);

        // Find by full id
        let note = backend.find_note(&id).await.unwrap();
        assert_eq!(note.id, id);
        assert_eq!(note.title, Some("Hello world".to_string()));

        // Find by full UUID compatibility path
        let resolved = backend.resolve_note_id(&id).await.unwrap();
        assert_eq!(resolved, id);

        sqlx::query("UPDATE notes SET short_id = 42 WHERE id = ?")
            .bind(&id)
            .execute(&backend.db.pool)
            .await
            .unwrap();
        let resolved = backend.resolve_note_id("42").await.unwrap();
        assert_eq!(resolved, id);

        // UUID prefixes are no longer accepted.
        let prefix = &id[..8];
        assert!(backend.resolve_note_id(prefix).await.is_err());

        // Find content
        let content = backend.find_note_content(&id).await.unwrap();
        assert_eq!(content, Some("# Hello world\n\nContent here.".to_string()));
    }

    #[tokio::test]
    async fn test_sqlite_backend_list_filter() {
        let backend = make_backend().await;
        let now = chrono::Utc::now().to_rfc3339();

        // Create two projects
        let proj_a = backend.create_project("Project A").await.unwrap();
        let proj_b = backend.create_project("Project B").await.unwrap();

        // Insert notes in different projects
        let id_a = uuid::Uuid::new_v4().to_string();
        backend
            .insert_note(&InsertNoteReq {
                id: &id_a,
                note_type: "normal",
                status: "ai_queued",
                title: Some("Note A"),
                content: Some("content a"),
                metadata: None,
                project_id: Some(&proj_a),
                now: &now,
            })
            .await
            .unwrap();

        let id_b = uuid::Uuid::new_v4().to_string();
        backend
            .insert_note(&InsertNoteReq {
                id: &id_b,
                note_type: "normal",
                status: "ai_queued",
                title: Some("Note B"),
                content: Some("content b"),
                metadata: None,
                project_id: Some(&proj_b),
                now: &now,
            })
            .await
            .unwrap();

        // List by project A
        let notes = backend
            .list_notes(&NoteFilter {
                project_id: Some(&proj_a),
                note_type: None,
                archived: false,
                limit: 20,
            })
            .await
            .unwrap();
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].id, id_a);
    }

    #[tokio::test]
    async fn test_sqlite_backend_search_notes() {
        let backend = make_backend().await;
        let now = chrono::Utc::now().to_rfc3339();

        let id = uuid::Uuid::new_v4().to_string();
        backend
            .insert_note(&InsertNoteReq {
                id: &id,
                note_type: "normal",
                status: "ai_queued",
                title: Some("Unique searchable title"),
                content: Some("some body text"),
                metadata: None,
                project_id: None,
                now: &now,
            })
            .await
            .unwrap();

        let results = backend
            .search_notes(
                &["Unique".to_string()],
                &NoteFilter {
                    project_id: None,
                    note_type: None,
                    archived: false,
                    limit: 20,
                },
            )
            .await
            .unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, id);

        // Empty keywords should return Err
        let err = backend
            .search_notes(
                &[],
                &NoteFilter {
                    project_id: None,
                    note_type: None,
                    archived: false,
                    limit: 20,
                },
            )
            .await;
        assert!(err.is_err());
    }

    #[tokio::test]
    async fn test_sqlite_backend_search_respects_type_filter() {
        let backend = make_backend().await;
        let now = chrono::Utc::now().to_rfc3339();

        let normal_id = uuid::Uuid::new_v4().to_string();
        backend
            .insert_note(&InsertNoteReq {
                id: &normal_id,
                note_type: "normal",
                status: "ai_queued",
                title: Some("Shared searchable title"),
                content: Some("normal body"),
                metadata: None,
                project_id: None,
                now: &now,
            })
            .await
            .unwrap();

        let link_id = uuid::Uuid::new_v4().to_string();
        backend
            .insert_note(&InsertNoteReq {
                id: &link_id,
                note_type: "link",
                status: "ai_queued",
                title: Some("Shared searchable title"),
                content: Some("link body"),
                metadata: None,
                project_id: None,
                now: &now,
            })
            .await
            .unwrap();

        let results = backend
            .search_notes(
                &["Shared".to_string()],
                &NoteFilter {
                    project_id: None,
                    note_type: Some("link"),
                    archived: false,
                    limit: 20,
                },
            )
            .await
            .unwrap();

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, link_id);
    }

    #[tokio::test]
    async fn test_sqlite_backend_archive() {
        let backend = make_backend().await;
        let id = uuid::Uuid::new_v4().to_string();
        let now = chrono::Utc::now().to_rfc3339();

        backend
            .insert_note(&InsertNoteReq {
                id: &id,
                note_type: "normal",
                status: "ai_queued",
                title: Some("To archive"),
                content: Some("content"),
                metadata: None,
                project_id: None,
                now: &now,
            })
            .await
            .unwrap();

        // Verify it appears in active list
        let active = backend
            .list_notes(&NoteFilter {
                project_id: None,
                note_type: None,
                archived: false,
                limit: 20,
            })
            .await
            .unwrap();
        assert!(active.iter().any(|n| n.id == id));

        // Archive it
        backend
            .set_note_deleted_at(&id, Some(&now), &now)
            .await
            .unwrap();

        // Should be gone from active
        let active_after = backend
            .list_notes(&NoteFilter {
                project_id: None,
                note_type: None,
                archived: false,
                limit: 20,
            })
            .await
            .unwrap();
        assert!(!active_after.iter().any(|n| n.id == id));

        // Should appear in archived
        let archived = backend
            .list_notes(&NoteFilter {
                project_id: None,
                note_type: None,
                archived: true,
                limit: 20,
            })
            .await
            .unwrap();
        assert!(archived.iter().any(|n| n.id == id));

        // Unarchive
        backend.set_note_deleted_at(&id, None, &now).await.unwrap();
        let active_restored = backend
            .list_notes(&NoteFilter {
                project_id: None,
                note_type: None,
                archived: false,
                limit: 20,
            })
            .await
            .unwrap();
        assert!(active_restored.iter().any(|n| n.id == id));
    }

    #[tokio::test]
    async fn test_find_archived_note() {
        let backend = make_backend().await;
        let id = uuid::Uuid::new_v4().to_string();
        let now = chrono::Utc::now().to_rfc3339();

        backend
            .insert_note(&InsertNoteReq {
                id: &id,
                note_type: "normal",
                status: "ai_queued",
                title: Some("Archived note"),
                content: Some("content"),
                metadata: None,
                project_id: None,
                now: &now,
            })
            .await
            .unwrap();

        // Not findable via find_archived_note before archiving
        assert!(backend.find_archived_note(&id).await.is_err());

        // Archive it
        backend
            .set_note_deleted_at(&id, Some(&now), &now)
            .await
            .unwrap();

        // Now findable via find_archived_note
        let note = backend.find_archived_note(&id).await.unwrap();
        assert_eq!(note.id, id);
        assert_eq!(note.title, Some("Archived note".to_string()));
        assert!(note.deleted_at.is_some());

        // No longer findable via find_note (active-only)
        assert!(backend.find_note(&id).await.is_err());
    }

    // ─── Fix: PowerSync view-UPDATE zero affected rows ────────────────────

    #[tokio::test]
    async fn test_move_note_to_project_ok() {
        let backend = make_backend().await;
        let now = chrono::Utc::now().to_rfc3339();
        let note_id = uuid::Uuid::new_v4().to_string();
        let proj_a = backend.create_project("Proj-A").await.unwrap();
        let proj_b = backend.create_project("Proj-B").await.unwrap();

        backend
            .insert_note(&InsertNoteReq {
                id: &note_id,
                note_type: "normal",
                status: "ai_queued",
                title: Some("Test note"),
                content: Some("body"),
                metadata: None,
                project_id: Some(&proj_a),
                now: &now,
            })
            .await
            .unwrap();

        // Move to proj_b — should succeed (not return NoteNotFound)
        let result = backend
            .move_note_to_project(&note_id, &proj_b, Some(&proj_a))
            .await
            .unwrap();
        // This note was the only one in proj_a, so proj_a gets deleted
        assert_eq!(result.as_deref(), Some("Proj-A"));

        // Verify the note is now in proj_b
        let note = backend.find_note(&note_id).await.unwrap();
        assert_eq!(note.project_id.as_deref(), Some(proj_b.as_str()));
    }

    #[tokio::test]
    async fn test_move_note_to_project_missing_returns_err() {
        let backend = make_backend().await;
        let fake_id = uuid::Uuid::new_v4().to_string();
        let proj_a = backend.create_project("Proj-A").await.unwrap();
        let proj_b = backend.create_project("Proj-B").await.unwrap();

        let err = backend
            .move_note_to_project(&fake_id, &proj_b, Some(&proj_a))
            .await
            .unwrap_err();
        match err {
            CliError::NoteNotFound { id } => assert_eq!(id, fake_id),
            _ => panic!("expected NoteNotFound, got {:?}", err),
        }
    }

    #[tokio::test]
    async fn test_move_note_to_project_same_project_noop() {
        let backend = make_backend().await;
        let now = chrono::Utc::now().to_rfc3339();
        let note_id = uuid::Uuid::new_v4().to_string();
        let proj_x = backend.create_project("Proj-X").await.unwrap();

        backend
            .insert_note(&InsertNoteReq {
                id: &note_id,
                note_type: "normal",
                status: "ai_queued",
                title: Some("Same-project note"),
                content: Some("body"),
                metadata: None,
                project_id: Some(&proj_x),
                now: &now,
            })
            .await
            .unwrap();

        // Same source and target — should be idempotent, return Ok(None),
        // not delete the project (it still holds the note).
        let result = backend
            .move_note_to_project(&note_id, &proj_x, Some(&proj_x))
            .await
            .unwrap();
        assert_eq!(result, None, "same-project move should not delete project");

        // Verify project still exists and note is still in it
        let note = backend.find_note(&note_id).await.unwrap();
        assert_eq!(note.project_id.as_deref(), Some(proj_x.as_str()));
        let active = backend.list_projects(false).await.unwrap();
        assert!(
            active.iter().any(|p| p.id == proj_x),
            "project should still exist"
        );
    }

    #[tokio::test]
    async fn test_update_note_title_ok() {
        let backend = make_backend().await;
        let now = chrono::Utc::now().to_rfc3339();
        let note_id = uuid::Uuid::new_v4().to_string();

        backend
            .insert_note(&InsertNoteReq {
                id: &note_id,
                note_type: "normal",
                status: "ai_queued",
                title: Some("Old title"),
                content: Some("body"),
                metadata: None,
                project_id: None,
                now: &now,
            })
            .await
            .unwrap();

        backend
            .update_note_title(&note_id, "New title")
            .await
            .unwrap();
        let note = backend.find_note(&note_id).await.unwrap();
        assert_eq!(note.title, Some("New title".to_string()));
    }

    #[tokio::test]
    async fn test_update_note_flagged_ok() {
        let backend = make_backend().await;
        let now = chrono::Utc::now().to_rfc3339();
        let note_id = uuid::Uuid::new_v4().to_string();

        backend
            .insert_note(&InsertNoteReq {
                id: &note_id,
                note_type: "normal",
                status: "ai_queued",
                title: Some("Flag me"),
                content: Some("body"),
                metadata: None,
                project_id: None,
                now: &now,
            })
            .await
            .unwrap();

        backend.update_note_flagged(&note_id, true).await.unwrap();
        let note = backend.find_note(&note_id).await.unwrap();
        assert_eq!(note.is_flagged, Some(1));

        backend.update_note_flagged(&note_id, false).await.unwrap();
        let note = backend.find_note(&note_id).await.unwrap();
        assert_eq!(note.is_flagged, Some(0));
    }

    #[tokio::test]
    async fn test_delete_project_archives() {
        let backend = make_backend().await;
        let proj_id = backend.create_project("ToDelete").await.unwrap();

        // Verify project exists
        let proj = backend.find_project(&proj_id).await.unwrap();
        assert_eq!(proj.name, "ToDelete");

        backend.delete_project(&proj_id).await.unwrap();

        // After archive, project should not appear in active list
        let active = backend.list_projects(false).await.unwrap();
        assert!(
            !active.iter().any(|p| p.id == proj_id),
            "deleted project should not appear in active list"
        );

        // Archived list should contain it
        let archived = backend.list_projects(true).await.unwrap();
        assert!(
            archived.iter().any(|p| p.id == proj_id),
            "deleted project should appear in archived list"
        );
    }

    #[tokio::test]
    async fn test_delete_project_missing_returns_err() {
        let backend = make_backend().await;
        let fake_id = uuid::Uuid::new_v4().to_string();

        let err = backend.delete_project(&fake_id).await.unwrap_err();
        match err {
            CliError::Other(msg) => assert!(msg.contains("not found"), "got: {msg}"),
            _ => panic!("expected Other error, got {:?}", err),
        }
    }
}
