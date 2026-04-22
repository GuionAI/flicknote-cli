#[cfg(feature = "powersync")]
use rusqlite::{params, types::ToSql};

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

// ─── Shared helpers ──────────────────────────────────────────────────────────

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

pub trait NoteDb {
    fn user_id(&self) -> &str;

    // Note resolution
    fn resolve_note_id(&self, prefix: &str) -> Result<String, CliError>;
    fn resolve_archived_note_id(&self, prefix: &str) -> Result<String, CliError>;

    // Note reads
    fn find_note(&self, id: &str) -> Result<Note, CliError>;
    fn find_archived_note(&self, id: &str) -> Result<Note, CliError>;
    fn find_note_content(&self, id: &str) -> Result<Option<String>, CliError>;
    fn list_notes(&self, filter: &NoteFilter<'_>) -> Result<Vec<Note>, CliError>;
    fn search_notes(
        &self,
        keywords: &[String],
        filter: &NoteFilter<'_>,
    ) -> Result<Vec<Note>, CliError>;

    // Note writes
    fn insert_note(&self, req: &InsertNoteReq<'_>) -> Result<(), CliError>;
    /// Update content. When `requeue` is true, also sets status = 'ai_queued'.
    fn update_note_content(&self, id: &str, content: &str, requeue: bool) -> Result<(), CliError>;
    /// Set deleted_at to the given timestamp, or NULL when `deleted_at` is None.
    /// `now` is used for the `updated_at` column and must match the timestamp
    /// used in the hook payload so subscribers see consistent values.
    fn set_note_deleted_at(
        &self,
        id: &str,
        deleted_at: Option<&str>,
        now: &str,
    ) -> Result<(), CliError>;

    /// Restore the most recently deleted note (sets deleted_at = NULL).
    /// Returns `Ok(())` for both "note restored" and "nothing to undo" — callers
    /// cannot distinguish the two cases.
    fn undo_last_delete(&self) -> Result<(), CliError>;

    // Project reads
    fn find_project_by_name(&self, name: &str) -> Result<Option<String>, CliError>;
    fn find_project_name_by_id(&self, project_id: &str) -> Result<Option<String>, CliError>;
    fn list_projects(&self, archived: bool) -> Result<Vec<Project>, CliError>;
    fn find_project(&self, id: &str) -> Result<Project, CliError>;
    fn resolve_project_id(&self, prefix: &str) -> Result<String, CliError>;

    // Project writes
    fn create_project(&self, name: &str) -> Result<String, CliError>;

    /// Move a note to a different project. Returns the deleted project name if the old
    /// project is now empty. Returns `NoteNotFound` if no such note exists.
    fn move_note_to_project(
        &self,
        note_id: &str,
        new_project_id: &str,
        old_project_id: Option<&str>,
    ) -> Result<Option<String>, CliError>;

    /// Update project metadata. `None` = don't change, `Some(None)` = clear, `Some(Some(v))` = set.
    fn update_project(
        &self,
        id: &str,
        prompt_id: Option<Option<&str>>,
        keyterm_id: Option<Option<&str>>,
        color: Option<Option<&str>>,
    ) -> Result<(), CliError>;

    /// Delete (archive) a project by ID. Returns `ProjectNotFound` if no such project exists.
    fn delete_project(&self, id: &str) -> Result<(), CliError>;

    // Note metadata writes
    /// Update a note's title. Returns `NoteNotFound` if no such note exists.
    fn update_note_title(&self, id: &str, title: &str) -> Result<(), CliError>;
    /// Update a note's flagged status. Returns `NoteNotFound` if no such note exists.
    fn update_note_flagged(&self, id: &str, flagged: bool) -> Result<(), CliError>;

    // Note reads (extended)
    fn count_notes(&self, filter: &NoteFilter<'_>) -> Result<u64, CliError>;
    fn list_note_topics(
        &self,
        note_ids: &[&str],
    ) -> Result<std::collections::HashMap<String, Vec<String>>, CliError>;

    // Prompt operations
    fn resolve_prompt_id(&self, prefix: &str) -> Result<String, CliError>;
    fn insert_prompt(
        &self,
        id: &str,
        title: &str,
        description: Option<&str>,
        prompt: &str,
        now: &str,
    ) -> Result<(), CliError>;
    fn find_prompt(&self, id: &str) -> Result<Prompt, CliError>;
    fn list_prompts(&self) -> Result<Vec<Prompt>, CliError>;
    fn update_prompt(
        &self,
        id: &str,
        title: Option<&str>,
        description: Option<&str>,
        prompt: Option<&str>,
    ) -> Result<(), CliError>;
    fn delete_prompt(&self, id: &str) -> Result<(), CliError>;

    // Keyterm operations
    fn resolve_keyterm_id(&self, prefix: &str) -> Result<String, CliError>;
    fn insert_keyterm(
        &self,
        id: &str,
        name: &str,
        description: Option<&str>,
        content: Option<&str>,
        now: &str,
    ) -> Result<(), CliError>;
    fn find_keyterm(&self, id: &str) -> Result<Keyterm, CliError>;
    fn list_keyterms(&self) -> Result<Vec<Keyterm>, CliError>;
    fn update_keyterm(
        &self,
        id: &str,
        name: Option<&str>,
        description: Option<&str>,
        content: Option<&str>,
    ) -> Result<(), CliError>;
    fn delete_keyterm(&self, id: &str) -> Result<(), CliError>;
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
const SQ_RESOLVE: &str =
    "SELECT id FROM notes WHERE user_id = ? AND id LIKE ? AND deleted_at IS NULL LIMIT 2";
#[cfg(feature = "powersync")]
const SQ_RESOLVE_ARCHIVED: &str =
    "SELECT id FROM notes WHERE user_id = ? AND id LIKE ? AND deleted_at IS NOT NULL LIMIT 2";
#[cfg(feature = "powersync")]
const SQ_FIND: &str = "SELECT id, user_id, type, status, title, content, summary, is_flagged, \
     project_id, metadata, source, external_id, created_at, updated_at, deleted_at \
     FROM notes WHERE user_id = ? AND id = ? AND deleted_at IS NULL LIMIT 1";
#[cfg(feature = "powersync")]
const SQ_FIND_ARCHIVED: &str = "SELECT id, user_id, type, status, title, content, summary, is_flagged, \
     project_id, metadata, source, external_id, created_at, updated_at, deleted_at \
     FROM notes WHERE user_id = ? AND id = ? AND deleted_at IS NOT NULL LIMIT 1";
#[cfg(feature = "powersync")]
const SQ_FIND_CONTENT: &str =
    "SELECT content FROM notes WHERE user_id = ? AND id = ? AND deleted_at IS NULL LIMIT 1";
#[cfg(feature = "powersync")]
const SQ_INSERT: &str = "INSERT INTO notes \
     (id, user_id, type, status, title, content, metadata, project_id, created_at, updated_at) \
     VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)";
#[cfg(feature = "powersync")]
const SQ_UPDATE_CONTENT: &str =
    "UPDATE notes SET content = ?, updated_at = ? WHERE user_id = ? AND id = ?";
#[cfg(feature = "powersync")]
const SQ_UPDATE_CONTENT_REQUEUE: &str = "UPDATE notes SET content = ?, status = 'ai_queued', updated_at = ? WHERE user_id = ? AND id = ?";
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
const SQ_COUNT_NOTES: &str = "SELECT COUNT(*) FROM notes WHERE user_id = ? AND deleted_at IS NULL";
#[cfg(feature = "powersync")]
const SQ_COUNT_NOTES_ARCHIVED: &str =
    "SELECT COUNT(*) FROM notes WHERE user_id = ? AND deleted_at IS NOT NULL";
#[cfg(feature = "powersync")]
const SQ_LIST_TOPICS: &str = "SELECT note_id, value FROM note_extractions WHERE user_id = ? AND type = 'topic' AND note_id IN ";

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
impl NoteDb for SqliteBackend {
    fn user_id(&self) -> &str {
        &self.user_id
    }

    fn resolve_note_id(&self, prefix: &str) -> Result<String, CliError> {
        validate_id_prefix(prefix)?;
        self.db.read(|conn| {
            let mut stmt = conn.prepare(SQ_RESOLVE)?;
            let mut rows = stmt.query(params![self.user_id, format!("{prefix}%")])?;
            let first = rows.next()?.map(|r| r.get::<_, String>(0)).transpose()?;
            let second = rows.next()?.is_some();
            match (first, second) {
                (Some(_), true) => Err(CliError::Other(format!("Ambiguous ID prefix: {prefix}"))),
                (Some(id), false) => Ok(id),
                (None, _) => Err(CliError::NoteNotFound {
                    id: prefix.to_string(),
                }),
            }
        })
    }

    fn resolve_archived_note_id(&self, prefix: &str) -> Result<String, CliError> {
        validate_id_prefix(prefix)?;
        self.db.read(|conn| {
            let mut stmt = conn.prepare(SQ_RESOLVE_ARCHIVED)?;
            let mut rows = stmt.query(params![self.user_id, format!("{prefix}%")])?;
            let first = rows.next()?.map(|r| r.get::<_, String>(0)).transpose()?;
            let second = rows.next()?.is_some();
            match (first, second) {
                (Some(_), true) => Err(CliError::Other(format!("Ambiguous ID prefix: {prefix}"))),
                (Some(id), false) => Ok(id),
                (None, _) => Err(CliError::NoteNotFound {
                    id: prefix.to_string(),
                }),
            }
        })
    }

    fn find_note(&self, id: &str) -> Result<Note, CliError> {
        self.db.read(|conn| {
            let mut stmt = conn.prepare(SQ_FIND)?;
            let mut rows = stmt.query(params![self.user_id, id])?;
            match rows.next()? {
                Some(row) => Ok(Note::from_row(row)?),
                None => Err(CliError::NoteNotFound { id: id.to_string() }),
            }
        })
    }

    fn find_archived_note(&self, id: &str) -> Result<Note, CliError> {
        self.db.read(|conn| {
            let mut stmt = conn.prepare(SQ_FIND_ARCHIVED)?;
            let mut rows = stmt.query(params![self.user_id, id])?;
            match rows.next()? {
                Some(row) => Ok(Note::from_row(row)?),
                None => Err(CliError::NoteNotFound { id: id.to_string() }),
            }
        })
    }

    fn find_note_content(&self, id: &str) -> Result<Option<String>, CliError> {
        self.db.read(|conn| {
            let mut stmt = conn.prepare(SQ_FIND_CONTENT)?;
            let mut rows = stmt.query(params![self.user_id, id])?;
            match rows.next()? {
                Some(row) => Ok(row.get::<_, Option<String>>(0)?),
                None => Err(CliError::NoteNotFound { id: id.to_string() }),
            }
        })
    }

    fn list_notes(&self, filter: &NoteFilter<'_>) -> Result<Vec<Note>, CliError> {
        self.db.read(|conn| {
            let archive_cond = if filter.archived {
                "deleted_at IS NOT NULL"
            } else {
                "deleted_at IS NULL"
            };
            let mut sql = format!("SELECT * FROM notes WHERE user_id = ? AND {archive_cond}");
            let mut params_vec: Vec<Box<dyn ToSql>> = vec![Box::new(self.user_id.clone())];

            if let Some(t) = filter.note_type {
                sql.push_str(" AND type = ?");
                params_vec.push(Box::new(t.to_string()));
            }
            if let Some(pid) = filter.project_id {
                sql.push_str(" AND project_id = ?");
                params_vec.push(Box::new(pid.to_string()));
            }
            sql.push_str(" ORDER BY created_at DESC LIMIT ?");
            params_vec.push(Box::new(filter.limit));

            let mut stmt = conn.prepare(&sql)?;
            let param_refs: Vec<&dyn ToSql> =
                params_vec.iter().map(std::convert::AsRef::as_ref).collect();
            let rows = stmt.query_map(param_refs.as_slice(), Note::from_row)?;
            rows.collect::<Result<Vec<_>, _>>().map_err(CliError::from)
        })
    }

    fn search_notes(
        &self,
        keywords: &[String],
        filter: &NoteFilter<'_>,
    ) -> Result<Vec<Note>, CliError> {
        if keywords.is_empty() {
            return Err(CliError::Other(
                "search_notes requires at least one keyword".into(),
            ));
        }
        let archive_cond = if filter.archived {
            "deleted_at IS NOT NULL"
        } else {
            "deleted_at IS NULL"
        };
        let keyword_blocks: Vec<String> = keywords
            .iter()
            .map(|_| "(title LIKE ? OR content LIKE ? OR summary LIKE ?)".to_string())
            .collect();
        let keywords_clause = keyword_blocks.join(" OR ");
        let mut sql = format!(
            "SELECT * FROM notes WHERE user_id = ? AND {archive_cond} AND ({keywords_clause})"
        );
        if filter.project_id.is_some() {
            sql.push_str(" AND project_id = ?");
        }
        sql.push_str(" ORDER BY updated_at DESC LIMIT ?");

        self.db.read(|conn| {
            let mut params_vec: Vec<Box<dyn ToSql>> = vec![Box::new(self.user_id.clone())];
            for kw in keywords {
                let pattern = format!("%{kw}%");
                params_vec.push(Box::new(pattern.clone()));
                params_vec.push(Box::new(pattern.clone()));
                params_vec.push(Box::new(pattern));
            }
            if let Some(pid) = filter.project_id {
                params_vec.push(Box::new(pid.to_string()));
            }
            params_vec.push(Box::new(filter.limit));

            let mut stmt = conn.prepare(&sql)?;
            let param_refs: Vec<&dyn ToSql> =
                params_vec.iter().map(std::convert::AsRef::as_ref).collect();
            let rows = stmt.query_map(param_refs.as_slice(), Note::from_row)?;
            rows.collect::<Result<Vec<_>, _>>()
                .map_err(|e| CliError::Other(format!("Failed to read note rows: {e}")))
        })
    }

    fn insert_note(&self, req: &InsertNoteReq<'_>) -> Result<(), CliError> {
        self.db.write(|conn| {
            conn.execute(
                SQ_INSERT,
                params![
                    req.id,
                    self.user_id,
                    req.note_type,
                    req.status,
                    req.title,
                    req.content,
                    req.metadata,
                    req.project_id,
                    req.now,
                    req.now
                ],
            )?;
            Ok(())
        })
    }

    fn update_note_content(&self, id: &str, content: &str, requeue: bool) -> Result<(), CliError> {
        let now = chrono::Utc::now().to_rfc3339();
        self.db.write(|conn| {
            if requeue {
                conn.execute(
                    SQ_UPDATE_CONTENT_REQUEUE,
                    params![content, now, self.user_id, id],
                )?;
            } else {
                conn.execute(SQ_UPDATE_CONTENT, params![content, now, self.user_id, id])?;
            }
            Ok(())
        })
    }

    fn set_note_deleted_at(
        &self,
        id: &str,
        deleted_at: Option<&str>,
        now: &str,
    ) -> Result<(), CliError> {
        self.db.write(|conn| {
            if let Some(ts) = deleted_at {
                conn.execute(SQ_SET_DELETED_AT, params![ts, now, self.user_id, id])?;
            } else {
                conn.execute(SQ_SET_DELETED_AT_NULL, params![now, self.user_id, id])?;
            }
            Ok(())
        })
    }

    fn undo_last_delete(&self) -> Result<(), CliError> {
        let now = chrono::Utc::now().to_rfc3339();
        self.db.write(|conn| {
            conn.execute(SQ_UNDO_DELETE, params![&now, &self.user_id])?;
            Ok(())
        })
    }

    fn find_project_by_name(&self, name: &str) -> Result<Option<String>, CliError> {
        self.db.read(|conn| {
            let mut stmt = conn.prepare(SQ_FIND_PROJECT)?;
            let mut rows = stmt.query(params![self.user_id, name])?;
            match rows.next()? {
                Some(row) => Ok(Some(row.get::<_, String>(0)?)),
                None => Ok(None),
            }
        })
    }

    fn find_project_name_by_id(&self, project_id: &str) -> Result<Option<String>, CliError> {
        self.db.read(|conn| {
            let mut stmt = conn.prepare(SQ_FIND_PROJECT_NAME)?;
            let mut rows = stmt.query(params![self.user_id, project_id])?;
            match rows.next()? {
                Some(row) => Ok(Some(row.get::<_, String>(0)?)),
                None => Ok(None),
            }
        })
    }

    fn list_projects(&self, archived: bool) -> Result<Vec<Project>, CliError> {
        let sql = if archived {
            SQ_LIST_PROJECTS_ARCHIVED
        } else {
            SQ_LIST_PROJECTS_ACTIVE
        };
        self.db.read(|conn| {
            let mut stmt = conn.prepare(sql)?;
            let rows = stmt.query_map(params![self.user_id], Project::from_row)?;
            rows.collect::<Result<Vec<_>, _>>().map_err(CliError::from)
        })
    }

    fn create_project(&self, name: &str) -> Result<String, CliError> {
        let id = uuid::Uuid::new_v4().to_string();
        let now = chrono::Utc::now().to_rfc3339();
        self.db.write(|conn| {
            conn.execute(SQ_CREATE_PROJECT, params![id, self.user_id, name, now])?;
            Ok(())
        })?;
        Ok(id)
    }

    fn move_note_to_project(
        &self,
        note_id: &str,
        new_project_id: &str,
        old_project_id: Option<&str>,
    ) -> Result<Option<String>, CliError> {
        self.db.write(|conn| {
            let now = chrono::Utc::now().to_rfc3339();
            // SQLite INSTEAD OF triggers on PowerSync views cause `execute` to
            // return 0 even on successful updates. Use a pre-SELECT to verify
            // the note exists first.
            let exists = conn
                .prepare("SELECT 1 FROM notes WHERE user_id = ? AND id = ? AND deleted_at IS NULL LIMIT 1")?
                .query(params![self.user_id, note_id])?
                .next()?
                .is_some();
            if !exists {
                return Err(CliError::NoteNotFound {
                    id: note_id.to_string(),
                });
            }
            conn.execute(
                SQ_UPDATE_PROJECT,
                params![new_project_id, now, self.user_id, note_id],
            )?;

            let Some(old_pid) = old_project_id else {
                return Ok(None);
            };

            let count: i64 = conn
                .prepare(SQ_COUNT_PROJECT_NOTES)?
                .query_row(params![self.user_id, old_pid], |r| r.get(0))?;

            if count == 0 {
                let old_name: Option<String> = match conn
                    .prepare(SQ_FIND_PROJECT_NAME)?
                    .query_row(params![self.user_id, old_pid], |r| r.get::<_, String>(0))
                {
                    Ok(name) => Some(name),
                    Err(rusqlite::Error::QueryReturnedNoRows) => None,
                    Err(e) => return Err(CliError::from(e)),
                };
                conn.execute(SQ_DELETE_PROJECT, params![self.user_id, old_pid])?;
                Ok(old_name)
            } else {
                Ok(None)
            }
        })
    }

    fn find_project(&self, id: &str) -> Result<Project, CliError> {
        self.db.read(|conn| {
            let mut stmt = conn.prepare(SQ_FIND_PROJECT_BY_ID)?;
            let mut rows = stmt.query(params![self.user_id, id])?;
            match rows.next()? {
                Some(row) => Ok(Project::from_row(row)?),
                None => Err(CliError::Other(format!("Project not found: {id}"))),
            }
        })
    }

    fn resolve_project_id(&self, prefix: &str) -> Result<String, CliError> {
        validate_id_prefix(prefix)?;
        self.db.read(|conn| {
            let mut stmt = conn.prepare(SQ_RESOLVE_PROJECT)?;
            let mut rows = stmt.query(params![self.user_id, format!("{prefix}%")])?;
            let first = rows.next()?.map(|r| r.get::<_, String>(0)).transpose()?;
            let second = rows.next()?.is_some();
            match (first, second) {
                (Some(_), true) => Err(CliError::Other(format!(
                    "Ambiguous project ID prefix: {prefix}"
                ))),
                (Some(id), false) => Ok(id),
                (None, _) => Err(CliError::Other(format!("Project not found: {prefix}"))),
            }
        })
    }

    fn update_project(
        &self,
        id: &str,
        prompt_id: Option<Option<&str>>,
        keyterm_id: Option<Option<&str>>,
        color: Option<Option<&str>>,
    ) -> Result<(), CliError> {
        // All UPDATEs run inside a single write closure; PowerSync wraps each
        // write call in an implicit transaction, so this is atomic (#4).
        self.db.write(|conn| {
            if let Some(val) = prompt_id {
                if let Some(v) = val {
                    conn.execute(
                        "UPDATE projects SET prompt_id = ? WHERE user_id = ? AND id = ?",
                        params![v, self.user_id, id],
                    )?;
                } else {
                    conn.execute(
                        "UPDATE projects SET prompt_id = NULL WHERE user_id = ? AND id = ?",
                        params![self.user_id, id],
                    )?;
                }
            }
            if let Some(val) = keyterm_id {
                if let Some(v) = val {
                    conn.execute(
                        "UPDATE projects SET keyterm_id = ? WHERE user_id = ? AND id = ?",
                        params![v, self.user_id, id],
                    )?;
                } else {
                    conn.execute(
                        "UPDATE projects SET keyterm_id = NULL WHERE user_id = ? AND id = ?",
                        params![self.user_id, id],
                    )?;
                }
            }
            if let Some(val) = color {
                if let Some(v) = val {
                    conn.execute(
                        "UPDATE projects SET color = ? WHERE user_id = ? AND id = ?",
                        params![v, self.user_id, id],
                    )?;
                } else {
                    conn.execute(
                        "UPDATE projects SET color = NULL WHERE user_id = ? AND id = ?",
                        params![self.user_id, id],
                    )?;
                }
            }
            Ok(())
        })
    }

    fn delete_project(&self, id: &str) -> Result<(), CliError> {
        self.db.write(|conn| {
            // SQLite INSTEAD OF triggers on PowerSync views cause `execute` to
            // return 0 even on successful updates. Use a pre-SELECT.
            let exists = conn
                .prepare("SELECT 1 FROM projects WHERE user_id = ? AND id = ? LIMIT 1")?
                .query(params![self.user_id, id])?
                .next()?
                .is_some();
            if !exists {
                return Err(CliError::Other(format!("Project not found: {id}")));
            }
            conn.execute(SQ_ARCHIVE_PROJECT, params![self.user_id, id])?;
            Ok(())
        })
    }

    fn update_note_title(&self, id: &str, title: &str) -> Result<(), CliError> {
        let now = chrono::Utc::now().to_rfc3339();
        self.db.write(|conn| {
            let exists = conn
                .prepare("SELECT 1 FROM notes WHERE user_id = ? AND id = ? AND deleted_at IS NULL LIMIT 1")?
                .query(params![self.user_id, id])?
                .next()?
                .is_some();
            if !exists {
                return Err(CliError::NoteNotFound { id: id.to_string() });
            }
            conn.execute(SQ_UPDATE_TITLE, params![title, now, self.user_id, id])?;
            Ok(())
        })
    }

    fn update_note_flagged(&self, id: &str, flagged: bool) -> Result<(), CliError> {
        let now = chrono::Utc::now().to_rfc3339();
        let val: i64 = if flagged { 1 } else { 0 };
        self.db.write(|conn| {
            let exists = conn
                .prepare("SELECT 1 FROM notes WHERE user_id = ? AND id = ? AND deleted_at IS NULL LIMIT 1")?
                .query(params![self.user_id, id])?
                .next()?
                .is_some();
            if !exists {
                return Err(CliError::NoteNotFound { id: id.to_string() });
            }
            conn.execute(SQ_UPDATE_FLAGGED, params![val, now, self.user_id, id])?;
            Ok(())
        })
    }

    fn count_notes(&self, filter: &NoteFilter<'_>) -> Result<u64, CliError> {
        let base_sql = if filter.archived {
            SQ_COUNT_NOTES_ARCHIVED
        } else {
            SQ_COUNT_NOTES
        };
        let mut sql = base_sql.to_string();
        let mut params_vec: Vec<Box<dyn ToSql>> = vec![Box::new(self.user_id.clone())];

        if let Some(t) = filter.note_type {
            sql.push_str(" AND type = ?");
            params_vec.push(Box::new(t.to_string()));
        }
        if let Some(pid) = filter.project_id {
            sql.push_str(" AND project_id = ?");
            params_vec.push(Box::new(pid.to_string()));
        }

        self.db.read(|conn| {
            let param_refs: Vec<&dyn ToSql> =
                params_vec.iter().map(std::convert::AsRef::as_ref).collect();
            let count: i64 = conn.query_row(&sql, param_refs.as_slice(), |r| r.get(0))?;
            count
                .try_into()
                .map_err(|_| CliError::Other(format!("unexpected negative count: {count}")))
        })
    }

    fn list_note_topics(
        &self,
        note_ids: &[&str],
    ) -> Result<std::collections::HashMap<String, Vec<String>>, CliError> {
        if note_ids.is_empty() {
            return Ok(std::collections::HashMap::new());
        }
        let placeholders = note_ids.iter().map(|_| "?").collect::<Vec<_>>().join(", ");
        let sql = format!("{SQ_LIST_TOPICS}({placeholders})");
        self.db.read(|conn| {
            let mut params_vec: Vec<Box<dyn ToSql>> = vec![Box::new(self.user_id.clone())];
            for id in note_ids {
                params_vec.push(Box::new(id.to_string()));
            }
            let param_refs: Vec<&dyn ToSql> =
                params_vec.iter().map(std::convert::AsRef::as_ref).collect();
            let mut stmt = conn.prepare(&sql)?;
            let mut map: std::collections::HashMap<String, Vec<String>> =
                std::collections::HashMap::new();
            let rows = stmt.query_map(param_refs.as_slice(), |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?;
            for row in rows {
                let (note_id, value) = row?;
                map.entry(note_id).or_default().push(value);
            }
            Ok(map)
        })
    }

    fn resolve_prompt_id(&self, prefix: &str) -> Result<String, CliError> {
        validate_id_prefix(prefix)?;
        self.db.read(|conn| {
            let mut stmt = conn.prepare(SQ_RESOLVE_PROMPT)?;
            let mut rows = stmt.query(params![self.user_id, format!("{prefix}%")])?;
            let first = rows.next()?.map(|r| r.get::<_, String>(0)).transpose()?;
            let second = rows.next()?.is_some();
            match (first, second) {
                (Some(_), true) => Err(CliError::Other(format!(
                    "Ambiguous prompt ID prefix: {prefix}"
                ))),
                (Some(id), false) => Ok(id),
                (None, _) => Err(CliError::Other(format!("Prompt not found: {prefix}"))),
            }
        })
    }

    fn insert_prompt(
        &self,
        id: &str,
        title: &str,
        description: Option<&str>,
        prompt: &str,
        now: &str,
    ) -> Result<(), CliError> {
        self.db.write(|conn| {
            conn.execute(
                SQ_INSERT_PROMPT,
                params![id, self.user_id, title, description, prompt, now],
            )?;
            Ok(())
        })
    }

    fn find_prompt(&self, id: &str) -> Result<Prompt, CliError> {
        self.db.read(|conn| {
            let mut stmt = conn.prepare(SQ_FIND_PROMPT)?;
            let mut rows = stmt.query(params![self.user_id, id])?;
            match rows.next()? {
                Some(row) => Ok(Prompt::from_row(row)?),
                None => Err(CliError::Other(format!("Prompt not found: {id}"))),
            }
        })
    }

    fn list_prompts(&self) -> Result<Vec<Prompt>, CliError> {
        self.db.read(|conn| {
            let mut stmt = conn.prepare(SQ_LIST_PROMPTS)?;
            let rows = stmt.query_map(params![self.user_id], Prompt::from_row)?;
            rows.collect::<Result<Vec<_>, _>>().map_err(CliError::from)
        })
    }

    fn update_prompt(
        &self,
        id: &str,
        title: Option<&str>,
        description: Option<&str>,
        prompt: Option<&str>,
    ) -> Result<(), CliError> {
        self.db.write(|conn| {
            if let Some(v) = title {
                conn.execute(
                    "UPDATE prompts SET title = ? WHERE user_id = ? AND id = ?",
                    params![v, self.user_id, id],
                )?;
            }
            if let Some(v) = description {
                conn.execute(
                    "UPDATE prompts SET description = ? WHERE user_id = ? AND id = ?",
                    params![v, self.user_id, id],
                )?;
            }
            if let Some(v) = prompt {
                conn.execute(
                    "UPDATE prompts SET prompt = ? WHERE user_id = ? AND id = ?",
                    params![v, self.user_id, id],
                )?;
            }
            Ok(())
        })
    }

    fn delete_prompt(&self, id: &str) -> Result<(), CliError> {
        self.db.write(|conn| {
            conn.execute(SQ_DELETE_PROMPT, params![self.user_id, id])?;
            Ok(())
        })
    }

    fn resolve_keyterm_id(&self, prefix: &str) -> Result<String, CliError> {
        validate_id_prefix(prefix)?;
        self.db.read(|conn| {
            let mut stmt = conn.prepare(SQ_RESOLVE_KEYTERM)?;
            let mut rows = stmt.query(params![self.user_id, format!("{prefix}%")])?;
            let first = rows.next()?.map(|r| r.get::<_, String>(0)).transpose()?;
            let second = rows.next()?.is_some();
            match (first, second) {
                (Some(_), true) => Err(CliError::Other(format!(
                    "Ambiguous keyterm ID prefix: {prefix}"
                ))),
                (Some(id), false) => Ok(id),
                (None, _) => Err(CliError::Other(format!("Keyterm not found: {prefix}"))),
            }
        })
    }

    fn insert_keyterm(
        &self,
        id: &str,
        name: &str,
        description: Option<&str>,
        content: Option<&str>,
        now: &str,
    ) -> Result<(), CliError> {
        self.db.write(|conn| {
            conn.execute(
                SQ_INSERT_KEYTERM,
                params![id, self.user_id, name, description, content, now, now],
            )?;
            Ok(())
        })
    }

    fn find_keyterm(&self, id: &str) -> Result<Keyterm, CliError> {
        self.db.read(|conn| {
            let mut stmt = conn.prepare(SQ_FIND_KEYTERM)?;
            let mut rows = stmt.query(params![self.user_id, id])?;
            match rows.next()? {
                Some(row) => Ok(Keyterm::from_row(row)?),
                None => Err(CliError::Other(format!("Keyterm not found: {id}"))),
            }
        })
    }

    fn list_keyterms(&self) -> Result<Vec<Keyterm>, CliError> {
        self.db.read(|conn| {
            let mut stmt = conn.prepare(SQ_LIST_KEYTERMS)?;
            let rows = stmt.query_map(params![self.user_id], Keyterm::from_row)?;
            rows.collect::<Result<Vec<_>, _>>().map_err(CliError::from)
        })
    }

    fn update_keyterm(
        &self,
        id: &str,
        name: Option<&str>,
        description: Option<&str>,
        content: Option<&str>,
    ) -> Result<(), CliError> {
        let now = chrono::Utc::now().to_rfc3339();
        self.db.write(|conn| {
            if let Some(v) = name {
                conn.execute("UPDATE keyterms SET name = ?, updated_at = ? WHERE user_id = ? AND id = ?", params![v, now, self.user_id, id])?;
            }
            if let Some(v) = description {
                conn.execute("UPDATE keyterms SET description = ?, updated_at = ? WHERE user_id = ? AND id = ?", params![v, now, self.user_id, id])?;
            }
            if let Some(v) = content {
                conn.execute("UPDATE keyterms SET content = ?, updated_at = ? WHERE user_id = ? AND id = ?", params![v, now, self.user_id, id])?;
            }
            Ok(())
        })
    }

    fn delete_keyterm(&self, id: &str) -> Result<(), CliError> {
        self.db.write(|conn| {
            conn.execute(SQ_DELETE_KEYTERM, params![self.user_id, id])?;
            Ok(())
        })
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
#[cfg(feature = "powersync")]
mod tests {
    use super::*;

    fn make_backend() -> SqliteBackend {
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

        let db = Database::open_local(&config).unwrap();
        let user_id = "test-user-id".to_string();

        // Keep dir alive by leaking it — acceptable in tests
        std::mem::forget(dir);

        SqliteBackend { db, user_id }
    }

    #[test]
    fn test_sqlite_backend_insert_and_find() {
        let backend = make_backend();
        let id = uuid::Uuid::new_v4().to_string();
        let now = chrono::Utc::now().to_rfc3339();

        backend
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
            .unwrap();

        // Find by full id
        let note = backend.find_note(&id).unwrap();
        assert_eq!(note.id, id);
        assert_eq!(note.title, Some("Hello world".to_string()));

        // Find by prefix
        let prefix = &id[..8];
        let resolved = backend.resolve_note_id(prefix).unwrap();
        assert_eq!(resolved, id);

        // Find content
        let content = backend.find_note_content(&id).unwrap();
        assert_eq!(content, Some("# Hello world\n\nContent here.".to_string()));
    }

    #[test]
    fn test_sqlite_backend_list_filter() {
        let backend = make_backend();
        let now = chrono::Utc::now().to_rfc3339();

        // Create two projects
        let proj_a = backend.create_project("Project A").unwrap();
        let proj_b = backend.create_project("Project B").unwrap();

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
            .unwrap();

        // List by project A
        let notes = backend
            .list_notes(&NoteFilter {
                project_id: Some(&proj_a),
                note_type: None,
                archived: false,
                limit: 20,
            })
            .unwrap();
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].id, id_a);
    }

    #[test]
    fn test_sqlite_backend_search_notes() {
        let backend = make_backend();
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
            .unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, id);

        // Empty keywords should return Err
        let err = backend.search_notes(
            &[],
            &NoteFilter {
                project_id: None,
                note_type: None,
                archived: false,
                limit: 20,
            },
        );
        assert!(err.is_err());
    }

    #[test]
    fn test_sqlite_backend_archive() {
        let backend = make_backend();
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
            .unwrap();

        // Verify it appears in active list
        let active = backend
            .list_notes(&NoteFilter {
                project_id: None,
                note_type: None,
                archived: false,
                limit: 20,
            })
            .unwrap();
        assert!(active.iter().any(|n| n.id == id));

        // Archive it
        backend.set_note_deleted_at(&id, Some(&now), &now).unwrap();

        // Should be gone from active
        let active_after = backend
            .list_notes(&NoteFilter {
                project_id: None,
                note_type: None,
                archived: false,
                limit: 20,
            })
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
            .unwrap();
        assert!(archived.iter().any(|n| n.id == id));

        // Unarchive
        backend.set_note_deleted_at(&id, None, &now).unwrap();
        let active_restored = backend
            .list_notes(&NoteFilter {
                project_id: None,
                note_type: None,
                archived: false,
                limit: 20,
            })
            .unwrap();
        assert!(active_restored.iter().any(|n| n.id == id));
    }

    #[test]
    fn test_find_archived_note() {
        let backend = make_backend();
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
            .unwrap();

        // Not findable via find_archived_note before archiving
        assert!(backend.find_archived_note(&id).is_err());

        // Archive it
        backend.set_note_deleted_at(&id, Some(&now), &now).unwrap();

        // Now findable via find_archived_note
        let note = backend.find_archived_note(&id).unwrap();
        assert_eq!(note.id, id);
        assert_eq!(note.title, Some("Archived note".to_string()));
        assert!(note.deleted_at.is_some());

        // No longer findable via find_note (active-only)
        assert!(backend.find_note(&id).is_err());
    }

    // ─── Fix: PowerSync view-UPDATE zero affected rows ────────────────────

    #[test]
    fn test_move_note_to_project_ok() {
        let backend = make_backend();
        let now = chrono::Utc::now().to_rfc3339();
        let note_id = uuid::Uuid::new_v4().to_string();
        let proj_a = backend.create_project("Proj-A").unwrap();
        let proj_b = backend.create_project("Proj-B").unwrap();

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
            .unwrap();

        // Move to proj_b — should succeed (not return NoteNotFound)
        let result = backend
            .move_note_to_project(&note_id, &proj_b, Some(&proj_a))
            .unwrap();
        // This note was the only one in proj_a, so proj_a gets deleted
        assert_eq!(result.as_deref(), Some("Proj-A"));

        // Verify the note is now in proj_b
        let note = backend.find_note(&note_id).unwrap();
        assert_eq!(note.project_id.as_deref(), Some(proj_b.as_str()));
    }

    #[test]
    fn test_move_note_to_project_missing_returns_err() {
        let backend = make_backend();
        let fake_id = uuid::Uuid::new_v4().to_string();
        let proj_a = backend.create_project("Proj-A").unwrap();
        let proj_b = backend.create_project("Proj-B").unwrap();

        let err = backend
            .move_note_to_project(&fake_id, &proj_b, Some(&proj_a))
            .unwrap_err();
        match err {
            CliError::NoteNotFound { id } => assert_eq!(id, fake_id),
            _ => panic!("expected NoteNotFound, got {:?}", err),
        }
    }

    #[test]
    fn test_move_note_to_project_same_project_noop() {
        let backend = make_backend();
        let now = chrono::Utc::now().to_rfc3339();
        let note_id = uuid::Uuid::new_v4().to_string();
        let proj_x = backend.create_project("Proj-X").unwrap();

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
            .unwrap();

        // Same source and target — should be idempotent, return Ok(None),
        // not delete the project (it still holds the note).
        let result = backend
            .move_note_to_project(&note_id, &proj_x, Some(&proj_x))
            .unwrap();
        assert_eq!(result, None, "same-project move should not delete project");

        // Verify project still exists and note is still in it
        let note = backend.find_note(&note_id).unwrap();
        assert_eq!(note.project_id.as_deref(), Some(proj_x.as_str()));
        let active = backend.list_projects(false).unwrap();
        assert!(
            active.iter().any(|p| p.id == proj_x),
            "project should still exist"
        );
    }

    #[test]
    fn test_update_note_title_ok() {
        let backend = make_backend();
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
            .unwrap();

        backend.update_note_title(&note_id, "New title").unwrap();
        let note = backend.find_note(&note_id).unwrap();
        assert_eq!(note.title, Some("New title".to_string()));
    }

    #[test]
    fn test_update_note_flagged_ok() {
        let backend = make_backend();
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
            .unwrap();

        backend.update_note_flagged(&note_id, true).unwrap();
        let note = backend.find_note(&note_id).unwrap();
        assert_eq!(note.is_flagged, Some(1));

        backend.update_note_flagged(&note_id, false).unwrap();
        let note = backend.find_note(&note_id).unwrap();
        assert_eq!(note.is_flagged, Some(0));
    }

    #[test]
    fn test_delete_project_archives() {
        let backend = make_backend();
        let proj_id = backend.create_project("ToDelete").unwrap();

        // Verify project exists
        let proj = backend.find_project(&proj_id).unwrap();
        assert_eq!(proj.name, "ToDelete");

        backend.delete_project(&proj_id).unwrap();

        // After archive, project should not appear in active list
        let active = backend.list_projects(false).unwrap();
        assert!(
            !active.iter().any(|p| p.id == proj_id),
            "deleted project should not appear in active list"
        );

        // Archived list should contain it
        let archived = backend.list_projects(true).unwrap();
        assert!(
            archived.iter().any(|p| p.id == proj_id),
            "deleted project should appear in archived list"
        );
    }

    #[test]
    fn test_delete_project_missing_returns_err() {
        let backend = make_backend();
        let fake_id = uuid::Uuid::new_v4().to_string();

        let err = backend.delete_project(&fake_id).unwrap_err();
        match err {
            CliError::Other(msg) => assert!(msg.contains("not found"), "got: {msg}"),
            _ => panic!("expected Other error, got {:?}", err),
        }
    }
}
