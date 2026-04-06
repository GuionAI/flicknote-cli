use std::cell::RefCell;

use base64::Engine;
use postgres::types::ToSql;

use crate::backend::{InsertNoteReq, NoteDb, NoteFilter};
use crate::error::CliError;
use crate::types::{Keyterm, Note, Project, Prompt};

pub struct PgWireBackend {
    client: RefCell<postgres::Client>,
    user_id: String,
}

impl PgWireBackend {
    pub fn connect(database_url: &str, token: &str) -> Result<Self, CliError> {
        let user_id = parse_jwt_sub(token).map_err(|e| CliError::Database(e.to_string()))?;
        let client = postgres::Client::connect(database_url, postgres::NoTls)
            .map_err(|e| CliError::Database(format!("connection failed: {e}")))?;
        Ok(Self {
            client: RefCell::new(client),
            user_id,
        })
    }

    fn exec(&self, sql: &str, params: &[&(dyn ToSql + Sync)]) -> Result<u64, CliError> {
        let mut c = self.client.borrow_mut();
        c.execute(sql, params)
            .map_err(|e| CliError::Database(e.to_string()))
    }

    fn exec_opt<T>(
        &self,
        sql: &str,
        params: &[&(dyn ToSql + Sync)],
        none_err: CliError,
        mut f: impl FnMut(&postgres::Row) -> Result<T, CliError>,
    ) -> Result<T, CliError> {
        let mut c = self.client.borrow_mut();
        let rows = c
            .query(sql, params)
            .map_err(|e| CliError::Database(e.to_string()))?;
        match rows.first() {
            Some(r) => f(r),
            None => Err(none_err),
        }
    }

    fn exec_all<T>(
        &self,
        sql: &str,
        params: &[&(dyn ToSql + Sync)],
        mut f: impl FnMut(&postgres::Row) -> Result<T, CliError>, // lint: FnMut required for .map(), mut on binding needed
    ) -> Result<Vec<T>, CliError> {
        let mut c = self.client.borrow_mut();
        let rows = c
            .query(sql, params)
            .map_err(|e| CliError::Database(e.to_string()))?;
        rows.iter().map(&mut f).collect()
    }
}

impl NoteDb for PgWireBackend {
    fn user_id(&self) -> &str {
        &self.user_id
    }

    fn resolve_note_id(&self, prefix: &str) -> Result<String, CliError> {
        crate::backend::validate_id_prefix(prefix)?;
        let pattern = format!("{prefix}%");
        let rows = self.exec_all(
            "SELECT id FROM notes WHERE id LIKE $1 AND deleted_at IS NULL LIMIT 2",
            &[&pattern],
            |r| Ok(r.get::<_, String>(0)),
        )?;
        match rows.len() {
            1 => Ok(rows.into_iter().next().unwrap()),
            0 => Err(CliError::NoteNotFound {
                id: prefix.to_string(),
            }),
            _ => Err(CliError::Other(format!("Ambiguous ID prefix: {prefix}"))),
        }
    }

    fn resolve_archived_note_id(&self, prefix: &str) -> Result<String, CliError> {
        crate::backend::validate_id_prefix(prefix)?;
        let pattern = format!("{prefix}%");
        let rows = self.exec_all(
            "SELECT id FROM notes WHERE id LIKE $1 AND deleted_at IS NOT NULL LIMIT 2",
            &[&pattern],
            |r| Ok(r.get::<_, String>(0)),
        )?;
        match rows.len() {
            1 => Ok(rows.into_iter().next().unwrap()),
            0 => Err(CliError::NoteNotFound {
                id: prefix.to_string(),
            }),
            _ => Err(CliError::Other(format!("Ambiguous ID prefix: {prefix}"))),
        }
    }

    fn find_note(&self, id: &str) -> Result<Note, CliError> {
        self.exec_opt(
            "SELECT id, user_id, type, status, title, content, summary, is_flagged, \
             project_id, metadata, source, external_id, created_at, updated_at, deleted_at \
             FROM notes WHERE id = $1::text::uuid AND deleted_at IS NULL LIMIT 1",
            &[&id],
            CliError::NoteNotFound { id: id.to_string() },
            Note::from_pg_row,
        )
    }

    fn find_archived_note(&self, id: &str) -> Result<Note, CliError> {
        self.exec_opt(
            "SELECT id, user_id, type, status, title, content, summary, is_flagged, \
             project_id, metadata, source, external_id, created_at, updated_at, deleted_at \
             FROM notes WHERE id = $1::text::uuid AND deleted_at IS NOT NULL LIMIT 1",
            &[&id],
            CliError::NoteNotFound { id: id.to_string() },
            Note::from_pg_row,
        )
    }

    fn find_note_content(&self, id: &str) -> Result<Option<String>, CliError> {
        let rows = self.exec_all(
            "SELECT content FROM notes WHERE id = $1::text::uuid AND deleted_at IS NULL LIMIT 1",
            &[&id],
            |r| Ok(r.get::<_, Option<String>>(0)),
        )?;
        Ok(rows.into_iter().next().unwrap_or(None))
    }

    fn list_notes(&self, filter: &NoteFilter<'_>) -> Result<Vec<Note>, CliError> {
        let archive_cond = if filter.archived {
            "deleted_at IS NOT NULL"
        } else {
            "deleted_at IS NULL"
        };
        let mut sql = format!(
            "SELECT id, user_id, type, status, title, content, summary, is_flagged, \
             project_id, metadata, source, external_id, created_at, updated_at, deleted_at \
             FROM notes WHERE {archive_cond}"
        );
        let mut params: Vec<Box<dyn ToSql + Sync>> = vec![];

        if let Some(t) = filter.note_type {
            params.push(Box::new(t.to_string()));
            sql.push_str(&format!(" AND type = ${}", params.len()));
        }
        if let Some(pid) = filter.project_id {
            params.push(Box::new(pid.to_string()));
            sql.push_str(&format!(" AND project_id = ${}::text::uuid", params.len()));
        }
        params.push(Box::new(i64::from(filter.limit)));
        sql.push_str(&format!(
            " ORDER BY created_at DESC LIMIT ${}",
            params.len()
        ));

        let refs: Vec<&(dyn ToSql + Sync)> = params.iter().map(AsRef::as_ref).collect();
        self.exec_all(&sql, &refs, Note::from_pg_row)
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
            .enumerate()
            .map(|(i, _)| {
                let base = i * 3 + 1;
                format!(
                    "(title ILIKE ${} OR content ILIKE ${} OR summary ILIKE ${})",
                    base,
                    base + 1,
                    base + 2
                )
            })
            .collect();
        let keywords_clause = keyword_blocks.join(" OR ");
        let mut sql = format!(
            "SELECT id, user_id, type, status, title, content, summary, is_flagged, \
             project_id, metadata, source, external_id, created_at, updated_at, deleted_at \
             FROM notes WHERE {archive_cond} AND ({keywords_clause})"
        );
        let mut params: Vec<Box<dyn ToSql + Sync>> = vec![];

        for kw in keywords {
            params.push(Box::new(format!("%{kw}%")));
            params.push(Box::new(format!("%{kw}%")));
            params.push(Box::new(format!("%{kw}%")));
        }
        if let Some(ref pid) = filter.project_id {
            params.push(Box::new(pid.to_string()));
            sql.push_str(&format!(" AND project_id = ${}::text::uuid", params.len()));
        }
        params.push(Box::new(i64::from(filter.limit)));
        sql.push_str(&format!(
            " ORDER BY updated_at DESC LIMIT ${}",
            params.len()
        ));

        let refs: Vec<&(dyn ToSql + Sync)> = params.iter().map(AsRef::as_ref).collect();
        self.exec_all(&sql, &refs, Note::from_pg_row)
    }

    fn insert_note(&self, req: &InsertNoteReq<'_>) -> Result<(), CliError> {
        let metadata_json: Option<serde_json::Value> = req
            .metadata
            .map(serde_json::from_str)
            .transpose()
            .map_err(|e| CliError::Database(format!("invalid metadata JSON: {e}")))?;

        self.exec(
            "INSERT INTO notes \
             (id, user_id, type, status, title, content, metadata, project_id, created_at, updated_at) \
             VALUES ($1::text::uuid, $2::text::uuid, $3, $4, $5, $6, $7, $8::text::uuid, ($9 || '')::timestamptz, ($10 || '')::timestamptz)",
            &[&req.id, &self.user_id, &req.note_type, &req.status, &req.title, &req.content,
              &metadata_json, &req.project_id, &req.now, &req.now],
        )?;
        Ok(())
    }

    fn update_note_content(&self, id: &str, content: &str, requeue: bool) -> Result<(), CliError> {
        let now = chrono::Utc::now().to_rfc3339();
        let sql = if requeue {
            "UPDATE notes SET content = $1, status = 'ai_queued', updated_at = $2 WHERE id = $3::text::uuid"
        } else {
            "UPDATE notes SET content = $1, updated_at = $2 WHERE id = $3::text::uuid"
        };
        let affected = self.exec(sql, &[&content, &now, &id])?;
        if affected == 0 {
            return Err(CliError::NoteNotFound { id: id.to_string() });
        }
        Ok(())
    }

    fn set_note_deleted_at(
        &self,
        id: &str,
        deleted_at: Option<&str>,
        now: &str,
    ) -> Result<(), CliError> {
        let affected = if let Some(ts) = deleted_at {
            self.exec(
                "UPDATE notes SET deleted_at = $1, updated_at = $2 WHERE id = $3::text::uuid",
                &[&ts, &now, &id],
            )?
        } else {
            self.exec(
                "UPDATE notes SET deleted_at = NULL, updated_at = $1 WHERE id = $2::text::uuid",
                &[&now, &id],
            )?
        };
        if affected == 0 {
            return Err(CliError::NoteNotFound { id: id.to_string() });
        }
        Ok(())
    }

    fn undo_last_delete(&self) -> Result<(), CliError> {
        let now = chrono::Utc::now().to_rfc3339();
        self.exec(
            "UPDATE notes SET deleted_at = NULL, updated_at = $1 \
             WHERE id = (SELECT id FROM notes WHERE deleted_at IS NOT NULL \
             ORDER BY deleted_at DESC LIMIT 1)",
            &[&now],
        )?;
        Ok(())
    }

    fn find_project_by_name(&self, name: &str) -> Result<Option<String>, CliError> {
        let rows = self.exec_all(
            "SELECT id FROM projects WHERE user_id = $1::text::uuid AND name = $2 AND (is_archived IS NOT TRUE) LIMIT 1",
            &[&self.user_id, &name],
            |r| Ok(r.get::<_, String>(0)),
        )?;
        Ok(rows.into_iter().next())
    }

    fn find_project_name_by_id(&self, project_id: &str) -> Result<Option<String>, CliError> {
        let rows = self.exec_all(
            "SELECT name FROM projects WHERE user_id = $1::text::uuid AND id = $2::text::uuid LIMIT 1",
            &[&self.user_id, &project_id],
            |r| Ok(r.get::<_, Option<String>>(0)),
        )?;
        Ok(rows.into_iter().next().unwrap_or(None))
    }

    fn list_projects(&self, archived: bool) -> Result<Vec<Project>, CliError> {
        let sql = if archived {
            "SELECT id::text AS id, user_id::text AS user_id, name, color, \
             prompt_id::text AS prompt_id, keyterm_id::text AS keyterm_id, \
             (CASE WHEN is_archived THEN 1 ELSE 0 END) AS is_archived, created_at::text AS created_at \
             FROM projects WHERE user_id = $1::text::uuid AND is_archived IS TRUE ORDER BY name"
        } else {
            "SELECT id::text AS id, user_id::text AS user_id, name, color, \
             prompt_id::text AS prompt_id, keyterm_id::text AS keyterm_id, \
             (CASE WHEN is_archived THEN 1 ELSE 0 END) AS is_archived, created_at::text AS created_at \
             FROM projects WHERE user_id = $1::text::uuid AND is_archived IS NOT TRUE ORDER BY name"
        };
        self.exec_all(sql, &[&self.user_id], Project::from_pg_row)
    }

    fn create_project(&self, name: &str) -> Result<String, CliError> {
        let id = uuid::Uuid::new_v4().to_string();
        let now = chrono::Utc::now().to_rfc3339();
        self.exec(
            "INSERT INTO projects (id, user_id, name, is_archived, created_at) VALUES ($1::text::uuid, $2::text::uuid, $3, FALSE, $4)",
            &[&id, &self.user_id, &name, &now],
        )?;
        Ok(id)
    }

    fn move_note_to_project(
        &self,
        note_id: &str,
        new_project_id: &str,
        old_project_id: Option<&str>,
    ) -> Result<Option<String>, CliError> {
        let now = chrono::Utc::now().to_rfc3339();
        let mut c = self.client.borrow_mut();
        let mut tx = c
            .transaction()
            .map_err(|e| CliError::Database(e.to_string()))?;

        let affected = tx
            .execute(
                "UPDATE notes SET project_id = $1::text::uuid, updated_at = $2 WHERE id = $3::text::uuid",
                &[&new_project_id, &now, &note_id],
            )
            .map_err(|e| CliError::Database(e.to_string()))?;
        if affected == 0 {
            return Err(CliError::NoteNotFound {
                id: note_id.to_string(),
            });
        }

        let Some(old_pid) = old_project_id else {
            tx.commit().map_err(|e| CliError::Database(e.to_string()))?;
            return Ok(None);
        };

        let count: i64 = tx
            .query_one(
                "SELECT COUNT(*) FROM notes WHERE project_id = $1::text::uuid AND deleted_at IS NULL",
                &[&old_pid],
            )
            .map_err(|e| CliError::Database(e.to_string()))?
            .try_get(0)
            .map_err(|e| CliError::Database(e.to_string()))?;

        if count == 0 {
            let old_name: Option<String> = tx
                .query_one(
                    "SELECT name FROM projects WHERE id = $1::text::uuid",
                    &[&old_pid],
                )
                .map_err(|e| CliError::Database(e.to_string()))?
                .try_get(0)
                .map_err(|e| CliError::Database(e.to_string()))?;
            tx.execute(
                "DELETE FROM projects WHERE id = $1::text::uuid",
                &[&old_pid],
            )
            .map_err(|e| CliError::Database(e.to_string()))?;
            tx.commit().map_err(|e| CliError::Database(e.to_string()))?;
            Ok(old_name)
        } else {
            tx.commit().map_err(|e| CliError::Database(e.to_string()))?;
            Ok(None)
        }
    }

    fn find_project(&self, id: &str) -> Result<Project, CliError> {
        self.exec_opt(
            "SELECT id::text AS id, user_id::text AS user_id, name, color, \
             prompt_id::text AS prompt_id, keyterm_id::text AS keyterm_id, \
             (CASE WHEN is_archived THEN 1 ELSE 0 END) AS is_archived, created_at::text AS created_at \
             FROM projects WHERE user_id = $1::text::uuid AND id = $2::text::uuid LIMIT 1",
            &[&self.user_id, &id],
            CliError::Other(format!("Project not found: {id}")),
            Project::from_pg_row,
        )
    }

    fn resolve_project_id(&self, prefix: &str) -> Result<String, CliError> {
        crate::backend::validate_id_prefix(prefix)?;
        let pattern = format!("{prefix}%");
        let rows = self.exec_all(
            "SELECT id FROM projects WHERE user_id = $1::text::uuid AND id LIKE $2 LIMIT 2",
            &[&self.user_id, &pattern],
            |r| Ok(r.get::<_, String>(0)),
        )?;
        match rows.len() {
            1 => Ok(rows.into_iter().next().unwrap()),
            0 => Err(CliError::Other(format!("Project not found: {prefix}"))),
            _ => Err(CliError::Other(format!(
                "Ambiguous project ID prefix: {prefix}"
            ))),
        }
    }

    fn update_project(
        &self,
        id: &str,
        prompt_id: Option<Option<&str>>,
        keyterm_id: Option<Option<&str>>,
        color: Option<Option<&str>>,
    ) -> Result<(), CliError> {
        let mut updates: Vec<String> = vec![];
        let mut params: Vec<Box<dyn ToSql + Sync>> = vec![];

        if let Some(val) = prompt_id {
            if let Some(v) = val {
                params.push(Box::new(Some(v.to_string())));
            } else {
                params.push(Box::new(None::<String>));
            }
            updates.push(format!("prompt_id = ${}", params.len()));
        }
        if let Some(val) = keyterm_id {
            if let Some(v) = val {
                params.push(Box::new(Some(v.to_string())));
            } else {
                params.push(Box::new(None::<String>));
            }
            updates.push(format!("keyterm_id = ${}", params.len()));
        }
        if let Some(val) = color {
            if let Some(v) = val {
                params.push(Box::new(Some(v.to_string())));
            } else {
                params.push(Box::new(None::<String>));
            }
            updates.push(format!("color = ${}", params.len()));
        }

        if updates.is_empty() {
            return Ok(());
        }

        params.push(Box::new(id.to_string()));
        let sql = format!(
            "UPDATE projects SET {} WHERE id = ${}::text::uuid",
            updates.join(", "),
            params.len()
        );
        let refs: Vec<&(dyn ToSql + Sync)> = params.iter().map(AsRef::as_ref).collect();
        let mut c = self.client.borrow_mut();
        c.execute(&sql, &refs)
            .map_err(|e| CliError::Database(e.to_string()))?;
        Ok(())
    }

    fn delete_project(&self, id: &str) -> Result<(), CliError> {
        let affected = self.exec("UPDATE projects SET is_archived = TRUE WHERE user_id = $1::text::uuid AND id = $2::text::uuid", &[&self.user_id, &id])?;
        if affected == 0 {
            return Err(CliError::Other(format!("Project not found: {id}")));
        }
        Ok(())
    }

    fn update_note_title(&self, id: &str, title: &str) -> Result<(), CliError> {
        let now = chrono::Utc::now().to_rfc3339();
        let affected = self.exec(
            "UPDATE notes SET title = $1, updated_at = $2 WHERE id = $3::text::uuid",
            &[&title, &now, &id],
        )?;
        if affected == 0 {
            return Err(CliError::NoteNotFound { id: id.to_string() });
        }
        Ok(())
    }

    fn update_note_flagged(&self, id: &str, flagged: bool) -> Result<(), CliError> {
        let now = chrono::Utc::now().to_rfc3339();
        let val: i64 = if flagged { 1 } else { 0 };
        let affected = self.exec(
            "UPDATE notes SET is_flagged = $1, updated_at = $2 WHERE id = $3::text::uuid",
            &[&val, &now, &id],
        )?;
        if affected == 0 {
            return Err(CliError::NoteNotFound { id: id.to_string() });
        }
        Ok(())
    }

    fn count_notes(&self, filter: &NoteFilter<'_>) -> Result<u64, CliError> {
        let archive_cond = if filter.archived {
            "deleted_at IS NOT NULL"
        } else {
            "deleted_at IS NULL"
        };
        let mut sql = format!("SELECT COUNT(*) FROM notes WHERE {archive_cond}");
        let mut params: Vec<Box<dyn ToSql + Sync>> = vec![];

        if let Some(t) = filter.note_type {
            params.push(Box::new(t.to_string()));
            sql.push_str(&format!(" AND type = ${}", params.len()));
        }
        if let Some(pid) = filter.project_id {
            params.push(Box::new(pid.to_string()));
            sql.push_str(&format!(" AND project_id = ${}::text::uuid", params.len()));
        }

        let refs: Vec<&(dyn ToSql + Sync)> = params.iter().map(AsRef::as_ref).collect();
        let count: i64 = self.exec_opt(
            &sql,
            &refs,
            CliError::Other("count query failed".into()),
            |r| Ok(r.get::<_, i64>(0)),
        )?;
        count
            .try_into()
            .map_err(|_| CliError::Other(format!("unexpected negative count: {count}")))
    }

    fn list_note_topics(
        &self,
        note_ids: &[&str],
    ) -> Result<std::collections::HashMap<String, Vec<String>>, CliError> {
        if note_ids.is_empty() {
            return Ok(std::collections::HashMap::new());
        }
        let placeholders: Vec<String> = (1..=note_ids.len())
            .map(|i| format!("${}::text::uuid", i))
            .collect();
        let sql = format!(
            "SELECT note_id, value FROM note_extractions WHERE type = 'topic' AND note_id IN ({})",
            placeholders.join(", ")
        );
        let params: Vec<Box<&str>> = note_ids.iter().map(|id| Box::new(*id)).collect();
        let refs: Vec<&(dyn ToSql + Sync)> = params.iter().map(|p| p.as_ref() as _).collect();
        self.exec_all(&sql, &refs, |r| {
            Ok((r.get::<_, String>(0), r.get::<_, String>(1)))
        })
        .map(|rows| {
            let mut map: std::collections::HashMap<String, Vec<String>> =
                std::collections::HashMap::new();
            for (note_id, value) in rows {
                map.entry(note_id).or_default().push(value);
            }
            map
        })
    }

    fn resolve_prompt_id(&self, prefix: &str) -> Result<String, CliError> {
        crate::backend::validate_id_prefix(prefix)?;
        let pattern = format!("{prefix}%");
        let rows = self.exec_all(
            "SELECT id FROM prompts WHERE id LIKE $1 LIMIT 2",
            &[&pattern],
            |r| Ok(r.get::<_, String>(0)),
        )?;
        match rows.len() {
            1 => Ok(rows.into_iter().next().unwrap()),
            0 => Err(CliError::Other(format!("Prompt not found: {prefix}"))),
            _ => Err(CliError::Other(format!(
                "Ambiguous prompt ID prefix: {prefix}"
            ))),
        }
    }

    fn insert_prompt(
        &self,
        id: &str,
        title: &str,
        description: Option<&str>,
        prompt: &str,
        now: &str,
    ) -> Result<(), CliError> {
        self.exec(
            "INSERT INTO prompts (id, user_id, title, description, prompt, created_at) VALUES ($1, $2, $3, $4, $5, $6)",
            &[&id, &self.user_id, &title, &description, &prompt, &now],
        )?;
        Ok(())
    }

    fn find_prompt(&self, id: &str) -> Result<Prompt, CliError> {
        self.exec_opt(
            "SELECT id, user_id, title, description, prompt, created_at FROM prompts WHERE id = $1 LIMIT 1",
            &[&id],
            CliError::Other(format!("Prompt not found: {id}")),
            Prompt::from_pg_row,
        )
    }

    fn list_prompts(&self) -> Result<Vec<Prompt>, CliError> {
        self.exec_all(
            "SELECT id, user_id, title, description, prompt, created_at FROM prompts ORDER BY created_at DESC",
            &[],
            Prompt::from_pg_row,
        )
    }

    fn update_prompt(
        &self,
        id: &str,
        title: Option<&str>,
        description: Option<&str>,
        prompt: Option<&str>,
    ) -> Result<(), CliError> {
        let mut updates: Vec<String> = vec![];
        let mut params: Vec<Box<dyn ToSql + Sync>> = vec![];

        if let Some(v) = title {
            params.push(Box::new(v.to_string()));
            updates.push(format!("title = ${}", params.len()));
        }
        if let Some(v) = description {
            params.push(Box::new(v.to_string()));
            updates.push(format!("description = ${}", params.len()));
        }
        if let Some(v) = prompt {
            params.push(Box::new(v.to_string()));
            updates.push(format!("prompt = ${}", params.len()));
        }

        if updates.is_empty() {
            return Ok(());
        }

        params.push(Box::new(id.to_string()));
        let sql = format!(
            "UPDATE prompts SET {} WHERE id = ${}",
            updates.join(", "),
            params.len()
        );
        let refs: Vec<&(dyn ToSql + Sync)> = params.iter().map(AsRef::as_ref).collect();
        let mut c = self.client.borrow_mut();
        c.execute(&sql, &refs)
            .map_err(|e| CliError::Database(e.to_string()))?;
        Ok(())
    }

    fn delete_prompt(&self, id: &str) -> Result<(), CliError> {
        self.exec("DELETE FROM prompts WHERE id = $1", &[&id])?;
        Ok(())
    }

    fn resolve_keyterm_id(&self, prefix: &str) -> Result<String, CliError> {
        crate::backend::validate_id_prefix(prefix)?;
        let pattern = format!("{prefix}%");
        let rows = self.exec_all(
            "SELECT id FROM keyterms WHERE id LIKE $1 LIMIT 2",
            &[&pattern],
            |r| Ok(r.get::<_, String>(0)),
        )?;
        match rows.len() {
            1 => Ok(rows.into_iter().next().unwrap()),
            0 => Err(CliError::Other(format!("Keyterm not found: {prefix}"))),
            _ => Err(CliError::Other(format!(
                "Ambiguous keyterm ID prefix: {prefix}"
            ))),
        }
    }

    fn insert_keyterm(
        &self,
        id: &str,
        name: &str,
        description: Option<&str>,
        content: Option<&str>,
        now: &str,
    ) -> Result<(), CliError> {
        self.exec(
            "INSERT INTO keyterms (id, user_id, name, description, content, created_at, updated_at) VALUES ($1, $2, $3, $4, $5, $6, $7)",
            &[&id, &self.user_id, &name, &description, &content, &now, &now],
        )?;
        Ok(())
    }

    fn find_keyterm(&self, id: &str) -> Result<Keyterm, CliError> {
        self.exec_opt(
            "SELECT id, user_id, name, description, content, created_at, updated_at FROM keyterms WHERE id = $1 LIMIT 1",
            &[&id],
            CliError::Other(format!("Keyterm not found: {id}")),
            Keyterm::from_pg_row,
        )
    }

    fn list_keyterms(&self) -> Result<Vec<Keyterm>, CliError> {
        self.exec_all(
            "SELECT id, user_id, name, description, content, created_at, updated_at FROM keyterms ORDER BY name",
            &[],
            Keyterm::from_pg_row,
        )
    }

    fn update_keyterm(
        &self,
        id: &str,
        name: Option<&str>,
        description: Option<&str>,
        content: Option<&str>,
    ) -> Result<(), CliError> {
        let now = chrono::Utc::now().to_rfc3339();
        let mut updates: Vec<String> = vec![];
        let mut params: Vec<Box<dyn ToSql + Sync>> = vec![];

        if let Some(v) = name {
            params.push(Box::new(v.to_string()));
            updates.push(format!("name = ${}", params.len()));
        }
        if let Some(v) = description {
            params.push(Box::new(v.to_string()));
            updates.push(format!("description = ${}", params.len()));
        }
        if let Some(v) = content {
            params.push(Box::new(v.to_string()));
            updates.push(format!("content = ${}", params.len()));
        }

        if updates.is_empty() {
            return Ok(());
        }

        params.push(Box::new(now));
        updates.push(format!("updated_at = ${}", params.len()));
        params.push(Box::new(id.to_string()));
        let sql = format!(
            "UPDATE keyterms SET {} WHERE id = ${}",
            updates.join(", "),
            params.len()
        );
        let refs: Vec<&(dyn ToSql + Sync)> = params.iter().map(AsRef::as_ref).collect();
        let mut c = self.client.borrow_mut();
        c.execute(&sql, &refs)
            .map_err(|e| CliError::Database(e.to_string()))?;
        Ok(())
    }

    fn delete_keyterm(&self, id: &str) -> Result<(), CliError> {
        self.exec("DELETE FROM keyterms WHERE id = $1", &[&id])?;
        Ok(())
    }
}

fn parse_jwt_sub(token: &str) -> Result<String, Box<dyn std::error::Error>> {
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() != 3 {
        return Err("Invalid JWT format".into());
    }
    let mut payload_b64 = parts[1].to_string();
    let remainder = payload_b64.len() % 4;
    if remainder > 0 {
        payload_b64.push_str(&"=".repeat(4 - remainder));
    }
    payload_b64 = payload_b64.replace('-', "+").replace('_', "/");
    let payload_json =
        String::from_utf8(base64::engine::general_purpose::STANDARD.decode(&payload_b64)?)?;
    let value: serde_json::Value = serde_json::from_str(&payload_json)?;
    value
        .get("sub")
        .and_then(|v| v.as_str())
        .map(String::from)
        .ok_or_else(|| "Missing 'sub' claim in JWT".into())
}
