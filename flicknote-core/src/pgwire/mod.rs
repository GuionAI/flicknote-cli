//! PgWire backend for Supabase Postgres.
//!
//! This backend assumes the connection is routed through pgwire-supabase-proxy
//! (or equivalent) which sets the JWT/RLS context for the session. Tenant
//! isolation is enforced by RLS, so queries do not add user_id predicates.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::{PgPool, Row, postgres::PgPoolOptions};
use uuid::Uuid;

use crate::backend::{InsertNoteReq, NoteDb, NoteFilter};
use crate::error::CliError;
use crate::types::{Keyterm, Note, Project, Prompt};

const PG_FIND_NOTE: &str = "SELECT id, user_id, type, status, title, content, summary, is_flagged, \
     project_id, metadata, source, external_id, created_at, updated_at, deleted_at \
     FROM notes WHERE id = $1 AND deleted_at IS NULL LIMIT 1";
const PG_FIND_ARCHIVED_NOTE: &str = "SELECT id, user_id, type, status, title, content, summary, is_flagged, \
     project_id, metadata, source, external_id, created_at, updated_at, deleted_at \
     FROM notes WHERE id = $1 AND deleted_at IS NOT NULL LIMIT 1";
const PG_FIND_PROJECT: &str = "SELECT id, user_id, name, color, prompt_id, keyterm_id, is_archived, created_at \
     FROM projects WHERE id = $1 LIMIT 1";
const PG_LIST_PROJECTS_ACTIVE: &str = "SELECT id, user_id, name, color, prompt_id, keyterm_id, is_archived, created_at \
     FROM projects WHERE COALESCE(is_archived, false) = false ORDER BY name";
const PG_LIST_PROJECTS_ARCHIVED: &str = "SELECT id, user_id, name, color, prompt_id, keyterm_id, is_archived, created_at \
     FROM projects WHERE COALESCE(is_archived, false) = true ORDER BY name";
const PG_FIND_PROMPT: &str =
    "SELECT id, user_id, title, description, prompt, created_at FROM prompts WHERE id = $1 LIMIT 1";
const PG_LIST_PROMPTS: &str = "SELECT id, user_id, title, description, prompt, created_at FROM prompts ORDER BY created_at DESC";
const PG_FIND_KEYTERM: &str = "SELECT id, user_id, name, description, content, created_at, updated_at \
     FROM keyterms WHERE id = $1 LIMIT 1";
const PG_LIST_KEYTERMS: &str = "SELECT id, user_id, name, description, content, created_at, updated_at \
     FROM keyterms ORDER BY name";

#[derive(sqlx::FromRow)]
struct NotePgRow {
    pub id: Uuid,
    pub user_id: Uuid,
    #[sqlx(rename = "type")]
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

#[derive(sqlx::FromRow)]
struct ProjectPgRow {
    pub id: Uuid,
    pub user_id: Uuid,
    pub name: String,
    pub color: Option<String>,
    pub prompt_id: Option<Uuid>,
    pub keyterm_id: Option<Uuid>,
    pub is_archived: Option<bool>,
    pub created_at: Option<DateTime<Utc>>,
}

#[derive(sqlx::FromRow)]
struct PromptPgRow {
    pub id: Uuid,
    pub user_id: Uuid,
    pub title: String,
    pub description: Option<String>,
    pub prompt: String,
    pub created_at: Option<DateTime<Utc>>,
}

#[derive(sqlx::FromRow)]
struct KeytermPgRow {
    pub id: Uuid,
    pub user_id: Uuid,
    pub name: String,
    pub description: Option<String>,
    pub content: Option<String>,
    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
}

impl From<NotePgRow> for Note {
    fn from(r: NotePgRow) -> Self {
        Self {
            id: r.id.to_string(),
            user_id: r.user_id.to_string(),
            r#type: r.r#type,
            status: r.status,
            title: r.title,
            content: r.content,
            summary: r.summary,
            is_flagged: r.is_flagged.map(|b| if b { 1 } else { 0 }),
            project_id: r.project_id.map(|u| u.to_string()),
            metadata: r.metadata.map(|v| v.to_string()),
            source: r.source.map(|v| v.to_string()),
            external_id: r.external_id.map(|v| v.to_string()),
            created_at: r.created_at.map(|t| t.to_rfc3339()),
            updated_at: r.updated_at.map(|t| t.to_rfc3339()),
            deleted_at: r.deleted_at.map(|t| t.to_rfc3339()),
        }
    }
}

impl From<ProjectPgRow> for Project {
    fn from(r: ProjectPgRow) -> Self {
        Self {
            id: r.id.to_string(),
            user_id: r.user_id.to_string(),
            name: r.name,
            color: r.color,
            prompt_id: r.prompt_id.map(|u| u.to_string()),
            keyterm_id: r.keyterm_id.map(|u| u.to_string()),
            is_archived: r.is_archived.map(|b| if b { 1 } else { 0 }),
            created_at: r.created_at.map(|t| t.to_rfc3339()),
        }
    }
}

impl From<PromptPgRow> for Prompt {
    fn from(r: PromptPgRow) -> Self {
        Self {
            id: r.id.to_string(),
            user_id: r.user_id.to_string(),
            title: r.title,
            description: r.description,
            prompt: r.prompt,
            created_at: r.created_at.map(|t| t.to_rfc3339()),
        }
    }
}

impl From<KeytermPgRow> for Keyterm {
    fn from(r: KeytermPgRow) -> Self {
        Self {
            id: r.id.to_string(),
            user_id: r.user_id.to_string(),
            name: r.name,
            description: r.description,
            content: r.content,
            created_at: r.created_at.map(|t| t.to_rfc3339()),
            updated_at: r.updated_at.map(|t| t.to_rfc3339()),
        }
    }
}

fn parse_uuid(s: &str) -> Result<Uuid, CliError> {
    Uuid::parse_str(s).map_err(|e| CliError::Database(format!("invalid UUID {s:?}: {e}")))
}

fn parse_uuid_opt(s: Option<&str>) -> Result<Option<Uuid>, CliError> {
    s.map(parse_uuid).transpose()
}

fn parse_iso_utc(s: &str) -> Result<DateTime<Utc>, CliError> {
    DateTime::parse_from_rfc3339(s)
        .map(|dt| dt.with_timezone(&Utc))
        .map_err(|e| CliError::Database(format!("invalid ISO timestamp {s:?}: {e}")))
}

async fn resolve_uuid_prefix(
    pool: &PgPool,
    sql: &str,
    prefix: &str,
    label: &str,
    missing: impl FnOnce() -> CliError,
) -> Result<String, CliError> {
    let rows = sqlx::query_scalar::<_, String>(sql)
        .bind(format!("{prefix}%"))
        .fetch_all(pool)
        .await?;
    match rows.as_slice() {
        [_, _, ..] => Err(CliError::Other(format!(
            "Ambiguous {label} prefix: {prefix}"
        ))),
        [id] => Ok(id.clone()),
        [] => Err(missing()),
    }
}

pub struct PgWireBackend {
    pool: PgPool,
}

impl PgWireBackend {
    pub async fn connect(database_url: &str) -> Result<Self, CliError> {
        let pool = PgPoolOptions::new()
            .max_connections(5)
            .connect(database_url)
            .await?;
        Ok(Self { pool })
    }
}

#[async_trait(?Send)]
impl NoteDb for PgWireBackend {
    fn user_id(&self) -> &str {
        ""
    }

    async fn resolve_note_id(&self, prefix: &str) -> Result<String, CliError> {
        crate::backend::validate_id_prefix(prefix)?;
        resolve_uuid_prefix(
            &self.pool,
            "SELECT id::text FROM notes WHERE id::text LIKE $1 AND deleted_at IS NULL LIMIT 2",
            prefix,
            "ID",
            || CliError::NoteNotFound {
                id: prefix.to_string(),
            },
        )
        .await
    }

    async fn resolve_archived_note_id(&self, prefix: &str) -> Result<String, CliError> {
        crate::backend::validate_id_prefix(prefix)?;
        resolve_uuid_prefix(
            &self.pool,
            "SELECT id::text FROM notes WHERE id::text LIKE $1 AND deleted_at IS NOT NULL LIMIT 2",
            prefix,
            "ID",
            || CliError::NoteNotFound {
                id: prefix.to_string(),
            },
        )
        .await
    }

    async fn find_note(&self, id: &str) -> Result<Note, CliError> {
        sqlx::query_as::<_, NotePgRow>(PG_FIND_NOTE)
            .bind(parse_uuid(id)?)
            .fetch_optional(&self.pool)
            .await?
            .map(Note::from)
            .ok_or_else(|| CliError::NoteNotFound { id: id.to_string() })
    }

    async fn find_archived_note(&self, id: &str) -> Result<Note, CliError> {
        sqlx::query_as::<_, NotePgRow>(PG_FIND_ARCHIVED_NOTE)
            .bind(parse_uuid(id)?)
            .fetch_optional(&self.pool)
            .await?
            .map(Note::from)
            .ok_or_else(|| CliError::NoteNotFound { id: id.to_string() })
    }

    async fn find_note_content(&self, id: &str) -> Result<Option<String>, CliError> {
        sqlx::query_scalar::<_, Option<String>>(
            "SELECT content FROM notes WHERE id = $1 AND deleted_at IS NULL LIMIT 1",
        )
        .bind(parse_uuid(id)?)
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| CliError::NoteNotFound { id: id.to_string() })
    }

    async fn list_notes(&self, filter: &NoteFilter<'_>) -> Result<Vec<Note>, CliError> {
        let project_id = parse_uuid_opt(filter.project_id)?;
        let limit = i64::from(filter.limit);
        let rows = sqlx::query_as!(
            NotePgRow,
            r#"
            SELECT
                id as "id!",
                user_id as "user_id!",
                type as "type!",
                status as "status!",
                title,
                content,
                summary,
                is_flagged,
                project_id,
                metadata as "metadata: _",
                source as "source: _",
                external_id as "external_id: _",
                created_at,
                updated_at,
                deleted_at
            FROM notes
            WHERE (deleted_at IS NOT NULL) = $1
              AND ($2::text IS NULL OR type = $2)
              AND ($3::uuid IS NULL OR project_id = $3)
            ORDER BY created_at DESC
            LIMIT $4
            "#,
            filter.archived,
            filter.note_type,
            project_id,
            limit,
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().map(Note::from).collect())
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
        let project_id = parse_uuid_opt(filter.project_id)?;
        let limit = i64::from(filter.limit);
        let rows = sqlx::query_as!(
            NotePgRow,
            r#"
            SELECT
                id as "id!",
                user_id as "user_id!",
                type as "type!",
                status as "status!",
                title,
                content,
                summary,
                is_flagged,
                project_id,
                metadata as "metadata: _",
                source as "source: _",
                external_id as "external_id: _",
                created_at,
                updated_at,
                deleted_at
            FROM notes
            WHERE (deleted_at IS NOT NULL) = $1
              AND ($2::text IS NULL OR type = $2)
              AND ($3::uuid IS NULL OR project_id = $3)
              AND EXISTS (
                SELECT 1 FROM unnest($4::text[]) AS kw(term)
                WHERE title ILIKE '%' || kw.term || '%'
                   OR content ILIKE '%' || kw.term || '%'
                   OR summary ILIKE '%' || kw.term || '%'
              )
            ORDER BY updated_at DESC
            LIMIT $5
            "#,
            filter.archived,
            filter.note_type,
            project_id,
            keywords,
            limit,
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().map(Note::from).collect())
    }

    async fn insert_note(&self, req: &InsertNoteReq<'_>) -> Result<(), CliError> {
        let metadata: Option<serde_json::Value> = req
            .metadata
            .map(serde_json::from_str)
            .transpose()
            .map_err(|e| CliError::Database(format!("invalid metadata JSON: {e}")))?;
        let now = parse_iso_utc(req.now)?;
        sqlx::query(
            "INSERT INTO notes \
             (id, type, status, title, content, metadata, project_id, created_at, updated_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
        )
        .bind(parse_uuid(req.id)?)
        .bind(req.note_type)
        .bind(req.status)
        .bind(req.title)
        .bind(req.content)
        .bind(metadata)
        .bind(parse_uuid_opt(req.project_id)?)
        .bind(now)
        .bind(now)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn update_note_content(
        &self,
        id: &str,
        content: &str,
        requeue: bool,
    ) -> Result<(), CliError> {
        let now = Utc::now();
        let result = if requeue {
            sqlx::query("UPDATE notes SET content = $1, status = 'ai_queued', updated_at = $2 WHERE id = $3")
                .bind(content)
                .bind(now)
                .bind(parse_uuid(id)?)
                .execute(&self.pool)
                .await?
        } else {
            sqlx::query("UPDATE notes SET content = $1, updated_at = $2 WHERE id = $3")
                .bind(content)
                .bind(now)
                .bind(parse_uuid(id)?)
                .execute(&self.pool)
                .await?
        };
        if result.rows_affected() == 0 {
            return Err(CliError::NoteNotFound { id: id.to_string() });
        }
        Ok(())
    }

    async fn set_note_deleted_at(
        &self,
        id: &str,
        deleted_at: Option<&str>,
        now: &str,
    ) -> Result<(), CliError> {
        let result = sqlx::query("UPDATE notes SET deleted_at = $1, updated_at = $2 WHERE id = $3")
            .bind(deleted_at.map(parse_iso_utc).transpose()?)
            .bind(parse_iso_utc(now)?)
            .bind(parse_uuid(id)?)
            .execute(&self.pool)
            .await?;
        if result.rows_affected() == 0 {
            return Err(CliError::NoteNotFound { id: id.to_string() });
        }
        Ok(())
    }

    async fn undo_last_delete(&self) -> Result<(), CliError> {
        sqlx::query(
            "UPDATE notes SET deleted_at = NULL, updated_at = $1 \
             WHERE id = (SELECT id FROM notes WHERE deleted_at IS NOT NULL ORDER BY deleted_at DESC LIMIT 1)",
        )
        .bind(Utc::now())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn find_project_by_name(&self, name: &str) -> Result<Option<String>, CliError> {
        Ok(sqlx::query_scalar::<_, String>(
            "SELECT id::text FROM projects WHERE name = $1 AND COALESCE(is_archived, false) = false LIMIT 1",
        )
        .bind(name)
        .fetch_optional(&self.pool)
        .await?)
    }

    async fn find_project_name_by_id(&self, project_id: &str) -> Result<Option<String>, CliError> {
        Ok(
            sqlx::query_scalar::<_, String>("SELECT name FROM projects WHERE id = $1 LIMIT 1")
                .bind(parse_uuid(project_id)?)
                .fetch_optional(&self.pool)
                .await?,
        )
    }

    async fn list_projects(&self, archived: bool) -> Result<Vec<Project>, CliError> {
        let sql = if archived {
            PG_LIST_PROJECTS_ARCHIVED
        } else {
            PG_LIST_PROJECTS_ACTIVE
        };
        let rows = sqlx::query_as::<_, ProjectPgRow>(sql)
            .fetch_all(&self.pool)
            .await?;
        Ok(rows.into_iter().map(Project::from).collect())
    }

    async fn create_project(&self, name: &str) -> Result<String, CliError> {
        let id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO projects (id, name, is_archived, created_at) VALUES ($1, $2, false, $3)",
        )
        .bind(id)
        .bind(name)
        .bind(Utc::now())
        .execute(&self.pool)
        .await?;
        Ok(id.to_string())
    }

    async fn move_note_to_project(
        &self,
        note_id: &str,
        new_project_id: &str,
        old_project_id: Option<&str>,
    ) -> Result<Option<String>, CliError> {
        let note_uuid = parse_uuid(note_id)?;
        let new_project_uuid = parse_uuid(new_project_id)?;
        let mut tx = self.pool.begin().await?;
        let result = sqlx::query("UPDATE notes SET project_id = $1, updated_at = $2 WHERE id = $3")
            .bind(new_project_uuid)
            .bind(Utc::now())
            .bind(note_uuid)
            .execute(&mut *tx)
            .await?;
        if result.rows_affected() == 0 {
            return Err(CliError::NoteNotFound {
                id: note_id.to_string(),
            });
        }
        let Some(old_pid) = old_project_id else {
            tx.commit().await?;
            return Ok(None);
        };
        let old_uuid = parse_uuid(old_pid)?;
        let count = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM notes WHERE project_id = $1 AND deleted_at IS NULL",
        )
        .bind(old_uuid)
        .fetch_one(&mut *tx)
        .await?;
        if count != 0 {
            tx.commit().await?;
            return Ok(None);
        }
        let old_name = sqlx::query_scalar::<_, String>("SELECT name FROM projects WHERE id = $1")
            .bind(old_uuid)
            .fetch_optional(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM projects WHERE id = $1")
            .bind(old_uuid)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(old_name)
    }

    async fn find_project(&self, id: &str) -> Result<Project, CliError> {
        sqlx::query_as::<_, ProjectPgRow>(PG_FIND_PROJECT)
            .bind(parse_uuid(id)?)
            .fetch_optional(&self.pool)
            .await?
            .map(Project::from)
            .ok_or_else(|| CliError::Other(format!("Project not found: {id}")))
    }

    async fn resolve_project_id(&self, prefix: &str) -> Result<String, CliError> {
        crate::backend::validate_id_prefix(prefix)?;
        resolve_uuid_prefix(
            &self.pool,
            "SELECT id::text FROM projects WHERE id::text LIKE $1 LIMIT 2",
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

        let prompt_value = prompt_id.map(parse_uuid_opt).transpose()?.flatten();
        let keyterm_value = keyterm_id.map(parse_uuid_opt).transpose()?.flatten();
        let project_id = parse_uuid(id)?;
        let color_value = color.flatten();
        let result = sqlx::query!(
            r#"
            UPDATE projects SET
                prompt_id = CASE WHEN $2::bool THEN $3::uuid ELSE prompt_id END,
                keyterm_id = CASE WHEN $4::bool THEN $5::uuid ELSE keyterm_id END,
                color = CASE WHEN $6::bool THEN $7::text ELSE color END
            WHERE id = $1
            "#,
            project_id,
            update_prompt,
            prompt_value,
            update_keyterm,
            keyterm_value,
            update_color,
            color_value,
        )
        .execute(&self.pool)
        .await?;
        if result.rows_affected() == 0 {
            return Err(CliError::Other(format!("Project not found: {id}")));
        }
        Ok(())
    }

    async fn delete_project(&self, id: &str) -> Result<(), CliError> {
        let result = sqlx::query("UPDATE projects SET is_archived = true WHERE id = $1")
            .bind(parse_uuid(id)?)
            .execute(&self.pool)
            .await?;
        if result.rows_affected() == 0 {
            return Err(CliError::Other(format!("Project not found: {id}")));
        }
        Ok(())
    }

    async fn update_note_title(&self, id: &str, title: &str) -> Result<(), CliError> {
        let result = sqlx::query("UPDATE notes SET title = $1, updated_at = $2 WHERE id = $3")
            .bind(title)
            .bind(Utc::now())
            .bind(parse_uuid(id)?)
            .execute(&self.pool)
            .await?;
        if result.rows_affected() == 0 {
            return Err(CliError::NoteNotFound { id: id.to_string() });
        }
        Ok(())
    }

    async fn update_note_flagged(&self, id: &str, flagged: bool) -> Result<(), CliError> {
        let result = sqlx::query("UPDATE notes SET is_flagged = $1, updated_at = $2 WHERE id = $3")
            .bind(flagged)
            .bind(Utc::now())
            .bind(parse_uuid(id)?)
            .execute(&self.pool)
            .await?;
        if result.rows_affected() == 0 {
            return Err(CliError::NoteNotFound { id: id.to_string() });
        }
        Ok(())
    }

    async fn count_notes(&self, filter: &NoteFilter<'_>) -> Result<u64, CliError> {
        let project_id = parse_uuid_opt(filter.project_id)?;
        let count = sqlx::query_scalar!(
            r#"
            SELECT COUNT(*) as "count!"
            FROM notes
            WHERE (deleted_at IS NOT NULL) = $1
              AND ($2::text IS NULL OR type = $2)
              AND ($3::uuid IS NULL OR project_id = $3)
            "#,
            filter.archived,
            filter.note_type,
            project_id,
        )
        .fetch_one(&self.pool)
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
        let mut map = std::collections::HashMap::<String, Vec<String>>::new();
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
            return Ok(std::collections::HashMap::<String, Vec<(String, String)>>::new());
        }
        let ids = note_ids
            .iter()
            .map(|id| parse_uuid(id))
            .collect::<Result<Vec<_>, _>>()?;
        let rows = sqlx::query(
            "SELECT note_id::text, type, value FROM note_extractions WHERE type = ANY($1) AND note_id = ANY($2) ORDER BY type, value",
        )
        .bind(extraction_types)
        .bind(&ids)
        .fetch_all(&self.pool)
        .await?;
        let mut map = std::collections::HashMap::<String, Vec<(String, String)>>::new();
        for row in rows {
            let note_id: String = row.try_get(0)?;
            let ext_type: String = row.try_get(1)?;
            let value: String = row.try_get(2)?;
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
        let note_uuid = parse_uuid(note_id)?;
        // Delete all existing rows for this note + type
        sqlx::query("DELETE FROM note_extractions WHERE note_id = $1 AND type = $2")
            .bind(note_uuid)
            .bind(extraction_type)
            .execute(&self.pool)
            .await?;
        // Insert new values
        for value in values {
            sqlx::query(
                "INSERT INTO note_extractions (note_id, user_id, type, value) VALUES ($1, (SELECT user_id FROM notes WHERE id = $1), $2, $3)",
            )
            .bind(note_uuid)
            .bind(extraction_type)
            .bind(value)
            .execute(&self.pool)
            .await?;
        }
        Ok(())
    }

    async fn resolve_prompt_id(&self, prefix: &str) -> Result<String, CliError> {
        crate::backend::validate_id_prefix(prefix)?;
        resolve_uuid_prefix(
            &self.pool,
            "SELECT id::text FROM prompts WHERE id::text LIKE $1 LIMIT 2",
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
        sqlx::query(
            "INSERT INTO prompts (id, title, description, prompt, created_at) VALUES ($1, $2, $3, $4, $5)",
        )
        .bind(parse_uuid(id)?)
        .bind(title)
        .bind(description)
        .bind(prompt)
        .bind(parse_iso_utc(now)?)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn find_prompt(&self, id: &str) -> Result<Prompt, CliError> {
        sqlx::query_as::<_, PromptPgRow>(PG_FIND_PROMPT)
            .bind(parse_uuid(id)?)
            .fetch_optional(&self.pool)
            .await?
            .map(Prompt::from)
            .ok_or_else(|| CliError::Other(format!("Prompt not found: {id}")))
    }

    async fn list_prompts(&self) -> Result<Vec<Prompt>, CliError> {
        let rows = sqlx::query_as::<_, PromptPgRow>(PG_LIST_PROMPTS)
            .fetch_all(&self.pool)
            .await?;
        Ok(rows.into_iter().map(Prompt::from).collect())
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

        let prompt_id = parse_uuid(id)?;
        let result = sqlx::query!(
            r#"
            UPDATE prompts SET
                title = CASE WHEN $2::bool THEN $3::text ELSE title END,
                description = CASE WHEN $4::bool THEN $5::text ELSE description END,
                prompt = CASE WHEN $6::bool THEN $7::text ELSE prompt END
            WHERE id = $1
            "#,
            prompt_id,
            update_title,
            title,
            update_description,
            description,
            update_prompt,
            prompt,
        )
        .execute(&self.pool)
        .await?;
        if result.rows_affected() == 0 {
            return Err(CliError::Other(format!("Prompt not found: {id}")));
        }
        Ok(())
    }

    async fn delete_prompt(&self, id: &str) -> Result<(), CliError> {
        let result = sqlx::query("DELETE FROM prompts WHERE id = $1")
            .bind(parse_uuid(id)?)
            .execute(&self.pool)
            .await?;
        if result.rows_affected() == 0 {
            return Err(CliError::Other(format!("Prompt not found: {id}")));
        }
        Ok(())
    }

    async fn resolve_keyterm_id(&self, prefix: &str) -> Result<String, CliError> {
        crate::backend::validate_id_prefix(prefix)?;
        resolve_uuid_prefix(
            &self.pool,
            "SELECT id::text FROM keyterms WHERE id::text LIKE $1 LIMIT 2",
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
        let now = parse_iso_utc(now)?;
        sqlx::query(
            "INSERT INTO keyterms (id, name, description, content, created_at, updated_at) \
             VALUES ($1, $2, $3, $4, $5, $6)",
        )
        .bind(parse_uuid(id)?)
        .bind(name)
        .bind(description)
        .bind(content)
        .bind(now)
        .bind(now)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn find_keyterm(&self, id: &str) -> Result<Keyterm, CliError> {
        sqlx::query_as::<_, KeytermPgRow>(PG_FIND_KEYTERM)
            .bind(parse_uuid(id)?)
            .fetch_optional(&self.pool)
            .await?
            .map(Keyterm::from)
            .ok_or_else(|| CliError::Other(format!("Keyterm not found: {id}")))
    }

    async fn list_keyterms(&self) -> Result<Vec<Keyterm>, CliError> {
        let rows = sqlx::query_as::<_, KeytermPgRow>(PG_LIST_KEYTERMS)
            .fetch_all(&self.pool)
            .await?;
        Ok(rows.into_iter().map(Keyterm::from).collect())
    }

    async fn update_keyterm(
        &self,
        id: &str,
        name: Option<&str>,
        description: Option<&str>,
        content: Option<&str>,
    ) -> Result<(), CliError> {
        let update_name = name.is_some();
        let update_description = description.is_some();
        let update_content = content.is_some();
        if !(update_name || update_description || update_content) {
            return Ok(());
        }

        let keyterm_id = parse_uuid(id)?;
        let now = Utc::now();
        let result = sqlx::query!(
            r#"
            UPDATE keyterms SET
                name = CASE WHEN $2::bool THEN $3::text ELSE name END,
                description = CASE WHEN $4::bool THEN $5::text ELSE description END,
                content = CASE WHEN $6::bool THEN $7::text ELSE content END,
                updated_at = CASE WHEN ($2::bool OR $4::bool OR $6::bool) THEN $8::timestamptz ELSE updated_at END
            WHERE id = $1
            "#,
            keyterm_id,
            update_name,
            name,
            update_description,
            description,
            update_content,
            content,
            now,
        )
        .execute(&self.pool)
        .await?;
        if result.rows_affected() == 0 {
            return Err(CliError::Other(format!("Keyterm not found: {id}")));
        }
        Ok(())
    }

    async fn delete_keyterm(&self, id: &str) -> Result<(), CliError> {
        let result = sqlx::query("DELETE FROM keyterms WHERE id = $1")
            .bind(parse_uuid(id)?)
            .execute(&self.pool)
            .await?;
        if result.rows_affected() == 0 {
            return Err(CliError::Other(format!("Keyterm not found: {id}")));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_uuid_valid() {
        let id = "550e8400-e29b-41d4-a716-446655440000";
        let result = parse_uuid(id);
        assert!(result.is_ok());
        assert_eq!(result.unwrap().to_string(), id);
    }

    #[test]
    fn test_parse_uuid_invalid() {
        let result = parse_uuid("not-a-uuid");
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_uuid_opt_some() {
        let id = "550e8400-e29b-41d4-a716-446655440000";
        let result = parse_uuid_opt(Some(id));
        assert!(result.is_ok());
        assert_eq!(result.unwrap().unwrap().to_string(), id);
    }

    #[test]
    fn test_parse_uuid_opt_none() {
        let result = parse_uuid_opt(None);
        assert!(result.is_ok());
        assert!(result.unwrap().is_none());
    }

    #[test]
    fn test_parse_iso_utc_valid() {
        let ts = "2026-04-08T12:00:00Z";
        let result = parse_iso_utc(ts);
        assert!(result.is_ok());
        let dt = result.unwrap();
        assert_eq!(
            dt.format("%Y-%m-%dT%H:%M:%S").to_string(),
            "2026-04-08T12:00:00"
        );
    }

    #[test]
    fn test_parse_iso_utc_with_offset() {
        let ts = "2026-04-08T14:00:00+02:00";
        let result = parse_iso_utc(ts);
        assert!(result.is_ok());
        let dt = result.unwrap();
        assert_eq!(
            dt.format("%Y-%m-%dT%H:%M:%S").to_string(),
            "2026-04-08T12:00:00"
        );
    }

    #[test]
    fn test_parse_iso_utc_invalid() {
        let result = parse_iso_utc("not-a-timestamp");
        assert!(result.is_err());
    }

    #[test]
    fn test_note_pg_row_from() {
        use chrono::TimeZone;
        let pg_row = NotePgRow {
            id: Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap(),
            user_id: Uuid::nil(),
            r#type: "text".into(),
            status: "active".into(),
            title: Some("Test".into()),
            content: None,
            summary: None,
            is_flagged: Some(true),
            project_id: None,
            metadata: Some(serde_json::json!({"key": "value"})),
            source: None,
            external_id: None,
            created_at: Utc.with_ymd_and_hms(2026, 4, 8, 12, 0, 0).single(),
            updated_at: None,
            deleted_at: None,
        };
        let note: Note = pg_row.into();
        assert_eq!(note.id, "550e8400-e29b-41d4-a716-446655440000");
        assert_eq!(note.is_flagged, Some(1));
        assert!(note.created_at.is_some());
        assert!(note.metadata.is_some());
    }

    #[test]
    fn test_project_pg_row_from() {
        use chrono::TimeZone;
        let pg_row = ProjectPgRow {
            id: Uuid::parse_str("550e8400-e29b-41d4-a716-446655440001").unwrap(),
            user_id: Uuid::nil(),
            name: "My Project".into(),
            color: Some("#ff0000".into()),
            prompt_id: None,
            keyterm_id: None,
            is_archived: Some(false),
            created_at: Utc.with_ymd_and_hms(2026, 4, 8, 12, 0, 0).single(),
        };
        let project: Project = pg_row.into();
        assert_eq!(project.id, "550e8400-e29b-41d4-a716-446655440001");
        assert_eq!(project.name, "My Project");
        assert_eq!(project.is_archived, Some(0));
        assert!(project.created_at.is_some());
    }

    #[test]
    fn test_prompt_pg_row_from() {
        use chrono::TimeZone;
        let pg_row = PromptPgRow {
            id: Uuid::parse_str("550e8400-e29b-41d4-a716-446655440002").unwrap(),
            user_id: Uuid::nil(),
            title: "Summarize".into(),
            description: Some("Give a brief summary".into()),
            prompt: "Summarize this text: {{text}}".into(),
            created_at: Utc.with_ymd_and_hms(2026, 4, 8, 12, 0, 0).single(),
        };
        let p: Prompt = pg_row.into();
        assert_eq!(p.id, "550e8400-e29b-41d4-a716-446655440002");
        assert_eq!(p.title, "Summarize");
        assert!(p.description.is_some());
        assert!(p.created_at.is_some());
    }

    #[test]
    fn test_keyterm_pg_row_from() {
        use chrono::TimeZone;
        let pg_row = KeytermPgRow {
            id: Uuid::parse_str("550e8400-e29b-41d4-a716-446655440003").unwrap(),
            user_id: Uuid::nil(),
            name: "TODO".into(),
            description: Some("Action items".into()),
            content: Some("topics".into()),
            created_at: Utc.with_ymd_and_hms(2026, 4, 8, 12, 0, 0).single(),
            updated_at: Utc.with_ymd_and_hms(2026, 4, 9, 10, 0, 0).single(),
        };
        let k: Keyterm = pg_row.into();
        assert_eq!(k.id, "550e8400-e29b-41d4-a716-446655440003");
        assert_eq!(k.name, "TODO");
        assert!(k.description.is_some());
        assert!(k.created_at.is_some());
        assert!(k.updated_at.is_some());
    }
}
