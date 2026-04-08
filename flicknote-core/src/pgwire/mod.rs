//! PgWire backend for Supabase Postgres.
//!
//! This backend assumes the connection is routed through pgwire-supabase-proxy
//! (or equivalent) which: (a) sets `request.jwt.claim.sub` GUC from the JWT,
//! (b) `SET ROLE authenticated`, (c) all tables have RLS policies on `auth.uid()`.
//!
//! Therefore: no `user_id` WHERE clauses, no `user_id` INSERT columns.
//! Tenant isolation is enforced by RLS, full stop.

use std::cell::RefCell;

use chrono::{DateTime, Utc};
use sea_query::{
    Alias, Condition, Expr, ExprTrait, IntoColumnRef, Order, PostgresQueryBuilder, Query,
};
use sea_query_postgres::PostgresBinder;
use uuid::Uuid;

use crate::backend::{InsertNoteReq, NoteDb, NoteFilter};
use crate::error::CliError;
use crate::types::{Keyterm, Note, Project, Prompt};

mod iden;
use iden::{Keyterms, NoteExtractions, Notes, Projects, Prompts};

// ─── Helpers ──────────────────────────────────────────────────────────────────

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

/// Cast a jsonb column to text on read so the pgwire client can deserialize it as String.
fn jsonb_read<C>(col: C) -> Expr
where
    C: IntoColumnRef,
{
    Expr::col(col).cast_as(Alias::new("text"))
}

/// Cast a uuid column to text so postgres(with-uuid-1) returns it as String.
fn uuid_read<C>(col: C) -> Expr
where
    C: IntoColumnRef,
{
    Expr::col(col).cast_as(Alias::new("text"))
}

/// Cast a timestamptz column to text so postgres(with-chrono-0_4) returns it as String.
/// Use ISO-8601-ish format compatible with existing sqlite values.
fn ts_read<C>(col: C) -> Expr
where
    C: IntoColumnRef,
{
    Expr::col(col).cast_as(Alias::new("text"))
}

/// Cast a boolean column to bigint so it decodes as Option<i64> (matches sqlite INTEGER 0/1).
fn bool_as_int<C>(col: C) -> Expr
where
    C: IntoColumnRef,
{
    Expr::cust_with_expr("($1)::int::bigint", Expr::col(col))
}

/// Helper to execute a query that expects zero or one result rows.
#[allow(clippy::needless_pass_by_value)]
fn exec_opt<T>(
    client: &mut postgres::Client,
    sql: &str,
    vals: sea_query_postgres::PostgresValues,
    none_err: CliError,
    mut f: impl FnMut(&postgres::Row) -> Result<T, CliError>,
) -> Result<T, CliError> {
    let params = vals.as_params();
    let rows = client
        .query(sql, &params)
        .map_err(|e| CliError::Database(e.to_string()))?;
    match rows.first() {
        Some(r) => f(r),
        None => Err(none_err),
    }
}

/// Helper to execute a query that expects multiple result rows.
#[allow(clippy::needless_pass_by_value)]
fn exec_all<T>(
    client: &mut postgres::Client,
    sql: &str,
    vals: sea_query_postgres::PostgresValues,
    mut f: impl FnMut(&postgres::Row) -> Result<T, CliError>,
) -> Result<Vec<T>, CliError> {
    let params = vals.as_params();
    let rows = client
        .query(sql, &params)
        .map_err(|e| CliError::Database(e.to_string()))?;
    rows.iter().map(&mut f).collect()
}

/// Helper to execute a mutation (INSERT/UPDATE/DELETE).
#[allow(clippy::needless_pass_by_value)]
fn exec_mutation(
    client: &mut postgres::Client,
    sql: &str,
    vals: sea_query_postgres::PostgresValues,
) -> Result<u64, CliError> {
    let params = vals.as_params();
    client
        .execute(sql, &params)
        .map_err(|e| CliError::Database(e.to_string()))
}

// ─── PgWireBackend ────────────────────────────────────────────────────────────

pub struct PgWireBackend {
    client: RefCell<postgres::Client>,
}

impl PgWireBackend {
    pub fn connect(database_url: &str) -> Result<Self, CliError> {
        let client = postgres::Client::connect(database_url, postgres::NoTls)
            .map_err(|e| CliError::Database(format!("connection failed: {e}")))?;
        Ok(Self {
            client: RefCell::new(client),
        })
    }
}

// ─── NoteDb impl ──────────────────────────────────────────────────────────────

impl NoteDb for PgWireBackend {
    fn user_id(&self) -> &str {
        // pgwire identity is enforced by RLS via the connection JWT;
        // fn-cli never needs the raw user_id on this backend.
        ""
    }

    fn resolve_note_id(&self, prefix: &str) -> Result<String, CliError> {
        crate::backend::validate_id_prefix(prefix)?;
        let pattern = format!("{prefix}%");
        let (sql, vals) = Query::select()
            .column(Notes::Id)
            .from(Notes::Table)
            .and_where(
                Expr::col(Notes::Id)
                    .cast_as(Alias::new("text"))
                    .like(&pattern),
            )
            .and_where(Expr::col(Notes::DeletedAt).is_null())
            .limit(2)
            .take()
            .build_postgres(PostgresQueryBuilder);
        let rows = exec_all(&mut self.client.borrow_mut(), sql.as_str(), vals, |r| {
            Ok(r.get::<_, String>(0))
        })?;
        rows.into_iter()
            .next()
            .ok_or_else(|| CliError::NoteNotFound {
                id: prefix.to_string(),
            })
    }

    fn resolve_archived_note_id(&self, prefix: &str) -> Result<String, CliError> {
        crate::backend::validate_id_prefix(prefix)?;
        let pattern = format!("{prefix}%");
        let (sql, vals) = Query::select()
            .column(Notes::Id)
            .from(Notes::Table)
            .and_where(
                Expr::col(Notes::Id)
                    .cast_as(Alias::new("text"))
                    .like(&pattern),
            )
            .and_where(Expr::col(Notes::DeletedAt).is_not_null())
            .limit(2)
            .take()
            .build_postgres(PostgresQueryBuilder);
        let rows = exec_all(&mut self.client.borrow_mut(), sql.as_str(), vals, |r| {
            Ok(r.get::<_, String>(0))
        })?;
        rows.into_iter()
            .next()
            .ok_or_else(|| CliError::NoteNotFound {
                id: prefix.to_string(),
            })
    }

    fn find_note(&self, id: &str) -> Result<Note, CliError> {
        let (sql, vals) = Query::select()
            .columns([
                Notes::Id,
                Notes::UserId,
                Notes::Type,
                Notes::Status,
                Notes::Title,
                Notes::Content,
                Notes::Summary,
                Notes::IsFlagged,
                Notes::ProjectId,
            ])
            .expr_as(jsonb_read(Notes::Metadata), Alias::new("metadata"))
            .columns([
                Notes::Source,
                Notes::ExternalId,
                Notes::CreatedAt,
                Notes::UpdatedAt,
                Notes::DeletedAt,
            ])
            .from(Notes::Table)
            .and_where(Expr::col(Notes::Id).eq(parse_uuid(id)?))
            .and_where(Expr::col(Notes::DeletedAt).is_null())
            .limit(1)
            .take()
            .build_postgres(PostgresQueryBuilder);
        exec_opt(
            &mut self.client.borrow_mut(),
            sql.as_str(),
            vals,
            CliError::NoteNotFound { id: id.to_string() },
            Note::from_pg_row,
        )
    }

    fn find_archived_note(&self, id: &str) -> Result<Note, CliError> {
        let (sql, vals) = Query::select()
            .columns([
                Notes::Id,
                Notes::UserId,
                Notes::Type,
                Notes::Status,
                Notes::Title,
                Notes::Content,
                Notes::Summary,
                Notes::IsFlagged,
                Notes::ProjectId,
            ])
            .expr_as(jsonb_read(Notes::Metadata), Alias::new("metadata"))
            .columns([
                Notes::Source,
                Notes::ExternalId,
                Notes::CreatedAt,
                Notes::UpdatedAt,
                Notes::DeletedAt,
            ])
            .from(Notes::Table)
            .and_where(Expr::col(Notes::Id).eq(parse_uuid(id)?))
            .and_where(Expr::col(Notes::DeletedAt).is_not_null())
            .limit(1)
            .take()
            .build_postgres(PostgresQueryBuilder);
        exec_opt(
            &mut self.client.borrow_mut(),
            sql.as_str(),
            vals,
            CliError::NoteNotFound { id: id.to_string() },
            Note::from_pg_row,
        )
    }

    fn find_note_content(&self, id: &str) -> Result<Option<String>, CliError> {
        let (sql, vals) = Query::select()
            .column(Notes::Content)
            .from(Notes::Table)
            .and_where(Expr::col(Notes::Id).eq(parse_uuid(id)?))
            .and_where(Expr::col(Notes::DeletedAt).is_null())
            .limit(1)
            .take()
            .build_postgres(PostgresQueryBuilder);
        exec_opt(
            &mut self.client.borrow_mut(),
            sql.as_str(),
            vals,
            CliError::NoteNotFound { id: id.to_string() },
            |r| Ok(r.get::<_, Option<String>>(0)),
        )
    }

    fn list_notes(&self, filter: &NoteFilter<'_>) -> Result<Vec<Note>, CliError> {
        let mut q = Query::select();
        q.columns([
            Notes::Id,
            Notes::UserId,
            Notes::Type,
            Notes::Status,
            Notes::Title,
            Notes::Content,
            Notes::Summary,
            Notes::IsFlagged,
            Notes::ProjectId,
        ])
        .expr_as(jsonb_read(Notes::Metadata), Alias::new("metadata"))
        .columns([
            Notes::Source,
            Notes::ExternalId,
            Notes::CreatedAt,
            Notes::UpdatedAt,
            Notes::DeletedAt,
        ])
        .from(Notes::Table)
        .and_where(if filter.archived {
            Expr::col(Notes::DeletedAt).is_not_null()
        } else {
            Expr::col(Notes::DeletedAt).is_null()
        });
        if let Some(t) = filter.note_type {
            q.and_where(Expr::col(Notes::Type).eq(t.to_string()));
        }
        if let Some(pid) = filter.project_id {
            q.and_where(Expr::col(Notes::ProjectId).eq(parse_uuid(pid)?));
        }
        q.order_by(Notes::CreatedAt, Order::Desc)
            .limit(filter.limit as u64);
        let (sql, vals) = q.take().build_postgres(PostgresQueryBuilder);
        exec_all(
            &mut self.client.borrow_mut(),
            sql.as_str(),
            vals,
            Note::from_pg_row,
        )
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
        let mut q = Query::select();
        q.columns([
            Notes::Id,
            Notes::UserId,
            Notes::Type,
            Notes::Status,
            Notes::Title,
            Notes::Content,
            Notes::Summary,
            Notes::IsFlagged,
            Notes::ProjectId,
        ])
        .expr_as(jsonb_read(Notes::Metadata), Alias::new("metadata"))
        .columns([
            Notes::Source,
            Notes::ExternalId,
            Notes::CreatedAt,
            Notes::UpdatedAt,
            Notes::DeletedAt,
        ])
        .from(Notes::Table)
        .and_where(if filter.archived {
            Expr::col(Notes::DeletedAt).is_not_null()
        } else {
            Expr::col(Notes::DeletedAt).is_null()
        });
        // Build OR condition per keyword using Condition::any()
        for kw in keywords {
            let pat = format!("%{kw}%");
            let cond = Condition::any()
                .add(Expr::cust_with_values("title ILIKE ?", [pat.clone()]))
                .add(Expr::cust_with_values("content ILIKE ?", [pat.clone()]))
                .add(Expr::cust_with_values("summary ILIKE ?", [pat]));
            q.cond_where(cond);
        }
        if let Some(pid) = filter.project_id {
            q.and_where(Expr::col(Notes::ProjectId).eq(parse_uuid(pid)?));
        }
        q.order_by(Notes::UpdatedAt, Order::Desc)
            .limit(filter.limit as u64);
        let (sql, vals) = q.take().build_postgres(PostgresQueryBuilder);
        exec_all(
            &mut self.client.borrow_mut(),
            sql.as_str(),
            vals,
            Note::from_pg_row,
        )
    }

    fn insert_note(&self, req: &InsertNoteReq<'_>) -> Result<(), CliError> {
        let metadata_str: Option<String> =
            if let Some(m) = req.metadata {
                let v: serde_json::Value = serde_json::from_str(m)
                    .map_err(|e| CliError::Database(format!("invalid metadata JSON: {e}")))?;
                Some(serde_json::to_string(&v).map_err(|e| {
                    CliError::Database(format!("failed to serialize metadata: {e}"))
                })?)
            } else {
                None
            };
        let now_dt = parse_iso_utc(req.now)?;
        let mut q = Query::insert();
        q.into_table(Notes::Table)
            .columns([
                Notes::Id,
                Notes::Type,
                Notes::Status,
                Notes::Title,
                Notes::Content,
                Notes::Metadata,
                Notes::ProjectId,
                Notes::CreatedAt,
                Notes::UpdatedAt,
            ])
            .values_panic([
                parse_uuid(req.id)?.into(),
                req.note_type.into(),
                req.status.into(),
                req.title.into(),
                req.content.into(),
                metadata_str.into(),
                parse_uuid_opt(req.project_id)?.into(),
                now_dt.into(),
                now_dt.into(),
            ]);
        let (sql, vals) = q.take().build_postgres(PostgresQueryBuilder);
        exec_mutation(&mut self.client.borrow_mut(), sql.as_str(), vals)?;
        Ok(())
    }

    fn update_note_content(&self, id: &str, content: &str, requeue: bool) -> Result<(), CliError> {
        let now = parse_iso_utc(&chrono::Utc::now().to_rfc3339())?;
        let mut q = Query::update();
        q.table(Notes::Table)
            .value(Notes::Content, Some(content))
            .value(Notes::UpdatedAt, now);
        if requeue {
            q.value(Notes::Status, Some("ai_queued"));
        }
        q.and_where(Expr::col(Notes::Id).eq(parse_uuid(id)?));
        let (sql, vals) = q.take().build_postgres(PostgresQueryBuilder);
        let affected = exec_mutation(&mut self.client.borrow_mut(), sql.as_str(), vals)?;
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
        let now_dt = parse_iso_utc(now)?;
        let mut q = Query::update();
        q.table(Notes::Table).value(Notes::UpdatedAt, now_dt);
        if let Some(ts) = deleted_at {
            q.value(Notes::DeletedAt, Some(parse_iso_utc(ts)?));
        } else {
            q.value(Notes::DeletedAt, None::<chrono::DateTime<Utc>>);
        }
        q.and_where(Expr::col(Notes::Id).eq(parse_uuid(id)?));
        let (sql, vals) = q.take().build_postgres(PostgresQueryBuilder);
        let affected = exec_mutation(&mut self.client.borrow_mut(), sql.as_str(), vals)?;
        if affected == 0 {
            return Err(CliError::NoteNotFound { id: id.to_string() });
        }
        Ok(())
    }

    fn undo_last_delete(&self) -> Result<(), CliError> {
        let now = parse_iso_utc(&chrono::Utc::now().to_rfc3339())?;
        let (sql, vals) = Query::update()
            .table(Notes::Table)
            .values([
                (Notes::DeletedAt, None::<chrono::DateTime<Utc>>.into()),
                (Notes::UpdatedAt, now.into()),
            ])
            .and_where(Expr::cust(
                "id = (SELECT id FROM notes WHERE deleted_at IS NOT NULL ORDER BY deleted_at DESC LIMIT 1)",
            ))
            .take()
            .build_postgres(PostgresQueryBuilder);
        let affected = exec_mutation(&mut self.client.borrow_mut(), sql.as_str(), vals)?;
        if affected == 0 {
            return Err(CliError::Other("no deleted notes to restore".into()));
        }
        Ok(())
    }

    fn find_project_by_name(&self, name: &str) -> Result<Option<String>, CliError> {
        let (sql, vals) = Query::select()
            .column(Projects::Id)
            .from(Projects::Table)
            .and_where(Expr::col(Projects::Name).eq(name))
            .and_where(Expr::col(Projects::IsArchived).is_not(true))
            .limit(1)
            .take()
            .build_postgres(PostgresQueryBuilder);
        exec_opt(
            &mut self.client.borrow_mut(),
            sql.as_str(),
            vals,
            CliError::Other("project not found".into()),
            |r| Ok(r.get::<_, Option<String>>(0)),
        )
    }

    fn find_project_name_by_id(&self, project_id: &str) -> Result<Option<String>, CliError> {
        let (sql, vals) = Query::select()
            .column(Projects::Name)
            .from(Projects::Table)
            .and_where(Expr::col(Projects::Id).eq(parse_uuid(project_id)?))
            .limit(1)
            .take()
            .build_postgres(PostgresQueryBuilder);
        exec_opt(
            &mut self.client.borrow_mut(),
            sql.as_str(),
            vals,
            CliError::Other("project not found".into()),
            |r| Ok(r.get::<_, Option<String>>(0)),
        )
    }

    fn list_projects(&self, archived: bool) -> Result<Vec<Project>, CliError> {
        let (sql, vals) = Query::select()
            .exprs([
                Expr::cust("id::text AS id"),
                Expr::cust("user_id::text AS user_id"),
                Expr::col(Projects::Name),
                Expr::col(Projects::Color),
                Expr::cust("prompt_id::text AS prompt_id"),
                Expr::cust("keyterm_id::text AS keyterm_id"),
                Expr::cust("(CASE WHEN is_archived THEN 1 ELSE 0 END) AS is_archived"),
                Expr::cust("created_at::text AS created_at"),
            ])
            .from(Projects::Table)
            .and_where(if archived {
                Expr::col(Projects::IsArchived).is_not_null()
            } else {
                Expr::col(Projects::IsArchived).is_null()
            })
            .order_by(Projects::Name, Order::Asc)
            .take()
            .build_postgres(PostgresQueryBuilder);
        exec_all(
            &mut self.client.borrow_mut(),
            sql.as_str(),
            vals,
            Project::from_pg_row,
        )
    }

    fn create_project(&self, name: &str) -> Result<String, CliError> {
        let id = Uuid::new_v4();
        let now = parse_iso_utc(&chrono::Utc::now().to_rfc3339())?;
        let (sql, vals) = Query::insert()
            .into_table(Projects::Table)
            .columns([
                Projects::Id,
                Projects::Name,
                Projects::IsArchived,
                Projects::CreatedAt,
            ])
            .values_panic([id.into(), name.into(), false.into(), now.into()])
            .take()
            .build_postgres(PostgresQueryBuilder);
        exec_mutation(&mut self.client.borrow_mut(), sql.as_str(), vals)?;
        Ok(id.to_string())
    }

    fn move_note_to_project(
        &self,
        note_id: &str,
        new_project_id: &str,
        old_project_id: Option<&str>,
    ) -> Result<Option<String>, CliError> {
        let now = parse_iso_utc(&chrono::Utc::now().to_rfc3339())?;
        let mut c = self.client.borrow_mut();
        let mut tx = c
            .transaction()
            .map_err(|e| CliError::Database(e.to_string()))?;

        // UPDATE notes SET project_id = $1, updated_at = $2 WHERE id = $3
        let (sql, vals) = Query::update()
            .table(Notes::Table)
            .values([
                (Notes::ProjectId, parse_uuid(new_project_id)?.into()),
                (Notes::UpdatedAt, now.into()),
            ])
            .and_where(Expr::col(Notes::Id).eq(parse_uuid(note_id)?))
            .take()
            .build_postgres(PostgresQueryBuilder);
        let params = vals.as_params();
        let affected = tx
            .execute(sql.as_str(), &params)
            .map_err(|e| CliError::Database(e.to_string()))?;
        if affected == 0 {
            drop(tx);
            return Err(CliError::NoteNotFound {
                id: note_id.to_string(),
            });
        }

        let Some(old_pid) = old_project_id else {
            tx.commit().map_err(|e| CliError::Database(e.to_string()))?;
            return Ok(None);
        };

        // SELECT COUNT(*) FROM notes WHERE project_id = $1 AND deleted_at IS NULL
        let (sql, vals) = Query::select()
            .expr(Expr::cust("COUNT(*)"))
            .from(Notes::Table)
            .and_where(Expr::col(Notes::ProjectId).eq(parse_uuid(old_pid)?))
            .and_where(Expr::col(Notes::DeletedAt).is_null())
            .take()
            .build_postgres(PostgresQueryBuilder);
        let params = vals.as_params();
        let count: i64 = tx
            .query_one(sql.as_str(), &params)
            .map_err(|e| CliError::Database(e.to_string()))?
            .get(0);

        if count == 0 {
            // SELECT name FROM projects WHERE id = $1
            let (sql, vals) = Query::select()
                .column(Projects::Name)
                .from(Projects::Table)
                .and_where(Expr::col(Projects::Id).eq(parse_uuid(old_pid)?))
                .take()
                .build_postgres(PostgresQueryBuilder);
            let params = vals.as_params();
            let old_name: Option<String> = tx
                .query_one(sql.as_str(), &params)
                .map_err(|e| CliError::Database(e.to_string()))?
                .get(0);

            // DELETE FROM projects WHERE id = $1
            let (sql, vals) = Query::delete()
                .from_table(Projects::Table)
                .and_where(Expr::col(Projects::Id).eq(parse_uuid(old_pid)?))
                .take()
                .build_postgres(PostgresQueryBuilder);
            let params = vals.as_params();
            tx.execute(sql.as_str(), &params)
                .map_err(|e| CliError::Database(e.to_string()))?;

            tx.commit().map_err(|e| CliError::Database(e.to_string()))?;
            Ok(old_name)
        } else {
            tx.commit().map_err(|e| CliError::Database(e.to_string()))?;
            Ok(None)
        }
    }

    fn find_project(&self, id: &str) -> Result<Project, CliError> {
        let (sql, vals) = Query::select()
            .exprs([
                Expr::cust("id::text AS id"),
                Expr::cust("user_id::text AS user_id"),
                Expr::col(Projects::Name),
                Expr::col(Projects::Color),
                Expr::cust("prompt_id::text AS prompt_id"),
                Expr::cust("keyterm_id::text AS keyterm_id"),
                Expr::cust("(CASE WHEN is_archived THEN 1 ELSE 0 END) AS is_archived"),
                Expr::cust("created_at::text AS created_at"),
            ])
            .from(Projects::Table)
            .and_where(Expr::col(Projects::Id).eq(parse_uuid(id)?))
            .limit(1)
            .take()
            .build_postgres(PostgresQueryBuilder);
        exec_opt(
            &mut self.client.borrow_mut(),
            sql.as_str(),
            vals,
            CliError::Other(format!("Project not found: {id}")),
            Project::from_pg_row,
        )
    }

    fn resolve_project_id(&self, prefix: &str) -> Result<String, CliError> {
        crate::backend::validate_id_prefix(prefix)?;
        let pattern = format!("{prefix}%");
        let (sql, vals) = Query::select()
            .column(Projects::Id)
            .from(Projects::Table)
            .and_where(
                Expr::col(Projects::Id)
                    .cast_as(Alias::new("text"))
                    .like(&pattern),
            )
            .limit(2)
            .take()
            .build_postgres(PostgresQueryBuilder);
        let rows = exec_all(&mut self.client.borrow_mut(), sql.as_str(), vals, |r| {
            Ok(r.get::<_, String>(0))
        })?;
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
        let mut q = Query::update();
        q.table(Projects::Table);
        let mut has_values = false;
        if let Some(val) = prompt_id {
            q.value(Projects::PromptId, parse_uuid_opt(val)?);
            has_values = true;
        }
        if let Some(val) = keyterm_id {
            q.value(Projects::KeytermId, parse_uuid_opt(val)?);
            has_values = true;
        }
        if let Some(val) = color {
            q.value(Projects::Color, val);
            has_values = true;
        }
        if !has_values {
            return Ok(());
        }
        q.and_where(Expr::col(Projects::Id).eq(parse_uuid(id)?));
        let (sql, vals) = q.take().build_postgres(PostgresQueryBuilder);
        let affected = exec_mutation(&mut self.client.borrow_mut(), sql.as_str(), vals)?;
        if affected == 0 {
            return Err(CliError::Other(format!("Project not found: {id}")));
        }
        Ok(())
    }

    fn delete_project(&self, id: &str) -> Result<(), CliError> {
        let (sql, vals) = Query::update()
            .table(Projects::Table)
            .value(Projects::IsArchived, true)
            .and_where(Expr::col(Projects::Id).eq(parse_uuid(id)?))
            .take()
            .build_postgres(PostgresQueryBuilder);
        let affected = exec_mutation(&mut self.client.borrow_mut(), sql.as_str(), vals)?;
        if affected == 0 {
            return Err(CliError::Other(format!("Project not found: {id}")));
        }
        Ok(())
    }

    fn update_note_title(&self, id: &str, title: &str) -> Result<(), CliError> {
        let now = parse_iso_utc(&chrono::Utc::now().to_rfc3339())?;
        let (sql, vals) = Query::update()
            .table(Notes::Table)
            .values([(Notes::Title, title.into()), (Notes::UpdatedAt, now.into())])
            .and_where(Expr::col(Notes::Id).eq(parse_uuid(id)?))
            .take()
            .build_postgres(PostgresQueryBuilder);
        let affected = exec_mutation(&mut self.client.borrow_mut(), sql.as_str(), vals)?;
        if affected == 0 {
            return Err(CliError::NoteNotFound { id: id.to_string() });
        }
        Ok(())
    }

    fn update_note_flagged(&self, id: &str, flagged: bool) -> Result<(), CliError> {
        let now = parse_iso_utc(&chrono::Utc::now().to_rfc3339())?;
        let val: i64 = if flagged { 1 } else { 0 };
        let (sql, vals) = Query::update()
            .table(Notes::Table)
            .values([
                (Notes::IsFlagged, val.into()),
                (Notes::UpdatedAt, now.into()),
            ])
            .and_where(Expr::col(Notes::Id).eq(parse_uuid(id)?))
            .take()
            .build_postgres(PostgresQueryBuilder);
        let affected = exec_mutation(&mut self.client.borrow_mut(), sql.as_str(), vals)?;
        if affected == 0 {
            return Err(CliError::NoteNotFound { id: id.to_string() });
        }
        Ok(())
    }

    fn count_notes(&self, filter: &NoteFilter<'_>) -> Result<u64, CliError> {
        let mut q = Query::select();
        q.expr(Expr::cust("COUNT(*)"))
            .from(Notes::Table)
            .and_where(if filter.archived {
                Expr::col(Notes::DeletedAt).is_not_null()
            } else {
                Expr::col(Notes::DeletedAt).is_null()
            });
        if let Some(t) = filter.note_type {
            q.and_where(Expr::col(Notes::Type).eq(t.to_string()));
        }
        if let Some(pid) = filter.project_id {
            q.and_where(Expr::col(Notes::ProjectId).eq(parse_uuid(pid)?));
        }
        let (sql, vals) = q.take().build_postgres(PostgresQueryBuilder);
        let params = vals.as_params();
        let rows = self
            .client
            .borrow_mut()
            .query(sql.as_str(), &params)
            .map_err(|e| CliError::Database(e.to_string()))?;
        let count: i64 = rows.first().map(|r| r.get::<_, i64>(0)).unwrap_or(0);
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
        let uuids: Vec<Uuid> = note_ids
            .iter()
            .map(|s| parse_uuid(s))
            .collect::<Result<Vec<_>, _>>()?;
        let (sql, vals) = Query::select()
            .columns([NoteExtractions::NoteId, NoteExtractions::Value])
            .from(NoteExtractions::Table)
            .and_where(Expr::col(NoteExtractions::Type).eq("topic"))
            .and_where(Expr::col(NoteExtractions::NoteId).is_in(uuids))
            .take()
            .build_postgres(PostgresQueryBuilder);
        let params = vals.as_params();
        let rows: Vec<(String, String)> = self
            .client
            .borrow_mut()
            .query(sql.as_str(), &params)
            .map_err(|e| CliError::Database(e.to_string()))?
            .into_iter()
            .map(|r| {
                Ok::<(String, String), CliError>((r.get::<_, String>(0), r.get::<_, String>(1)))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let mut map: std::collections::HashMap<String, Vec<String>> =
            std::collections::HashMap::new();
        for (note_id, value) in rows {
            map.entry(note_id).or_default().push(value);
        }
        Ok(map)
    }

    fn resolve_prompt_id(&self, prefix: &str) -> Result<String, CliError> {
        crate::backend::validate_id_prefix(prefix)?;
        let pattern = format!("{prefix}%");
        let (sql, vals) = Query::select()
            .column(Prompts::Id)
            .from(Prompts::Table)
            .and_where(
                Expr::col(Prompts::Id)
                    .cast_as(Alias::new("text"))
                    .like(&pattern),
            )
            .limit(2)
            .take()
            .build_postgres(PostgresQueryBuilder);
        let rows = exec_all(&mut self.client.borrow_mut(), sql.as_str(), vals, |r| {
            Ok(r.get::<_, String>(0))
        })?;
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
        let now_dt = parse_iso_utc(now)?;
        let (sql, vals) = Query::insert()
            .into_table(Prompts::Table)
            .columns([
                Prompts::Id,
                Prompts::Title,
                Prompts::Description,
                Prompts::Prompt,
                Prompts::CreatedAt,
            ])
            .values_panic([
                parse_uuid(id)?.into(),
                title.into(),
                description.into(),
                prompt.into(),
                now_dt.into(),
            ])
            .take()
            .build_postgres(PostgresQueryBuilder);
        exec_mutation(&mut self.client.borrow_mut(), sql.as_str(), vals)?;
        Ok(())
    }

    fn find_prompt(&self, id: &str) -> Result<Prompt, CliError> {
        let (sql, vals) = Query::select()
            .columns([
                Prompts::Id,
                Prompts::UserId,
                Prompts::Title,
                Prompts::Description,
                Prompts::Prompt,
                Prompts::CreatedAt,
            ])
            .from(Prompts::Table)
            .and_where(Expr::col(Prompts::Id).eq(parse_uuid(id)?))
            .limit(1)
            .take()
            .build_postgres(PostgresQueryBuilder);
        exec_opt(
            &mut self.client.borrow_mut(),
            sql.as_str(),
            vals,
            CliError::Other(format!("Prompt not found: {id}")),
            Prompt::from_pg_row,
        )
    }

    fn list_prompts(&self) -> Result<Vec<Prompt>, CliError> {
        let (sql, vals) = Query::select()
            .columns([
                Prompts::Id,
                Prompts::UserId,
                Prompts::Title,
                Prompts::Description,
                Prompts::Prompt,
                Prompts::CreatedAt,
            ])
            .from(Prompts::Table)
            .order_by(Prompts::CreatedAt, Order::Desc)
            .take()
            .build_postgres(PostgresQueryBuilder);
        exec_all(
            &mut self.client.borrow_mut(),
            sql.as_str(),
            vals,
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
        let mut q = Query::update();
        q.table(Prompts::Table);
        let mut has_values = false;
        if let Some(v) = title {
            q.value(Prompts::Title, v);
            has_values = true;
        }
        if let Some(v) = description {
            q.value(Prompts::Description, v);
            has_values = true;
        }
        if let Some(v) = prompt {
            q.value(Prompts::Prompt, v);
            has_values = true;
        }
        if !has_values {
            return Ok(());
        }
        q.and_where(Expr::col(Prompts::Id).eq(parse_uuid(id)?));
        let (sql, vals) = q.take().build_postgres(PostgresQueryBuilder);
        let affected = exec_mutation(&mut self.client.borrow_mut(), sql.as_str(), vals)?;
        if affected == 0 {
            return Err(CliError::Other(format!("Prompt not found: {id}")));
        }
        Ok(())
    }

    fn delete_prompt(&self, id: &str) -> Result<(), CliError> {
        let (sql, vals) = Query::delete()
            .from_table(Prompts::Table)
            .and_where(Expr::col(Prompts::Id).eq(parse_uuid(id)?))
            .take()
            .build_postgres(PostgresQueryBuilder);
        let affected = exec_mutation(&mut self.client.borrow_mut(), sql.as_str(), vals)?;
        if affected == 0 {
            return Err(CliError::Other(format!("Prompt not found: {id}")));
        }
        Ok(())
    }

    fn resolve_keyterm_id(&self, prefix: &str) -> Result<String, CliError> {
        crate::backend::validate_id_prefix(prefix)?;
        let pattern = format!("{prefix}%");
        let (sql, vals) = Query::select()
            .column(Keyterms::Id)
            .from(Keyterms::Table)
            .and_where(
                Expr::col(Keyterms::Id)
                    .cast_as(Alias::new("text"))
                    .like(&pattern),
            )
            .limit(2)
            .take()
            .build_postgres(PostgresQueryBuilder);
        let rows = exec_all(&mut self.client.borrow_mut(), sql.as_str(), vals, |r| {
            Ok(r.get::<_, String>(0))
        })?;
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
        let now_dt = parse_iso_utc(now)?;
        let (sql, vals) = Query::insert()
            .into_table(Keyterms::Table)
            .columns([
                Keyterms::Id,
                Keyterms::Name,
                Keyterms::Description,
                Keyterms::Content,
                Keyterms::CreatedAt,
                Keyterms::UpdatedAt,
            ])
            .values_panic([
                parse_uuid(id)?.into(),
                name.into(),
                description.into(),
                content.into(),
                now_dt.into(),
                now_dt.into(),
            ])
            .take()
            .build_postgres(PostgresQueryBuilder);
        exec_mutation(&mut self.client.borrow_mut(), sql.as_str(), vals)?;
        Ok(())
    }

    fn find_keyterm(&self, id: &str) -> Result<Keyterm, CliError> {
        let (sql, vals) = Query::select()
            .columns([
                Keyterms::Id,
                Keyterms::UserId,
                Keyterms::Name,
                Keyterms::Description,
                Keyterms::Content,
                Keyterms::CreatedAt,
                Keyterms::UpdatedAt,
            ])
            .from(Keyterms::Table)
            .and_where(Expr::col(Keyterms::Id).eq(parse_uuid(id)?))
            .limit(1)
            .take()
            .build_postgres(PostgresQueryBuilder);
        exec_opt(
            &mut self.client.borrow_mut(),
            sql.as_str(),
            vals,
            CliError::Other(format!("Keyterm not found: {id}")),
            Keyterm::from_pg_row,
        )
    }

    fn list_keyterms(&self) -> Result<Vec<Keyterm>, CliError> {
        let (sql, vals) = Query::select()
            .columns([
                Keyterms::Id,
                Keyterms::UserId,
                Keyterms::Name,
                Keyterms::Description,
                Keyterms::Content,
                Keyterms::CreatedAt,
                Keyterms::UpdatedAt,
            ])
            .from(Keyterms::Table)
            .order_by(Keyterms::Name, Order::Asc)
            .take()
            .build_postgres(PostgresQueryBuilder);
        exec_all(
            &mut self.client.borrow_mut(),
            sql.as_str(),
            vals,
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
        let mut q = Query::update();
        q.table(Keyterms::Table);
        let mut has_values = false;
        if let Some(v) = name {
            q.value(Keyterms::Name, v);
            has_values = true;
        }
        if let Some(v) = description {
            q.value(Keyterms::Description, v);
            has_values = true;
        }
        if let Some(v) = content {
            q.value(Keyterms::Content, v);
            has_values = true;
        }
        if !has_values {
            return Ok(());
        }
        q.value(Keyterms::UpdatedAt, chrono::Utc::now());
        q.and_where(Expr::col(Keyterms::Id).eq(parse_uuid(id)?));
        let (sql, vals) = q.take().build_postgres(PostgresQueryBuilder);
        exec_mutation(&mut self.client.borrow_mut(), sql.as_str(), vals)?;
        Ok(())
    }

    fn delete_keyterm(&self, id: &str) -> Result<(), CliError> {
        let (sql, vals) = Query::delete()
            .from_table(Keyterms::Table)
            .and_where(Expr::col(Keyterms::Id).eq(parse_uuid(id)?))
            .take()
            .build_postgres(PostgresQueryBuilder);
        let affected = exec_mutation(&mut self.client.borrow_mut(), sql.as_str(), vals)?;
        if affected == 0 {
            return Err(CliError::Other(format!("Keyterm not found: {id}")));
        }
        Ok(())
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

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
}
