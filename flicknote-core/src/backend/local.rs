use async_trait::async_trait;
use powersync::PowerSyncDatabase;
use rusqlite::{Connection, OptionalExtension, Params, Row, params};

use crate::TOPIC_EXTRACTION_KEY;
use crate::error::CliError;
use crate::types::{Note, Project};

use super::{
    InsertNoteReq, InsertedNote, NoteDb, NoteFilter, NoteLookup, NoteSearch, parse_note_lookup,
};

// ─── LocalPowerSyncBackend ───────────────────────────────────────────────────

pub struct LocalPowerSyncBackend {
    db: PowerSyncDatabase,
    user_id: String,
}
impl LocalPowerSyncBackend {
    pub fn new(db: PowerSyncDatabase, user_id: String) -> Self {
        Self { db, user_id }
    }

    #[cfg(test)]
    pub(crate) fn database(&self) -> &PowerSyncDatabase {
        &self.db
    }
}

// SQLite SQL constants — all scope by user_id.
// id column is TEXT in SQLite schema, so LIKE works directly.

const SQ_RESOLVE_UUID: &str =
    "SELECT id FROM notes WHERE user_id = ? AND id = ? AND deleted_at IS NULL LIMIT 1";
const SQ_RESOLVE_SHORT_ID: &str =
    "SELECT id FROM notes WHERE user_id = ? AND short_id = ? AND deleted_at IS NULL LIMIT 1";
const SQ_RESOLVE_ARCHIVED_UUID: &str =
    "SELECT id FROM notes WHERE user_id = ? AND id = ? AND deleted_at IS NOT NULL LIMIT 1";
const SQ_RESOLVE_ARCHIVED_SHORT_ID: &str =
    "SELECT id FROM notes WHERE user_id = ? AND short_id = ? AND deleted_at IS NOT NULL LIMIT 1";
const SQ_FIND: &str = "SELECT id, short_id, user_id, type, status, title, content, summary, is_flagged, \
     project_id, metadata, source, created_at, updated_at, deleted_at \
     FROM notes WHERE user_id = ? AND id = ? AND deleted_at IS NULL LIMIT 1";
const SQ_FIND_ARCHIVED: &str = "SELECT id, short_id, user_id, type, status, title, content, summary, is_flagged, \
     project_id, metadata, source, created_at, updated_at, deleted_at \
     FROM notes WHERE user_id = ? AND id = ? AND deleted_at IS NOT NULL LIMIT 1";
const SQ_FIND_CONTENT: &str =
    "SELECT content FROM notes WHERE user_id = ? AND id = ? AND deleted_at IS NULL LIMIT 1";
const SQ_INSERT: &str = "INSERT INTO notes \
     (id, user_id, type, status, title, content, metadata, project_id, created_at, updated_at) \
     VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)";
const SQ_UPDATE_CONTENT: &str = "UPDATE notes \
     SET content = ?, status = CASE WHEN ? THEN 'ai_queued' ELSE status END, updated_at = ? \
     WHERE user_id = ? AND id = ?";
const SQ_SET_DELETED_AT: &str =
    "UPDATE notes SET deleted_at = ?, updated_at = ? WHERE user_id = ? AND id = ?";
const SQ_SET_DELETED_AT_NULL: &str =
    "UPDATE notes SET deleted_at = NULL, updated_at = ? WHERE user_id = ? AND id = ?";
const SQ_UPDATE_PROJECT: &str =
    "UPDATE notes SET project_id = ?, updated_at = ? WHERE user_id = ? AND id = ?";

const SQ_FIND_PROJECT: &str = "SELECT id FROM projects WHERE user_id = ? AND name = ? \
     AND (is_archived = 0 OR is_archived IS NULL) LIMIT 1";
const SQ_FIND_PROJECT_NAME: &str = "SELECT name FROM projects WHERE user_id = ? AND id = ? LIMIT 1";
const SQ_LIST_PROJECTS_ACTIVE: &str = "SELECT id, user_id, name, color, is_archived, created_at FROM projects \
     WHERE user_id = ? AND (is_archived = 0 OR is_archived IS NULL) ORDER BY name";
const SQ_LIST_PROJECTS_ARCHIVED: &str = "SELECT id, user_id, name, color, is_archived, created_at FROM projects \
     WHERE user_id = ? AND is_archived = 1 ORDER BY name";
const SQ_CREATE_PROJECT: &str =
    "INSERT INTO projects (id, user_id, name, is_archived, created_at) VALUES (?, ?, ?, 0, ?)";
const SQ_COUNT_PROJECT_NOTES: &str =
    "SELECT COUNT(*) FROM notes WHERE user_id = ? AND project_id = ? AND deleted_at IS NULL";
const SQ_DELETE_PROJECT: &str = "DELETE FROM projects WHERE user_id = ? AND id = ?";

const SQ_UNDO_DELETE: &str = "UPDATE notes SET deleted_at = NULL, updated_at = ? \
     WHERE id = (SELECT id FROM notes WHERE deleted_at IS NOT NULL AND user_id = ? \
     ORDER BY deleted_at DESC LIMIT 1)";

const SQ_UPDATE_TITLE: &str =
    "UPDATE notes SET title = ?, updated_at = ? WHERE user_id = ? AND id = ?";
const SQ_UPDATE_FLAGGED: &str =
    "UPDATE notes SET is_flagged = ?, updated_at = ? WHERE user_id = ? AND id = ?";
const SQ_LIST_EXTRACTIONS: &str = "SELECT note_id, key, value FROM note_extractions \
     WHERE user_id = ? AND key IN (SELECT value FROM json_each(?)) \
     AND note_id IN (SELECT value FROM json_each(?)) \
     ORDER BY key, value";
const SQ_LIST_EXTRACTION_VALUES: &str = "SELECT DISTINCT e.value FROM note_extractions e \
     JOIN notes n ON n.id = e.note_id AND n.user_id = e.user_id \
     WHERE e.user_id = ? AND e.key IN (SELECT value FROM json_each(?)) \
     AND (n.deleted_at IS NOT NULL) = ? \
     ORDER BY e.value";
const SQ_CLEAR_EXTRACTIONS: &str = "DELETE FROM note_extractions \
     WHERE user_id = ? AND note_id = ? AND key = ?";
// PowerSync managed tables expose an implicit text `id` column for row identity.
// We write it so extraction rows sync, but reads/deletes use the domain key.
const SQ_INSERT_EXTRACTION: &str =
    "INSERT INTO note_extractions (id, note_id, user_id, key, value) VALUES (?, ?, ?, ?, ?)";

const SQ_FIND_PROJECT_BY_ID: &str = "SELECT id, user_id, name, color, is_archived, created_at FROM projects WHERE user_id = ? AND id = ? LIMIT 1";
const SQ_RESOLVE_PROJECT: &str = "SELECT id FROM projects WHERE user_id = ? AND id = ? LIMIT 1";
const SQ_ARCHIVE_PROJECT: &str = "UPDATE projects SET is_archived = 1 WHERE user_id = ? AND id = ?";
async fn resolve_sqlite_uuid_id(
    db: &PowerSyncDatabase,
    sql: &str,
    user_id: &str,
    input: &str,
    missing: impl FnOnce() -> CliError,
) -> Result<String, CliError> {
    if uuid::Uuid::parse_str(input).is_err() {
        return Err(missing());
    }
    let reader = db.reader().await?;
    let mut statement = reader.prepare(sql)?;
    let rows = statement
        .query_map(params![user_id, input], |row| row.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;

    match rows.as_slice() {
        [id] => Ok(id.clone()),
        [] => Err(missing()),
        [_, _, ..] => unreachable!("exact UUID lookup returns at most one row"),
    }
}

async fn resolve_sqlite_note_id(
    db: &PowerSyncDatabase,
    user_id: &str,
    input: &str,
    uuid_sql: &str,
    short_id_sql: &str,
) -> Result<String, CliError> {
    match parse_note_lookup(input)? {
        NoteLookup::ShortId(short_id) => {
            let reader = db.reader().await?;
            if let Some(id) = reader
                .query_row(short_id_sql, params![user_id, short_id], |row| {
                    row.get::<_, String>(0)
                })
                .optional()?
            {
                return Ok(id);
            }
            Err(CliError::NoteNotFound {
                id: input.to_string(),
            })
        }
        NoteLookup::Uuid(uuid) => {
            let reader = db.reader().await?;
            reader
                .query_row(uuid_sql, params![user_id, uuid], |row| {
                    row.get::<_, String>(0)
                })
                .optional()?
                .ok_or_else(|| CliError::NoteNotFound {
                    id: input.to_string(),
                })
        }
    }
}

async fn sqlite_exists(
    db: &PowerSyncDatabase,
    sql: &str,
    user_id: &str,
    id: &str,
) -> Result<bool, CliError> {
    let reader = db.reader().await?;
    let exists = reader
        .query_row(sql, params![user_id, id], |row| row.get::<_, i64>(0))
        .optional()?;
    Ok(exists.is_some())
}

fn decode_note(row: &Row<'_>) -> rusqlite::Result<Note> {
    Ok(Note {
        id: row.get("id")?,
        short_id: row.get("short_id")?,
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
        created_at: row.get("created_at")?,
        updated_at: row.get("updated_at")?,
        deleted_at: row.get("deleted_at")?,
    })
}

fn decode_project(row: &Row<'_>) -> rusqlite::Result<Project> {
    Ok(Project {
        id: row.get("id")?,
        user_id: row.get("user_id")?,
        name: row.get("name")?,
        color: row.get("color")?,
        is_archived: row.get("is_archived")?,
        created_at: row.get("created_at")?,
    })
}

fn query_notes(
    connection: &Connection,
    sql: &str,
    parameters: impl Params,
) -> Result<Vec<Note>, CliError> {
    let mut statement = connection.prepare(sql)?;
    Ok(statement
        .query_map(parameters, decode_note)?
        .collect::<Result<Vec<_>, _>>()?)
}

fn query_projects(
    connection: &Connection,
    sql: &str,
    parameters: impl Params,
) -> Result<Vec<Project>, CliError> {
    let mut statement = connection.prepare(sql)?;
    Ok(statement
        .query_map(parameters, decode_project)?
        .collect::<Result<Vec<_>, _>>()?)
}
#[async_trait]
impl NoteDb for LocalPowerSyncBackend {
    fn user_id(&self) -> &str {
        &self.user_id
    }

    async fn resolve_note_id(&self, prefix: &str) -> Result<String, CliError> {
        resolve_sqlite_note_id(
            &self.db,
            &self.user_id,
            prefix,
            SQ_RESOLVE_UUID,
            SQ_RESOLVE_SHORT_ID,
        )
        .await
    }

    async fn resolve_archived_note_id(&self, prefix: &str) -> Result<String, CliError> {
        resolve_sqlite_note_id(
            &self.db,
            &self.user_id,
            prefix,
            SQ_RESOLVE_ARCHIVED_UUID,
            SQ_RESOLVE_ARCHIVED_SHORT_ID,
        )
        .await
    }

    async fn find_note(&self, id: &str) -> Result<Note, CliError> {
        let reader = self.db.reader().await?;
        reader
            .query_row(SQ_FIND, params![self.user_id, id], decode_note)
            .optional()?
            .ok_or_else(|| CliError::NoteNotFound { id: id.to_string() })
    }

    async fn find_archived_note(&self, id: &str) -> Result<Note, CliError> {
        let reader = self.db.reader().await?;
        reader
            .query_row(SQ_FIND_ARCHIVED, params![self.user_id, id], decode_note)
            .optional()?
            .ok_or_else(|| CliError::NoteNotFound { id: id.to_string() })
    }

    async fn find_note_content(&self, id: &str) -> Result<Option<String>, CliError> {
        let reader = self.db.reader().await?;
        reader
            .query_row(SQ_FIND_CONTENT, params![self.user_id, id], |row| row.get(0))
            .optional()?
            .ok_or_else(|| CliError::NoteNotFound { id: id.to_string() })
    }

    async fn list_notes(&self, filter: &NoteFilter<'_>) -> Result<Vec<Note>, CliError> {
        let limit = i64::from(filter.limit);
        let reader = self.db.reader().await?;
        query_notes(
            &reader,
            r#"
            SELECT
                id,
                short_id,
                user_id,
                type,
                status,
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
            params![
                self.user_id,
                filter.archived,
                filter.note_type,
                filter.note_type,
                filter.project_id,
                filter.project_id,
                limit,
            ],
        )
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
        let reader = self.db.reader().await?;
        query_notes(
            &reader,
            r#"
            SELECT
                id,
                short_id,
                user_id,
                type,
                status,
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
            params![
                self.user_id,
                filter.archived,
                filter.note_type,
                filter.note_type,
                filter.project_id,
                filter.project_id,
                keywords_json,
                limit,
            ],
        )
    }

    async fn search_notes_structured(
        &self,
        search: &NoteSearch,
        filter: &NoteFilter<'_>,
    ) -> Result<Vec<Note>, CliError> {
        if search.keywords.is_empty() && search.extractions.is_empty() {
            return Err(CliError::Other(
                "search_notes_structured requires at least one keyword or structured filter".into(),
            ));
        }
        let limit = i64::from(filter.limit);
        let keywords_json = serde_json::to_string(&search.keywords)?;
        let extractions_json = serde_json::to_string(
            &search
                .extractions
                .iter()
                .map(|filter| {
                    serde_json::json!({
                        "key": filter.key,
                        "value": filter.value,
                    })
                })
                .collect::<Vec<_>>(),
        )?;
        let reader = self.db.reader().await?;
        query_notes(
            &reader,
            r#"
            SELECT
                id,
                short_id,
                user_id,
                type,
                status,
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
              AND (
                json_array_length(?) = 0 OR EXISTS (
                  SELECT 1 FROM json_each(?) AS kw
                  WHERE title LIKE '%' || kw.value || '%'
                     OR content LIKE '%' || kw.value || '%'
                     OR summary LIKE '%' || kw.value || '%'
                )
              )
              AND NOT EXISTS (
                SELECT 1 FROM json_each(?) AS filter
                WHERE NOT EXISTS (
                  SELECT 1 FROM note_extractions extraction
                  WHERE extraction.user_id = notes.user_id
                    AND extraction.note_id = notes.id
                    AND extraction.key = json_extract(filter.value, '$.key')
                    AND extraction.value = json_extract(filter.value, '$.value')
                )
              )
            ORDER BY updated_at DESC
            LIMIT ?
            "#,
            params![
                self.user_id,
                filter.archived,
                filter.note_type,
                filter.note_type,
                filter.project_id,
                filter.project_id,
                keywords_json,
                keywords_json,
                extractions_json,
                limit,
            ],
        )
    }

    async fn insert_note(&self, req: &InsertNoteReq<'_>) -> Result<InsertedNote, CliError> {
        let writer = self.db.writer().await?;
        writer.execute(
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
                req.now,
            ],
        )?;
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
        let writer = self.db.writer().await?;
        writer.execute(
            SQ_UPDATE_CONTENT,
            params![content, requeue, now, self.user_id, id],
        )?;
        Ok(())
    }

    async fn set_note_deleted_at(
        &self,
        id: &str,
        deleted_at: Option<&str>,
        now: &str,
    ) -> Result<(), CliError> {
        let writer = self.db.writer().await?;
        if let Some(ts) = deleted_at {
            writer.execute(SQ_SET_DELETED_AT, params![ts, now, self.user_id, id])?;
        } else {
            writer.execute(SQ_SET_DELETED_AT_NULL, params![now, self.user_id, id])?;
        }
        Ok(())
    }

    async fn undo_last_delete(&self) -> Result<(), CliError> {
        let now = chrono::Utc::now().to_rfc3339();
        let writer = self.db.writer().await?;
        writer.execute(SQ_UNDO_DELETE, params![now, self.user_id])?;
        Ok(())
    }

    async fn find_project_by_name(&self, name: &str) -> Result<Option<String>, CliError> {
        let reader = self.db.reader().await?;
        Ok(reader
            .query_row(SQ_FIND_PROJECT, params![self.user_id, name], |row| {
                row.get(0)
            })
            .optional()?)
    }

    async fn find_project_name_by_id(&self, project_id: &str) -> Result<Option<String>, CliError> {
        let reader = self.db.reader().await?;
        Ok(reader
            .query_row(
                SQ_FIND_PROJECT_NAME,
                params![self.user_id, project_id],
                |row| row.get(0),
            )
            .optional()?)
    }

    async fn list_projects(&self, archived: bool) -> Result<Vec<Project>, CliError> {
        let sql = if archived {
            SQ_LIST_PROJECTS_ARCHIVED
        } else {
            SQ_LIST_PROJECTS_ACTIVE
        };
        let reader = self.db.reader().await?;
        query_projects(&reader, sql, params![self.user_id])
    }

    async fn create_project(&self, name: &str) -> Result<String, CliError> {
        let id = uuid::Uuid::new_v4().to_string();
        let now = chrono::Utc::now().to_rfc3339();
        let writer = self.db.writer().await?;
        writer.execute(SQ_CREATE_PROJECT, params![id, self.user_id, name, now])?;
        Ok(id)
    }

    async fn move_note_to_project(
        &self,
        note_id: &str,
        new_project_id: &str,
        old_project_id: Option<&str>,
    ) -> Result<Option<String>, CliError> {
        let now = chrono::Utc::now().to_rfc3339();
        let mut writer = self.db.writer().await?;
        let tx = writer.transaction()?;
        let exists = tx
            .query_row(
                "SELECT 1 FROM notes WHERE user_id = ? AND id = ? AND deleted_at IS NULL LIMIT 1",
                params![self.user_id, note_id],
                |row| row.get::<_, i64>(0),
            )
            .optional()?
            .is_some();
        if !exists {
            return Err(CliError::NoteNotFound {
                id: note_id.to_string(),
            });
        }

        tx.execute(
            SQ_UPDATE_PROJECT,
            params![new_project_id, now, self.user_id, note_id],
        )?;

        let Some(old_pid) = old_project_id else {
            tx.commit()?;
            return Ok(None);
        };

        let count = tx.query_row(
            SQ_COUNT_PROJECT_NOTES,
            params![self.user_id, old_pid],
            |row| row.get::<_, i64>(0),
        )?;

        if count != 0 {
            tx.commit()?;
            return Ok(None);
        }

        let old_name = tx
            .query_row(
                SQ_FIND_PROJECT_NAME,
                params![self.user_id, old_pid],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        tx.execute(SQ_DELETE_PROJECT, params![self.user_id, old_pid])?;
        tx.commit()?;
        Ok(old_name)
    }

    async fn find_project(&self, id: &str) -> Result<Project, CliError> {
        let reader = self.db.reader().await?;
        reader
            .query_row(
                SQ_FIND_PROJECT_BY_ID,
                params![self.user_id, id],
                decode_project,
            )
            .optional()?
            .ok_or_else(|| CliError::Other(format!("Project not found: {id}")))
    }

    async fn resolve_project_id(&self, prefix: &str) -> Result<String, CliError> {
        resolve_sqlite_uuid_id(&self.db, SQ_RESOLVE_PROJECT, &self.user_id, prefix, || {
            CliError::Other(format!("Project not found: {prefix}"))
        })
        .await
    }

    async fn update_project(&self, id: &str, color: Option<Option<&str>>) -> Result<(), CliError> {
        let update_color = color.is_some();
        if !update_color {
            return Ok(());
        }

        let color_value = color.flatten();
        let writer = self.db.writer().await?;
        writer.execute(
            r#"
            UPDATE projects SET
                color = CASE WHEN ? THEN ? ELSE color END
            WHERE user_id = ? AND id = ?
            "#,
            params![update_color, color_value, self.user_id, id],
        )?;
        Ok(())
    }

    async fn delete_project(&self, id: &str) -> Result<(), CliError> {
        if !sqlite_exists(
            &self.db,
            "SELECT 1 FROM projects WHERE user_id = ? AND id = ? LIMIT 1",
            &self.user_id,
            id,
        )
        .await?
        {
            return Err(CliError::Other(format!("Project not found: {id}")));
        }
        let writer = self.db.writer().await?;
        writer.execute(SQ_ARCHIVE_PROJECT, params![self.user_id, id])?;
        Ok(())
    }

    async fn update_note_title(&self, id: &str, title: &str) -> Result<(), CliError> {
        let now = chrono::Utc::now().to_rfc3339();
        if !sqlite_exists(
            &self.db,
            "SELECT 1 FROM notes WHERE user_id = ? AND id = ? AND deleted_at IS NULL LIMIT 1",
            &self.user_id,
            id,
        )
        .await?
        {
            return Err(CliError::NoteNotFound { id: id.to_string() });
        }
        let writer = self.db.writer().await?;
        writer.execute(SQ_UPDATE_TITLE, params![title, now, self.user_id, id])?;
        Ok(())
    }

    async fn update_note_flagged(&self, id: &str, flagged: bool) -> Result<(), CliError> {
        let now = chrono::Utc::now().to_rfc3339();
        let val: i64 = if flagged { 1 } else { 0 };
        if !sqlite_exists(
            &self.db,
            "SELECT 1 FROM notes WHERE user_id = ? AND id = ? AND deleted_at IS NULL LIMIT 1",
            &self.user_id,
            id,
        )
        .await?
        {
            return Err(CliError::NoteNotFound { id: id.to_string() });
        }
        let writer = self.db.writer().await?;
        writer.execute(SQ_UPDATE_FLAGGED, params![val, now, self.user_id, id])?;
        Ok(())
    }

    async fn count_notes(&self, filter: &NoteFilter<'_>) -> Result<u64, CliError> {
        let reader = self.db.reader().await?;
        let count = reader.query_row(
            r#"
            SELECT COUNT(*)
            FROM notes
            WHERE user_id = ?
              AND (deleted_at IS NOT NULL) = ?
              AND (? IS NULL OR type = ?)
              AND (? IS NULL OR project_id = ?)
            "#,
            params![
                self.user_id,
                filter.archived,
                filter.note_type,
                filter.note_type,
                filter.project_id,
                filter.project_id,
            ],
            |row| row.get::<_, i64>(0),
        )?;
        count
            .try_into()
            .map_err(|_| CliError::Other(format!("unexpected negative count: {count}")))
    }

    async fn list_note_topics(
        &self,
        note_ids: &[&str],
    ) -> Result<std::collections::HashMap<String, Vec<String>>, CliError> {
        let extractions = self
            .list_note_extractions(note_ids, &[TOPIC_EXTRACTION_KEY])
            .await?;
        let mut map = std::collections::HashMap::new();
        for (note_id, pairs) in extractions {
            map.insert(note_id, pairs.into_iter().map(|(_, value)| value).collect());
        }
        Ok(map)
    }
    async fn list_note_extractions(
        &self,
        note_ids: &[&str],
        extraction_keys: &[&str],
    ) -> Result<std::collections::HashMap<String, Vec<(String, String)>>, CliError> {
        if note_ids.is_empty() || extraction_keys.is_empty() {
            return Ok(std::collections::HashMap::new());
        }
        let note_ids_json = serde_json::to_string(note_ids)?;
        let keys_json = serde_json::to_string(extraction_keys)?;
        let reader = self.db.reader().await?;
        let mut statement = reader.prepare(SQ_LIST_EXTRACTIONS)?;
        let rows = statement
            .query_map(params![self.user_id, keys_json, note_ids_json], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        let mut map: std::collections::HashMap<String, Vec<(String, String)>> =
            std::collections::HashMap::new();
        for (note_id, ext_type, value) in rows {
            map.entry(note_id).or_default().push((ext_type, value));
        }
        Ok(map)
    }

    async fn list_extraction_values(
        &self,
        extraction_keys: &[&str],
        archived: bool,
    ) -> Result<Vec<String>, CliError> {
        if extraction_keys.is_empty() {
            return Ok(Vec::new());
        }
        let keys_json = serde_json::to_string(extraction_keys)?;
        let reader = self.db.reader().await?;
        let mut statement = reader.prepare(SQ_LIST_EXTRACTION_VALUES)?;
        Ok(statement
            .query_map(params![self.user_id, keys_json, archived], |row| row.get(0))?
            .collect::<Result<Vec<_>, _>>()?)
    }
    async fn set_note_extractions(
        &self,
        note_id: &str,
        extraction_key: &str,
        values: &[String],
    ) -> Result<(), CliError> {
        let mut writer = self.db.writer().await?;
        let transaction = writer.transaction()?;
        transaction.execute(
            SQ_CLEAR_EXTRACTIONS,
            params![self.user_id, note_id, extraction_key],
        )?;
        for value in values {
            let id = uuid::Uuid::new_v4().to_string();
            transaction.execute(
                SQ_INSERT_EXTRACTION,
                params![id, note_id, self.user_id, extraction_key, value],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }
}
