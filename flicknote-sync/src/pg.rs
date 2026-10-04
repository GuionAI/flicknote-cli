//! Request-scoped PostgreSQL adapter. Every operation runs on one transaction
//! with the verified identity installed before any application SQL executes.
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use async_trait::async_trait;
use deadpool_postgres::{Object, Pool};
use flicknote_client::dto::RecallCandidate;
use flicknote_core::backend::{
    InsertNoteReq, InsertedNote, NoteDb, NoteFilter, NoteSearch as StructuredSearch,
    RouteProjectUpdate,
};
use flicknote_core::error::CliError;
use flicknote_core::services::error::ServiceError;
use flicknote_core::services::ports::{CreateNote, CreatedNote, NoteCreator};
use flicknote_core::types::{Note, Project};
use tokio::sync::Mutex;
use tokio_postgres::{Row, types::ToSql};
use uuid::Uuid;

const NOTE_COLUMNS: &str = "id, short_id, user_id, type, status, title, content, summary, is_flagged, project_id, metadata, source, created_at, updated_at, deleted_at";
const PROJECT_COLUMNS: &str = "id, user_id, name, color, metadata, is_archived, created_at";
const SAFE_ROLE_SQL: &str = "SELECT NOT (r.rolsuper OR r.rolbypassrls) AND NOT EXISTS (SELECT 1 FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname='public' AND c.relname IN ('notes','projects','note_extractions') AND pg_has_role(r.oid,c.relowner,'member')) FROM pg_roles r WHERE r.rolname=current_user";

fn pg_error(error: impl std::fmt::Display) -> CliError {
    CliError::Other(format!("PostgreSQL error: {error}"))
}
fn uuid(input: &str) -> Result<Uuid, CliError> {
    Uuid::parse_str(input).map_err(|_| CliError::NoteNotFound { id: input.into() })
}
fn note_missing(id: &str) -> CliError {
    CliError::NoteNotFound { id: id.into() }
}
fn project_missing(id: &str) -> CliError {
    CliError::ProjectNotFound { name: id.into() }
}
fn note(row: &Row) -> Note {
    let id: Uuid = row.get("id");
    let user: Uuid = row.get("user_id");
    let project: Option<Uuid> = row.get("project_id");
    let metadata: Option<serde_json::Value> = row.get("metadata");
    let source: Option<serde_json::Value> = row.get("source");
    let created: chrono::DateTime<chrono::Utc> = row.get("created_at");
    let updated: chrono::DateTime<chrono::Utc> = row.get("updated_at");
    let deleted: Option<chrono::DateTime<chrono::Utc>> = row.get("deleted_at");
    Note {
        id: id.to_string(),
        short_id: row.get::<_, Option<i32>>("short_id").map(i64::from),
        user_id: user.to_string(),
        r#type: row.get("type"),
        status: row.get("status"),
        title: row.get("title"),
        content: row.get("content"),
        summary: row.get("summary"),
        is_flagged: Some(i64::from(row.get::<_, bool>("is_flagged"))),
        project_id: project.map(|id| id.to_string()),
        metadata: metadata.map(|v| v.to_string()),
        source: source.map(|v| v.to_string()),
        created_at: Some(created.to_rfc3339()),
        updated_at: Some(updated.to_rfc3339()),
        deleted_at: deleted.map(|v| v.to_rfc3339()),
    }
}
fn project(row: &Row) -> Project {
    let metadata: serde_json::Value = row.get("metadata");
    let created: chrono::DateTime<chrono::Utc> = row.get("created_at");
    Project {
        id: row.get::<_, Uuid>("id").to_string(),
        user_id: row.get::<_, Uuid>("user_id").to_string(),
        name: row.get("name"),
        color: row.get("color"),
        metadata: Some(metadata.to_string()),
        is_archived: Some(i64::from(row.get::<_, bool>("is_archived"))),
        created_at: Some(created.to_rfc3339()),
    }
}

/// A checked-out connection is never returned to the pool while a transaction
/// might still be open. Cancellation drops the object and closes its socket.
pub struct PgRequestDb {
    user_id: String,
    connection: Mutex<Option<Object>>,
    failed: AtomicBool,
}

/// Discard a checkout unless setup or finalization explicitly releases it.
/// This owns the connection across awaits before/after the request owns it.
struct DisconnectOnDrop(Option<Object>);

impl DisconnectOnDrop {
    fn release(mut self) -> Object {
        self.0.take().expect("guard owns its checkout")
    }
}

impl std::ops::Deref for DisconnectOnDrop {
    type Target = Object;

    fn deref(&self) -> &Object {
        self.0.as_ref().expect("guard owns its checkout")
    }
}

impl Drop for DisconnectOnDrop {
    fn drop(&mut self) {
        if let Some(connection) = self.0.take() {
            drop(Object::take(connection));
        }
    }
}

#[cfg(test)]
struct SetupPause {
    ready: tokio::sync::oneshot::Sender<(i32, String)>,
    resume: tokio::sync::oneshot::Receiver<()>,
}

impl PgRequestDb {
    pub async fn begin(pool: &Pool, user_id: Uuid) -> Result<Arc<Self>, CliError> {
        Self::begin_inner(
            pool,
            user_id,
            #[cfg(test)]
            None,
        )
        .await
    }

    async fn begin_inner(
        pool: &Pool,
        user_id: Uuid,
        #[cfg(test)] pause: Option<SetupPause>,
    ) -> Result<Arc<Self>, CliError> {
        let connection = DisconnectOnDrop(Some(pool.get().await.map_err(pg_error)?));
        if let Err(error) = connection.batch_execute("BEGIN").await {
            return Err(pg_error(error));
        }
        let safe_login = connection.query_one(SAFE_ROLE_SQL, &[]).await;
        if !matches!(safe_login, Ok(ref row) if row.get::<_, bool>(0)) {
            return Err(pg_error(
                "database login role must not own tables, be superuser, or bypass RLS",
            ));
        }
        if let Err(error) = connection
            .batch_execute("SET LOCAL ROLE authenticated")
            .await
        {
            return Err(pg_error(error));
        }
        if let Err(error) = connection.query_one("SELECT set_config('request.jwt.claim.sub', $1, true), set_config('request.jwt.claim.role', 'authenticated', true)", &[&user_id.to_string()]).await {
            return Err(pg_error(error));
        }
        #[cfg(test)]
        if let Some(pause) = pause {
            let row = connection
                .query_one("SELECT pg_backend_pid(), auth.uid()::text", &[])
                .await
                .map_err(pg_error)?;
            pause
                .ready
                .send((row.get(0), row.get(1)))
                .expect("setup observer is present");
            pause.resume.await.expect("setup resumer is present");
        }
        let safe_role = connection.query_one(SAFE_ROLE_SQL, &[]).await;
        if !matches!(safe_role, Ok(ref row) if row.get::<_, bool>(0)) {
            return Err(pg_error(
                "database role must not own tables, be superuser, or bypass RLS",
            ));
        }
        Ok(Arc::new(Self {
            user_id: user_id.to_string(),
            connection: Mutex::new(Some(connection.release())),
            failed: AtomicBool::new(false),
        }))
    }
    pub fn mark_failed(&self) {
        self.failed.store(true, Ordering::Relaxed);
    }
    pub fn failed(&self) -> bool {
        self.failed.load(Ordering::Relaxed)
    }
    pub async fn finish(&self, success: bool) -> Result<(), CliError> {
        let mut guard = self.connection.lock().await;
        if let Some(connection) = guard.take() {
            let connection = DisconnectOnDrop(Some(connection));
            connection
                .batch_execute(if success { "COMMIT" } else { "ROLLBACK" })
                .await
                .map_err(pg_error)?;
            drop(connection.release());
        }
        Ok(())
    }
    async fn query(&self, sql: &str, params: &[&(dyn ToSql + Sync)]) -> Result<Vec<Row>, CliError> {
        let guard = self.connection.lock().await;
        let connection = guard
            .as_ref()
            .ok_or_else(|| pg_error("transaction already closed"))?;
        connection.query(sql, params).await.map_err(pg_error)
    }
    async fn query_opt(
        &self,
        sql: &str,
        params: &[&(dyn ToSql + Sync)],
    ) -> Result<Option<Row>, CliError> {
        let guard = self.connection.lock().await;
        let connection = guard
            .as_ref()
            .ok_or_else(|| pg_error("transaction already closed"))?;
        connection.query_opt(sql, params).await.map_err(pg_error)
    }
    async fn execute(&self, sql: &str, params: &[&(dyn ToSql + Sync)]) -> Result<u64, CliError> {
        let guard = self.connection.lock().await;
        let connection = guard
            .as_ref()
            .ok_or_else(|| pg_error("transaction already closed"))?;
        connection.execute(sql, params).await.map_err(pg_error)
    }
    async fn owned_project(&self, project_id: &str) -> Result<Uuid, CliError> {
        let id = Uuid::parse_str(project_id).map_err(|_| project_missing(project_id))?;
        if self.query_opt("SELECT id FROM projects WHERE id=$1 AND user_id=auth.uid() AND NOT is_archived FOR UPDATE", &[&id]).await?.is_none() {
            return Err(project_missing(project_id));
        }
        Ok(id)
    }
}
impl Drop for PgRequestDb {
    fn drop(&mut self) {
        if let Some(connection) = self.connection.get_mut().take() {
            drop(Object::take(connection));
        }
    }
}

pub struct PgNoteCreator(pub Arc<PgRequestDb>);
#[async_trait]
impl NoteCreator for PgNoteCreator {
    async fn create(&self, request: CreateNote) -> Result<CreatedNote, ServiceError> {
        if request.attachment_path.is_some() {
            return Err(ServiceError::InvalidArgument(
                "remote attachment creation is unavailable".into(),
            ));
        }
        let inserted = self.0.insert_note(&request.as_insert_request()).await?;
        if !request.topics.is_empty() {
            self.0
                .set_note_extractions(
                    &inserted.uuid,
                    flicknote_core::TOPIC_EXTRACTION_KEY,
                    &request.topics,
                )
                .await?;
        }
        Ok(CreatedNote {
            inserted,
            confirmed_extraction_ids: Vec::new(),
        })
    }
}

#[async_trait]
impl NoteDb for PgRequestDb {
    fn user_id(&self) -> &str {
        &self.user_id
    }
    async fn resolve_note_id(&self, input: &str) -> Result<String, CliError> {
        self.resolve_note(input, false).await
    }
    async fn resolve_archived_note_id(&self, input: &str) -> Result<String, CliError> {
        self.resolve_note(input, true).await
    }
    async fn find_note(&self, id: &str) -> Result<Note, CliError> {
        self.get_note(id, false).await
    }
    async fn find_archived_note(&self, id: &str) -> Result<Note, CliError> {
        self.get_note(id, true).await
    }
    async fn find_note_content(&self, id: &str) -> Result<Option<String>, CliError> {
        let id = uuid(id)?;
        self.query_opt("SELECT content FROM notes WHERE id=$1 AND user_id=auth.uid() AND deleted_at IS NULL FOR UPDATE", &[&id]).await?.map(|row| row.get(0)).ok_or_else(|| note_missing(&id.to_string()))
    }
    async fn list_notes(&self, f: &NoteFilter<'_>) -> Result<Vec<Note>, CliError> {
        let project = f
            .project_id
            .map(Uuid::parse_str)
            .transpose()
            .map_err(pg_error)?;
        let sql = format!(
            "SELECT {NOTE_COLUMNS} FROM notes WHERE user_id=auth.uid() AND (deleted_at IS NOT NULL)=$1 AND ($2::text IS NULL OR type=$2) AND ($3::text IS NULL OR status=$3) AND ($4::uuid IS NULL OR project_id=$4) AND (NOT $5 OR project_id IS NULL) AND (NOT $6 OR metadata->'created_by_ai' IS DISTINCT FROM 'true'::jsonb) AND ($7::bigint IS NULL OR created_at >= TIMESTAMPTZ 'epoch' + $7 * INTERVAL '1 microsecond') AND ($8::bigint IS NULL OR created_at < TIMESTAMPTZ 'epoch' + $8 * INTERVAL '1 microsecond') AND ($9::bigint IS NULL OR short_id < $9) ORDER BY short_id DESC LIMIT $10"
        );
        let limit = i64::from(f.limit);
        Ok(self
            .query(
                &sql,
                &[
                    &f.archived,
                    &f.note_type,
                    &f.status,
                    &project,
                    &f.no_project,
                    &f.human,
                    &f.created_after_micros,
                    &f.created_before_micros,
                    &f.cursor,
                    &limit,
                ],
            )
            .await?
            .iter()
            .map(note)
            .collect())
    }
    async fn search_notes_structured(
        &self,
        search: &StructuredSearch,
        f: &NoteFilter<'_>,
    ) -> Result<Vec<Note>, CliError> {
        if search.extractions.is_empty() {
            return Err(CliError::Other(
                "search_notes_structured requires an extraction filter".into(),
            ));
        }
        let project = f
            .project_id
            .map(Uuid::parse_str)
            .transpose()
            .map_err(pg_error)?;
        let filters = serde_json::to_value(
            search
                .extractions
                .iter()
                .map(|filter| serde_json::json!({"key": filter.key, "value": filter.value}))
                .collect::<Vec<_>>(),
        )?;
        let limit = i64::from(f.limit);
        let sql = format!(
            "SELECT {NOTE_COLUMNS} FROM notes n WHERE n.user_id=auth.uid() AND (n.deleted_at IS NOT NULL)=$1 AND ($1 OR n.status<>'draft') AND ($2::text IS NULL OR n.type=$2) AND ($3::uuid IS NULL OR n.project_id=$3) AND NOT EXISTS (SELECT 1 FROM jsonb_to_recordset($4::jsonb) AS f(key text, value text) WHERE NOT EXISTS (SELECT 1 FROM note_extractions e WHERE e.note_id=n.id AND e.user_id=auth.uid() AND e.key=f.key AND e.value=f.value)) AND ($6::bigint IS NULL OR n.created_at >= TIMESTAMPTZ 'epoch' + $6 * INTERVAL '1 microsecond') AND ($7::bigint IS NULL OR n.created_at < TIMESTAMPTZ 'epoch' + $7 * INTERVAL '1 microsecond') AND (NOT $8 OR n.metadata->'created_by_ai' IS DISTINCT FROM 'true'::jsonb) ORDER BY n.updated_at DESC LIMIT $5"
        );
        Ok(self
            .query(
                &sql,
                &[
                    &f.archived,
                    &f.note_type,
                    &project,
                    &filters,
                    &limit,
                    &f.created_after_micros,
                    &f.created_before_micros,
                    &f.human,
                ],
            )
            .await?
            .iter()
            .map(note)
            .collect())
    }
    async fn recall_notes(
        &self,
        _prompt: &str,
        _filter: &NoteFilter<'_>,
    ) -> Result<Vec<RecallCandidate>, CliError> {
        Err(CliError::Other("recall is local-only".into()))
    }
    async fn insert_note(&self, req: &InsertNoteReq<'_>) -> Result<InsertedNote, CliError> {
        let id = uuid(req.id)?;
        let project = match req.project_id {
            Some(id) => Some(self.owned_project(id).await?),
            None => None,
        };
        let metadata = req
            .metadata
            .map(serde_json::from_str::<serde_json::Value>)
            .transpose()?;
        let now = chrono::DateTime::parse_from_rfc3339(req.now).map_err(pg_error)?;
        let row = self.query_opt("INSERT INTO notes(id,user_id,type,status,title,content,metadata,project_id,created_at,updated_at) VALUES($1,auth.uid(),$2,$3,$4,$5,$6,$7,$8,$8) RETURNING short_id", &[&id,&req.note_type,&req.status,&req.title,&req.content,&metadata,&project,&now]).await?.ok_or_else(|| pg_error("insert returned no row"))?;
        Ok(InsertedNote {
            uuid: id.to_string(),
            short_id: row.get::<_, Option<i32>>(0).map(i64::from),
        })
    }
    async fn update_note_content(&self, id: &str, content: &str) -> Result<(), CliError> {
        self.update_note_field(id, "content=$2", &[&content]).await
    }
    async fn route_notes_to_projects(
        &self,
        _updates: &[RouteProjectUpdate],
    ) -> Result<(), CliError> {
        Err(CliError::Other(
            "automatic project routing is local-only".into(),
        ))
    }
    async fn submit_draft(&self, id: &str) -> Result<bool, CliError> {
        let id = uuid(id)?;
        Ok(self.execute("UPDATE notes SET status='ai_queued', updated_at=now() WHERE id=$1 AND user_id=auth.uid() AND deleted_at IS NULL AND status='draft'", &[&id]).await? > 0)
    }
    async fn set_note_deleted_at(
        &self,
        id: &str,
        deleted_at: Option<&str>,
        now: &str,
    ) -> Result<(), CliError> {
        let id = uuid(id)?;
        let deleted = deleted_at
            .map(chrono::DateTime::parse_from_rfc3339)
            .transpose()
            .map_err(pg_error)?;
        let updated = chrono::DateTime::parse_from_rfc3339(now).map_err(pg_error)?;
        if self
            .execute(
                "UPDATE notes SET deleted_at=$2, updated_at=$3 WHERE id=$1 AND user_id=auth.uid()",
                &[&id, &deleted, &updated],
            )
            .await?
            == 0
        {
            return Err(note_missing(&id.to_string()));
        }
        Ok(())
    }
    async fn undo_last_delete(&self) -> Result<(), CliError> {
        self.execute("UPDATE notes SET deleted_at=NULL,updated_at=now() WHERE id=(SELECT id FROM notes WHERE user_id=auth.uid() AND deleted_at IS NOT NULL ORDER BY deleted_at DESC LIMIT 1 FOR UPDATE)", &[]).await?;
        Ok(())
    }
    async fn find_project_by_name(&self, name: &str) -> Result<Option<String>, CliError> {
        Ok(self
            .query_opt(
                "SELECT id FROM projects WHERE user_id=auth.uid() AND name=$1 AND NOT is_archived",
                &[&name],
            )
            .await?
            .map(|row| row.get::<_, Uuid>(0).to_string()))
    }
    async fn find_project_name_by_id(&self, id: &str) -> Result<Option<String>, CliError> {
        let id = Uuid::parse_str(id).map_err(|_| project_missing(id))?;
        Ok(self
            .query_opt(
                "SELECT name FROM projects WHERE id=$1 AND user_id=auth.uid()",
                &[&id],
            )
            .await?
            .map(|row| row.get(0)))
    }
    async fn list_projects(&self, archived: bool) -> Result<Vec<Project>, CliError> {
        let sql = format!(
            "SELECT {PROJECT_COLUMNS} FROM projects WHERE user_id=auth.uid() AND is_archived=$1 ORDER BY name"
        );
        Ok(self
            .query(&sql, &[&archived])
            .await?
            .iter()
            .map(project)
            .collect())
    }
    async fn find_project(&self, id: &str) -> Result<Project, CliError> {
        let id = Uuid::parse_str(id).map_err(|_| project_missing(id))?;
        let sql = format!(
            "SELECT {PROJECT_COLUMNS} FROM projects WHERE id=$1 AND user_id=auth.uid() FOR UPDATE"
        );
        self.query_opt(&sql, &[&id])
            .await?
            .as_ref()
            .map(project)
            .ok_or_else(|| project_missing(&id.to_string()))
    }
    async fn resolve_project_id(&self, id: &str) -> Result<String, CliError> {
        let parsed = Uuid::parse_str(id).map_err(|_| project_missing(id))?;
        if self
            .query_opt(
                "SELECT id FROM projects WHERE id=$1 AND user_id=auth.uid()",
                &[&parsed],
            )
            .await?
            .is_none()
        {
            return Err(project_missing(id));
        }
        Ok(parsed.to_string())
    }
    async fn create_project(&self, name: &str) -> Result<String, CliError> {
        let row = self
            .query_opt(
                "INSERT INTO projects(user_id,name) VALUES(auth.uid(),$1) RETURNING id",
                &[&name],
            )
            .await?
            .ok_or_else(|| pg_error("insert returned no project"))?;
        Ok(row.get::<_, Uuid>(0).to_string())
    }
    async fn update_note_project(
        &self,
        id: &str,
        project_id: Option<&str>,
    ) -> Result<(), CliError> {
        let project = match project_id {
            Some(id) => Some(self.owned_project(id).await?),
            None => None,
        };
        let id = uuid(id)?;
        if self.execute("UPDATE notes SET project_id=$2,updated_at=now(),metadata=CASE WHEN metadata ? 'project_routing' THEN metadata - 'project_routing' ELSE metadata END WHERE id=$1 AND user_id=auth.uid() AND deleted_at IS NULL", &[&id,&project]).await? == 0 { return Err(note_missing(&id.to_string())); }
        Ok(())
    }
    async fn update_project(
        &self,
        id: &str,
        color: Option<Option<&str>>,
        summary: Option<Option<&str>>,
    ) -> Result<(), CliError> {
        let id = Uuid::parse_str(id).map_err(|_| project_missing(id))?;
        let row = self
            .query_opt(
                "SELECT metadata FROM projects WHERE id=$1 AND user_id=auth.uid() FOR UPDATE",
                &[&id],
            )
            .await?
            .ok_or_else(|| project_missing(&id.to_string()))?;
        let mut metadata: serde_json::Value = row.get(0);
        let object = metadata
            .as_object_mut()
            .ok_or_else(|| pg_error("project metadata must be object"))?;
        object.remove("pinned");
        if let Some(summary) = summary {
            match summary {
                Some(value) => {
                    object.insert("summary".into(), value.into());
                }
                None => {
                    object.remove("summary");
                }
            }
        }
        let update_color = color.is_some();
        let color = color.flatten();
        self.execute("UPDATE projects SET color=CASE WHEN $2 THEN $3 ELSE color END,metadata=$4 WHERE id=$1 AND user_id=auth.uid()", &[&id,&update_color,&color,&metadata]).await?;
        Ok(())
    }
    async fn delete_project(&self, id: &str) -> Result<(), CliError> {
        let id = Uuid::parse_str(id).map_err(|_| project_missing(id))?;
        if self
            .execute(
                "UPDATE projects SET is_archived=true WHERE id=$1 AND user_id=auth.uid()",
                &[&id],
            )
            .await?
            == 0
        {
            return Err(project_missing(&id.to_string()));
        }
        Ok(())
    }
    async fn update_note_title(&self, id: &str, title: Option<&str>) -> Result<(), CliError> {
        self.update_note_field(id, "title=$2", &[&title]).await
    }
    async fn update_note_summary(&self, id: &str, summary: Option<&str>) -> Result<(), CliError> {
        self.update_note_field(id, "summary=$2", &[&summary]).await
    }
    async fn update_note_flagged(&self, id: &str, flagged: Option<bool>) -> Result<(), CliError> {
        self.update_note_field(id, "is_flagged=$2", &[&flagged.unwrap_or(false)])
            .await
    }
    async fn count_notes(&self, f: &NoteFilter<'_>) -> Result<u64, CliError> {
        let project = f
            .project_id
            .map(Uuid::parse_str)
            .transpose()
            .map_err(pg_error)?;
        let row = self.query_opt("SELECT count(*) FROM notes WHERE user_id=auth.uid() AND (deleted_at IS NOT NULL)=$1 AND ($2::text IS NULL OR type=$2) AND ($3::uuid IS NULL OR project_id=$3) AND (NOT $4 OR metadata->'created_by_ai' IS DISTINCT FROM 'true'::jsonb)", &[&f.archived,&f.note_type,&project,&f.human]).await?.ok_or_else(|| pg_error("count returned no row"))?;
        Ok(row.get::<_, i64>(0).try_into().map_err(pg_error)?)
    }
    async fn list_note_topics(
        &self,
        note_ids: &[&str],
    ) -> Result<HashMap<String, Vec<String>>, CliError> {
        let rows = self
            .list_note_extractions(note_ids, &[flicknote_core::TOPIC_EXTRACTION_KEY])
            .await?;
        Ok(rows
            .into_iter()
            .map(|(id, values)| (id, values.into_iter().map(|(_, value)| value).collect()))
            .collect())
    }
    async fn list_note_extractions(
        &self,
        note_ids: &[&str],
        keys: &[&str],
    ) -> Result<HashMap<String, Vec<(String, String)>>, CliError> {
        if note_ids.is_empty() || keys.is_empty() {
            return Ok(HashMap::new());
        }
        let ids = note_ids
            .iter()
            .map(|id| Uuid::parse_str(id))
            .collect::<Result<Vec<_>, _>>()
            .map_err(pg_error)?;
        let keys = keys.to_vec();
        let mut output = HashMap::new();
        for row in self.query("SELECT e.note_id,e.key,e.value FROM note_extractions e JOIN notes n ON n.id=e.note_id AND n.user_id=e.user_id WHERE e.user_id=auth.uid() AND e.note_id=ANY($1) AND e.key=ANY($2) ORDER BY e.key,e.value", &[&ids,&keys]).await? {
            let id = row.get::<_,Uuid>(0).to_string();
            let key: String = row.get(1);
            output.entry(id).or_insert_with(Vec::new).push((key,row.get(2)));
        }
        Ok(output)
    }
    async fn list_extraction_values(
        &self,
        keys: &[&str],
        archived: bool,
    ) -> Result<Vec<String>, CliError> {
        if keys.is_empty() {
            return Ok(Vec::new());
        }
        let keys = keys.to_vec();
        Ok(self.query("SELECT DISTINCT e.value FROM note_extractions e JOIN notes n ON n.id=e.note_id AND n.user_id=e.user_id WHERE e.user_id=auth.uid() AND e.key=ANY($1) AND (n.deleted_at IS NOT NULL)=$2 ORDER BY e.value", &[&keys,&archived]).await?.iter().map(|row| row.get(0)).collect())
    }
    async fn set_note_extractions(
        &self,
        note_id: &str,
        key: &str,
        values: &[String],
    ) -> Result<(), CliError> {
        let id = uuid(note_id)?;
        if self
            .query_opt(
                "SELECT id FROM notes WHERE id=$1 AND user_id=auth.uid() FOR UPDATE",
                &[&id],
            )
            .await?
            .is_none()
        {
            return Err(note_missing(note_id));
        }
        self.execute(
            "DELETE FROM note_extractions WHERE note_id=$1 AND user_id=auth.uid() AND key=$2",
            &[&id, &key],
        )
        .await?;
        for value in values {
            self.execute("INSERT INTO note_extractions(note_id,user_id,key,value) VALUES($1,auth.uid(),$2,$3) ON CONFLICT (note_id,key,value) DO NOTHING", &[&id,&key,&value]).await?;
        }
        Ok(())
    }
}
impl PgRequestDb {
    async fn resolve_note(&self, input: &str, archived: bool) -> Result<String, CliError> {
        let row = if input.chars().all(|c| c.is_ascii_digit()) {
            let short = input.parse::<i32>().map_err(|_| note_missing(input))?;
            self.query_opt("SELECT id FROM notes WHERE short_id=$1 AND user_id=auth.uid() AND (deleted_at IS NOT NULL)=$2", &[&short,&archived]).await?
        } else {
            let id = uuid(input)?;
            self.query_opt("SELECT id FROM notes WHERE id=$1 AND user_id=auth.uid() AND (deleted_at IS NOT NULL)=$2", &[&id,&archived]).await?
        };
        row.map(|row| row.get::<_, Uuid>(0).to_string())
            .ok_or_else(|| note_missing(input))
    }
    async fn get_note(&self, input: &str, archived: bool) -> Result<Note, CliError> {
        let id = uuid(input)?;
        let sql = format!(
            "SELECT {NOTE_COLUMNS} FROM notes WHERE id=$1 AND user_id=auth.uid() AND (deleted_at IS NOT NULL)=$2 FOR UPDATE"
        );
        self.query_opt(&sql, &[&id, &archived])
            .await?
            .as_ref()
            .map(note)
            .ok_or_else(|| note_missing(input))
    }
    async fn update_note_field(
        &self,
        input: &str,
        set: &str,
        value: &[&(dyn ToSql + Sync)],
    ) -> Result<(), CliError> {
        let id = uuid(input)?;
        let sql = format!(
            "UPDATE notes SET {set},updated_at=now() WHERE id=$1 AND user_id=auth.uid() AND deleted_at IS NULL"
        );
        let mut params: Vec<&(dyn ToSql + Sync)> = vec![&id];
        params.extend_from_slice(value);
        if self.execute(&sql, &params).await? == 0 {
            return Err(note_missing(input));
        }
        Ok(())
    }
}

pub struct PgSearch(pub Arc<PgRequestDb>);
#[async_trait]
impl crate::search::NoteSearch for PgSearch {
    async fn find(
        &self,
        input: &flicknote_client::dto::NoteFindInput,
    ) -> Result<Vec<flicknote_client::dto::SearchHit>, String> {
        use flicknote_client::dto::{SearchHit, SearchSnippet};
        if input.archived || !input.extractions.is_empty() {
            return Err("PGroonga search supports active keyword searches only".into());
        }
        let terms = input
            .keywords
            .iter()
            .map(|t| t.trim().to_owned())
            .filter(|t| !t.is_empty())
            .collect::<Vec<_>>();
        let (after, before) = flicknote_core::services::note::validate_created_range(
            input.created_after.as_deref(),
            input.created_before.as_deref(),
        )
        .map_err(|error| error.to_string())?;
        if terms.is_empty() || input.limit == 0 {
            return Ok(Vec::new());
        }
        if terms.len() == 1
            && terms[0].len() == 1
            && terms[0].chars().all(|c| c.is_ascii_alphabetic())
        {
            return Ok(Vec::new());
        }
        let project = match &input.project {
            Some(name) => match self
                .0
                .find_project_by_name(name)
                .await
                .map_err(|e| e.to_string())?
            {
                Some(id) => Some(Uuid::parse_str(&id).map_err(|e| e.to_string())?),
                None => return Ok(Vec::new()),
            },
            None => None,
        };
        let mut clauses = Vec::new();
        let mut params: Vec<&(dyn ToSql + Sync)> = Vec::new();
        for (index, term) in terms.iter().enumerate() {
            let arg = index + 1;
            clauses.push(format!(
                "(n.title &@ ${arg} OR n.summary &@ ${arg} OR n.content &@ ${arg})"
            ));
            params.push(term);
        }
        let project_arg = terms.len() + 1;
        let limit_arg = terms.len() + 2;
        let terms_arg = terms.len() + 3;
        let after_arg = terms.len() + 4;
        let before_arg = terms.len() + 5;
        let human_arg = terms.len() + 6;
        let limit = i64::from(input.limit);
        params.push(&project);
        params.push(&limit);
        params.push(&terms);
        params.push(&after);
        params.push(&before);
        params.push(&input.human);
        let sql = format!(
            "SELECT n.id,n.short_id,n.type,n.title,n.summary,n.content,n.created_at,n.updated_at,n.project_id, (SELECT coalesce(sum(CASE WHEN strpos(lower(coalesce(n.title,'')),lower(t.term))>0 THEN 3 WHEN strpos(lower(coalesce(n.summary,'')),lower(t.term))>0 THEN 2 WHEN strpos(lower(coalesce(n.content,'')),lower(t.term))>0 THEN 1 ELSE 0 END),0) FROM unnest(${terms_arg}::text[]) AS t(term)) AS coverage FROM notes n WHERE n.user_id=auth.uid() AND n.deleted_at IS NULL AND n.status <> 'draft' AND (${project_arg}::uuid IS NULL OR n.project_id=${project_arg}) AND (${after_arg}::bigint IS NULL OR n.created_at >= TIMESTAMPTZ 'epoch' + ${after_arg} * INTERVAL '1 microsecond') AND (${before_arg}::bigint IS NULL OR n.created_at < TIMESTAMPTZ 'epoch' + ${before_arg} * INTERVAL '1 microsecond') AND (NOT ${human_arg} OR n.metadata->'created_by_ai' IS DISTINCT FROM 'true'::jsonb) AND ({}) ORDER BY coverage DESC,n.updated_at DESC,n.short_id DESC LIMIT ${limit_arg}",
            clauses.join(" OR ")
        );
        let rows = self
            .0
            .query(&sql, &params)
            .await
            .map_err(|e| e.to_string())?;
        let hits = rows
            .iter()
            .map(|row| {
                let content = row.get::<_, Option<String>>(5).unwrap_or_default();
                let title: Option<String> = row.get(3);
                let summary: Option<String> = row.get(4);
                let excerpt = if !content.is_empty() {
                    &content
                } else {
                    title.as_deref().or(summary.as_deref()).unwrap_or("")
                };
                let snippet = snippet(excerpt, &terms);
                SearchHit {
                    short_id: row.get::<_, Option<i32>>(1).map(i64::from),
                    note_type: row.get(2),
                    content_bytes: content.len() as u64,
                    draft: false,
                    title,
                    summary,
                    created_at: Some(row.get::<_, chrono::DateTime<chrono::Utc>>(6).to_rfc3339()),
                    updated_at: Some(row.get::<_, chrono::DateTime<chrono::Utc>>(7).to_rfc3339()),
                    project_id: row.get::<_, Option<Uuid>>(8).map(|id| id.to_string()),
                    snippet: SearchSnippet { segments: snippet },
                }
            })
            .collect();
        Ok(hits)
    }
}
fn snippet(content: &str, terms: &[String]) -> Vec<flicknote_client::dto::SnippetSegment> {
    use flicknote_client::dto::SnippetSegment;
    if content.is_empty() {
        return Vec::new();
    }
    let Some((start, _)) = find_highlight(content, terms) else {
        return vec![SnippetSegment {
            text: content.chars().take(180).collect(),
            highlighted: false,
        }];
    };
    let begin = content[..start]
        .char_indices()
        .rev()
        .nth(32)
        .map_or(0, |(i, _)| i);
    let end = content[start..]
        .char_indices()
        .nth(100)
        .map_or(content.len(), |(i, _)| start + i);
    let excerpt = &content[begin..end];
    let mut output = Vec::new();
    let mut remaining = excerpt;
    while !remaining.is_empty() {
        let Some((index, end)) = find_highlight(remaining, terms) else {
            output.push(SnippetSegment {
                text: remaining.into(),
                highlighted: false,
            });
            break;
        };
        if index > 0 {
            output.push(SnippetSegment {
                text: remaining[..index].into(),
                highlighted: false,
            });
        }
        output.push(SnippetSegment {
            text: remaining[index..end].into(),
            highlighted: true,
        });
        remaining = &remaining[end..];
    }
    output
}

fn find_highlight(content: &str, terms: &[String]) -> Option<(usize, usize)> {
    for (start, _) in content.char_indices() {
        for term in terms {
            if term.is_empty() {
                continue;
            }
            let tail = &content[start..];
            let end = tail
                .char_indices()
                .nth(term.chars().count())
                .map_or(tail.len(), |(i, _)| i);
            if tail[..end].to_lowercase() == term.to_lowercase() {
                return Some((start, start + end));
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[ignore = "requires the test-owned PGroonga container started by scripts/test-private-pg.sh"]
    async fn cancelled_identity_setup_discards_backend_before_pool_reuse() {
        use deadpool_postgres::{Manager, ManagerConfig, RecyclingMethod};
        use flicknote_client::dto::NoteAddInput;
        use flicknote_core::services::note::NoteService;
        use tokio::sync::oneshot;
        use tokio_postgres::NoTls;

        let port = std::env::var("FLICKNOTE_TEST_PG_PORT").expect("run through test-private-pg.sh");
        let config =
            format!("postgresql://flicknote_mcp@127.0.0.1:{port}/supabase?sslmode=disable")
                .parse()
                .unwrap();
        let pool = Pool::builder(Manager::from_config(
            config,
            NoTls,
            ManagerConfig {
                recycling_method: RecyclingMethod::Fast,
            },
        ))
        .max_size(1)
        .build()
        .unwrap();
        let alice = Uuid::parse_str("11111111-1111-4111-8111-111111111111").unwrap();
        let bob = Uuid::parse_str("22222222-2222-4222-8222-222222222222").unwrap();
        let a = PgRequestDb::begin(&pool, alice).await.unwrap();
        let created = NoteService::new(a.as_ref())
            .add(
                &PgNoteCreator(a.clone()),
                NoteAddInput {
                    content: "Setup cancellation isolation".into(),
                    project: None,
                    interpret_as_url: true,
                    draft: true,
                    topics: Vec::new(),
                    created_by_ai: false,
                    created_at: None,
                },
            )
            .await
            .unwrap();
        a.finish(true).await.unwrap();
        let (ready, installed) = oneshot::channel();
        let (_resume, resume) = oneshot::channel();
        let setup_pool = pool.clone();
        let setup = tokio::spawn(async move {
            PgRequestDb::begin_inner(&setup_pool, alice, Some(SetupPause { ready, resume })).await
        });
        let (old_pid, identity) =
            tokio::time::timeout(std::time::Duration::from_secs(2), installed)
                .await
                .unwrap()
                .unwrap();
        assert_eq!(identity, alice.to_string());
        setup.abort();
        assert!(matches!(setup.await, Err(error) if error.is_cancelled()));

        let clean = pool.get().await.unwrap();
        let row = clean.query_one("SELECT pg_backend_pid(), current_user::text, nullif(current_setting('request.jwt.claim.sub', true), '')", &[]).await.unwrap();
        assert_ne!(
            row.get::<_, i32>(0),
            old_pid,
            "cancelled setup backend reentered the pool"
        );
        assert_eq!(row.get::<_, String>(1), "flicknote_mcp");
        assert_eq!(row.get::<_, Option<String>>(2), None);
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            while clean
                .query_one(
                    "SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE pid=$1)",
                    &[&old_pid],
                )
                .await
                .unwrap()
                .get::<_, bool>(0)
            {
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("cancelled setup backend did not disconnect");
        drop(clean);
        let b = PgRequestDb::begin(&pool, bob).await.unwrap();
        assert!(b.resolve_note_id(&created.uuid).await.is_err());
        b.finish(true).await.unwrap();
        let a = PgRequestDb::begin(&pool, alice).await.unwrap();
        assert!(a.resolve_note_id(&created.uuid).await.is_ok());
        a.finish(true).await.unwrap();
    }

    #[test]
    fn pgroonga_snippet_preserves_utf8_and_highlight_boundaries() {
        let parts = snippet(
            "前文 中文 苹果 and English APPLE 尾声",
            &["苹果".into(), "apple".into()],
        );
        assert!(
            parts
                .iter()
                .any(|part| part.text == "苹果" && part.highlighted)
        );
        assert!(
            parts
                .iter()
                .any(|part| part.text == "APPLE" && part.highlighted)
        );
        assert_eq!(
            parts
                .iter()
                .map(|part| part.text.as_str())
                .collect::<String>(),
            "前文 中文 苹果 and English APPLE 尾声"
        );
    }
}
