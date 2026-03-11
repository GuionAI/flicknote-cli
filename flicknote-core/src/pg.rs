use std::cell::RefCell;

use chrono::{DateTime, Utc};
use postgres::{Client, NoTls};

use crate::backend::{InsertNoteReq, NoteDb, NoteFilter, validate_id_prefix};
use crate::error::CliError;
use crate::types::{Note, Project};

pub struct PgBackend {
    client: RefCell<Client>,
    user_id: String,
}

impl PgBackend {
    pub fn connect(url: &str, user_id: String) -> Result<Self, CliError> {
        let client = Client::connect(url, NoTls)?;
        Ok(Self {
            client: RefCell::new(client),
            user_id,
        })
    }
}

// ─── SQL constants ───────────────────────────────────────────────────────────
// id and project_id are uuid type in PG — cast to ::text for LIKE comparisons
// and to read as String. is_archived is boolean. timestamps are timestamptz.

const PG_NOTE_COLS: &str = "id::text, user_id::text, type, status, title, content, summary, \
     is_flagged, project_id::text, metadata::text, source::text, \
     external_id::text, created_at, updated_at, deleted_at";

const PG_FIND: &str = "SELECT id::text, user_id::text, type, status, title, content, summary, \
     is_flagged, project_id::text, metadata::text, source::text, \
     external_id::text, created_at, updated_at, deleted_at \
     FROM notes WHERE user_id = ($1::text)::uuid AND id = ($2::text)::uuid AND deleted_at IS NULL LIMIT 1";

const PG_RESOLVE: &str = "SELECT id::text FROM notes WHERE user_id = ($1::text)::uuid AND id::text LIKE $2 \
     AND deleted_at IS NULL LIMIT 2";
const PG_RESOLVE_ARCHIVED: &str = "SELECT id::text FROM notes WHERE user_id = ($1::text)::uuid AND id::text LIKE $2 \
     AND deleted_at IS NOT NULL LIMIT 2";
const PG_FIND_CONTENT: &str = "SELECT content FROM notes WHERE user_id = ($1::text)::uuid AND id = ($2::text)::uuid \
     AND deleted_at IS NULL LIMIT 1";
const PG_INSERT: &str = "INSERT INTO notes \
     (id, user_id, type, status, title, content, metadata, project_id, created_at, updated_at) \
     VALUES (($1::text)::uuid, ($2::text)::uuid, $3, $4, $5, $6, $7::text::jsonb, ($8::text)::uuid, ($9::text)::timestamptz, ($10::text)::timestamptz)";
const PG_UPDATE_CONTENT: &str = "UPDATE notes SET content = $1, updated_at = ($2::text)::timestamptz WHERE user_id = ($3::text)::uuid AND id = ($4::text)::uuid";
const PG_UPDATE_CONTENT_REQUEUE: &str = "UPDATE notes SET content = $1, status = 'ai_queued', updated_at = ($2::text)::timestamptz \
     WHERE user_id = ($3::text)::uuid AND id = ($4::text)::uuid";
const PG_SET_DELETED_AT: &str = "UPDATE notes SET deleted_at = ($1::text)::timestamptz, updated_at = ($2::text)::timestamptz WHERE user_id = ($3::text)::uuid AND id = ($4::text)::uuid";
const PG_SET_DELETED_AT_NULL: &str = "UPDATE notes SET deleted_at = NULL, updated_at = ($1::text)::timestamptz WHERE user_id = ($2::text)::uuid AND id = ($3::text)::uuid";
const PG_UPDATE_PROJECT: &str = "UPDATE notes SET project_id = ($1::text)::uuid, updated_at = ($2::text)::timestamptz WHERE user_id = ($3::text)::uuid AND id = ($4::text)::uuid";

const PG_FIND_PROJECT: &str = "SELECT id::text FROM projects WHERE user_id = ($1::text)::uuid AND name = $2 AND NOT is_archived LIMIT 1";
const PG_FIND_PROJECT_NAME: &str =
    "SELECT name FROM projects WHERE user_id = ($1::text)::uuid AND id = ($2::text)::uuid LIMIT 1";
const PG_LIST_PROJECTS_ACTIVE: &str = "SELECT id::text, user_id::text, name, color, is_archived::int, created_at \
     FROM projects WHERE user_id = ($1::text)::uuid AND NOT is_archived ORDER BY name";
const PG_LIST_PROJECTS_ARCHIVED: &str = "SELECT id::text, user_id::text, name, color, is_archived::int, created_at \
     FROM projects WHERE user_id = ($1::text)::uuid AND is_archived ORDER BY name";
const PG_CREATE_PROJECT: &str = "INSERT INTO projects (id, user_id, name, is_archived, created_at) \
     VALUES (($1::text)::uuid, ($2::text)::uuid, $3, false, ($4::text)::timestamptz)";
const PG_COUNT_PROJECT_NOTES: &str = "SELECT COUNT(*) FROM notes WHERE user_id = ($1::text)::uuid AND project_id = ($2::text)::uuid AND deleted_at IS NULL";
const PG_DELETE_PROJECT: &str =
    "DELETE FROM projects WHERE user_id = ($1::text)::uuid AND id = ($2::text)::uuid";
const PG_UNDO_DELETE: &str = "UPDATE notes SET deleted_at = NULL, updated_at = ($1::text)::timestamptz \
     WHERE id = (SELECT id FROM notes WHERE deleted_at IS NOT NULL AND user_id = ($2::text)::uuid \
     ORDER BY deleted_at DESC LIMIT 1)";

// ─── Row helpers ─────────────────────────────────────────────────────────────

fn ts_col(row: &postgres::Row, col: &str) -> Result<Option<String>, postgres::Error> {
    Ok(row
        .try_get::<_, Option<DateTime<Utc>>>(col)?
        .map(|dt| dt.to_rfc3339()))
}

fn note_from_pg_row(row: &postgres::Row) -> Result<Note, postgres::Error> {
    let is_flagged: Option<i64> = row
        .try_get::<_, Option<bool>>("is_flagged")?
        .map(|b| if b { 1_i64 } else { 0_i64 });

    Ok(Note {
        id: row.try_get("id")?,
        user_id: row.try_get("user_id")?,
        r#type: row.try_get("type")?,
        status: row.try_get("status")?,
        title: row.try_get("title")?,
        content: row.try_get("content")?,
        summary: row.try_get("summary")?,
        is_flagged,
        project_id: row.try_get("project_id")?,
        metadata: row.try_get("metadata")?,
        source: row.try_get("source")?,
        external_id: row.try_get("external_id")?,
        created_at: ts_col(row, "created_at")?,
        updated_at: ts_col(row, "updated_at")?,
        deleted_at: ts_col(row, "deleted_at")?,
    })
}

fn project_from_pg_row(row: &postgres::Row) -> Result<Project, postgres::Error> {
    Ok(Project {
        id: row.try_get("id")?,
        user_id: row.try_get("user_id")?,
        name: row.try_get("name")?,
        color: row.try_get("color")?,
        is_archived: row.try_get::<_, Option<i32>>("is_archived")?.map(i64::from),
        created_at: ts_col(row, "created_at")?,
    })
}

// ─── NoteDb impl ─────────────────────────────────────────────────────────────

impl NoteDb for PgBackend {
    fn user_id(&self) -> &str {
        &self.user_id
    }

    fn resolve_note_id(&self, prefix: &str) -> Result<String, CliError> {
        validate_id_prefix(prefix)?;
        let rows = self
            .client
            .borrow_mut()
            .query(PG_RESOLVE, &[&self.user_id, &format!("{prefix}%")])?;
        match rows.len() {
            0 => Err(CliError::NoteNotFound {
                id: prefix.to_string(),
            }),
            1 => Ok(rows[0].get(0)),
            _ => Err(CliError::Other(format!("Ambiguous ID prefix: {prefix}"))),
        }
    }

    fn resolve_archived_note_id(&self, prefix: &str) -> Result<String, CliError> {
        validate_id_prefix(prefix)?;
        let rows = self
            .client
            .borrow_mut()
            .query(PG_RESOLVE_ARCHIVED, &[&self.user_id, &format!("{prefix}%")])?;
        match rows.len() {
            0 => Err(CliError::NoteNotFound {
                id: prefix.to_string(),
            }),
            1 => Ok(rows[0].get(0)),
            _ => Err(CliError::Other(format!("Ambiguous ID prefix: {prefix}"))),
        }
    }

    fn find_note(&self, id: &str) -> Result<Note, CliError> {
        let rows = self
            .client
            .borrow_mut()
            .query(PG_FIND, &[&self.user_id, &id])?;
        rows.first()
            .map(note_from_pg_row)
            .transpose()?
            .ok_or_else(|| CliError::NoteNotFound { id: id.to_string() })
    }

    fn find_note_content(&self, id: &str) -> Result<Option<String>, CliError> {
        let rows = self
            .client
            .borrow_mut()
            .query(PG_FIND_CONTENT, &[&self.user_id, &id])?;
        match rows.first() {
            Some(row) => Ok(row.get("content")),
            None => Err(CliError::NoteNotFound { id: id.to_string() }),
        }
    }

    fn list_notes(&self, filter: &NoteFilter<'_>) -> Result<Vec<Note>, CliError> {
        let archive_cond = if filter.archived {
            "deleted_at IS NOT NULL"
        } else {
            "deleted_at IS NULL"
        };
        let mut sql = format!(
            "SELECT {PG_NOTE_COLS} FROM notes WHERE user_id = ($1::text)::uuid AND {archive_cond}"
        );
        let mut param_idx = 2usize;
        if filter.note_type.is_some() {
            sql.push_str(&format!(" AND type = ${param_idx}"));
            param_idx += 1;
        }
        if filter.project_id.is_some() {
            sql.push_str(&format!(" AND project_id = (${param_idx}::text)::uuid"));
            param_idx += 1;
        }
        sql.push_str(&format!(" ORDER BY created_at DESC LIMIT ${param_idx}"));
        let limit = filter.limit as i64;

        // Build params
        let user_id_box: Box<dyn postgres::types::ToSql + Sync> = Box::new(self.user_id.clone());
        let mut params_storage: Vec<Box<dyn postgres::types::ToSql + Sync>> = vec![user_id_box];
        if let Some(t) = filter.note_type {
            params_storage.push(Box::new(t.to_string()));
        }
        if let Some(p) = filter.project_id {
            params_storage.push(Box::new(p.to_string()));
        }
        params_storage.push(Box::new(limit));

        let param_refs: Vec<&(dyn postgres::types::ToSql + Sync)> = params_storage
            .iter()
            .map(std::convert::AsRef::as_ref)
            .collect();

        let rows = self
            .client
            .borrow_mut()
            .query(sql.as_str(), param_refs.as_slice())?;
        rows.iter()
            .map(note_from_pg_row)
            .collect::<Result<Vec<_>, _>>()
            .map_err(CliError::Postgres)
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

        let user_id_box: Box<dyn postgres::types::ToSql + Sync> = Box::new(self.user_id.clone());
        let mut params_storage: Vec<Box<dyn postgres::types::ToSql + Sync>> = vec![user_id_box];

        let mut param_idx = 2usize;
        let keyword_blocks: Vec<String> = keywords
            .iter()
            .map(|kw| {
                let pattern = format!("%{kw}%");
                let t = param_idx;
                let c = param_idx + 1;
                let s = param_idx + 2;
                param_idx += 3;
                params_storage.push(Box::new(pattern.clone()));
                params_storage.push(Box::new(pattern.clone()));
                params_storage.push(Box::new(pattern));
                format!("(title ILIKE ${t} OR content ILIKE ${c} OR summary ILIKE ${s})")
            })
            .collect();
        let keywords_clause = keyword_blocks.join(" OR ");

        let mut sql = format!(
            "SELECT {PG_NOTE_COLS} FROM notes WHERE user_id = ($1::text)::uuid AND {archive_cond} \
             AND ({keywords_clause})"
        );
        if filter.project_id.is_some() {
            sql.push_str(&format!(" AND project_id = (${param_idx}::text)::uuid"));
            param_idx += 1;
        }
        if let Some(p) = filter.project_id {
            params_storage.push(Box::new(p.to_string()));
        }
        sql.push_str(&format!(" ORDER BY updated_at DESC LIMIT ${param_idx}"));
        params_storage.push(Box::new(filter.limit as i64));

        let param_refs: Vec<&(dyn postgres::types::ToSql + Sync)> = params_storage
            .iter()
            .map(std::convert::AsRef::as_ref)
            .collect();

        let rows = self
            .client
            .borrow_mut()
            .query(sql.as_str(), param_refs.as_slice())?;
        rows.iter()
            .map(note_from_pg_row)
            .collect::<Result<Vec<_>, _>>()
            .map_err(CliError::Postgres)
    }

    fn insert_note(&self, req: &InsertNoteReq<'_>) -> Result<(), CliError> {
        self.client.borrow_mut().execute(
            PG_INSERT,
            &[
                &req.id,
                &self.user_id,
                &req.note_type,
                &req.status,
                &req.title,
                &req.content,
                &req.metadata,
                &req.project_id,
                &req.now,
                &req.now,
            ],
        )?;
        Ok(())
    }

    fn update_note_content(&self, id: &str, content: &str, requeue: bool) -> Result<(), CliError> {
        let now = chrono::Utc::now().to_rfc3339();
        if requeue {
            self.client.borrow_mut().execute(
                PG_UPDATE_CONTENT_REQUEUE,
                &[&content, &now, &self.user_id, &id],
            )?;
        } else {
            self.client
                .borrow_mut()
                .execute(PG_UPDATE_CONTENT, &[&content, &now, &self.user_id, &id])?;
        }
        Ok(())
    }

    fn set_note_deleted_at(&self, id: &str, deleted_at: Option<&str>) -> Result<(), CliError> {
        let now = chrono::Utc::now().to_rfc3339();
        if let Some(ts) = deleted_at {
            self.client
                .borrow_mut()
                .execute(PG_SET_DELETED_AT, &[&ts, &now, &self.user_id, &id])?;
        } else {
            self.client
                .borrow_mut()
                .execute(PG_SET_DELETED_AT_NULL, &[&now, &self.user_id, &id])?;
        }
        Ok(())
    }

    fn undo_last_delete(&self) -> Result<(), CliError> {
        let now = chrono::Utc::now().to_rfc3339();
        self.client
            .borrow_mut()
            .execute(PG_UNDO_DELETE, &[&now, &self.user_id])?;
        Ok(())
    }

    fn find_project_by_name(&self, name: &str) -> Result<Option<String>, CliError> {
        let rows = self
            .client
            .borrow_mut()
            .query(PG_FIND_PROJECT, &[&self.user_id, &name])?;
        Ok(rows.first().map(|r| r.get(0)))
    }

    fn find_project_name_by_id(&self, project_id: &str) -> Result<Option<String>, CliError> {
        let rows = self
            .client
            .borrow_mut()
            .query(PG_FIND_PROJECT_NAME, &[&self.user_id, &project_id])?;
        Ok(rows.first().map(|r| r.get(0)))
    }

    fn list_projects(&self, archived: bool) -> Result<Vec<Project>, CliError> {
        let sql = if archived {
            PG_LIST_PROJECTS_ARCHIVED
        } else {
            PG_LIST_PROJECTS_ACTIVE
        };
        let rows = self.client.borrow_mut().query(sql, &[&self.user_id])?;
        rows.iter()
            .map(project_from_pg_row)
            .collect::<Result<Vec<_>, _>>()
            .map_err(CliError::Postgres)
    }

    fn create_project(&self, name: &str) -> Result<String, CliError> {
        let id = uuid::Uuid::new_v4().to_string();
        let now = chrono::Utc::now().to_rfc3339();
        self.client
            .borrow_mut()
            .execute(PG_CREATE_PROJECT, &[&id, &self.user_id, &name, &now])?;
        Ok(id)
    }

    fn move_note_to_project(
        &self,
        note_id: &str,
        new_project_id: &str,
        old_project_id: Option<&str>,
    ) -> Result<Option<String>, CliError> {
        let mut client = self.client.borrow_mut();
        let now = chrono::Utc::now().to_rfc3339();

        let mut tx = client.transaction()?;

        let affected = tx.execute(
            PG_UPDATE_PROJECT,
            &[&new_project_id, &now, &self.user_id, &note_id],
        )?;
        if affected == 0 {
            return Err(CliError::NoteNotFound {
                id: note_id.to_string(),
            });
        }

        let old_name = if let Some(old_pid) = old_project_id {
            let count: i64 = tx
                .query_one(PG_COUNT_PROJECT_NOTES, &[&self.user_id, &old_pid])?
                .get(0);
            if count == 0 {
                let name: Option<String> = tx
                    .query_opt(PG_FIND_PROJECT_NAME, &[&self.user_id, &old_pid])?
                    .map(|r| r.get(0));
                tx.execute(PG_DELETE_PROJECT, &[&self.user_id, &old_pid])?;
                name
            } else {
                None
            }
        } else {
            None
        };

        tx.commit()?;
        Ok(old_name)
    }
}
