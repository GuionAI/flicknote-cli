use std::collections::HashMap;
use std::fmt;

use reqwest::blocking::Client;
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderValue};
use serde::Deserialize;

use crate::backend::{InsertNoteReq, NoteDb, NoteFilter, validate_id_prefix};
use crate::error::CliError;
use crate::types::{Keyterm, Note, Project, Prompt};

// ─── Column select constants ──────────────────────────────────────────────────

const NOTE_SELECT: &str = "id,user_id,type,status,title,content,summary,is_flagged,project_id,metadata,source,external_id,created_at,updated_at,deleted_at";
const PROJECT_SELECT: &str = "id,user_id,name,color,prompt_id,keyterm_id,is_archived,created_at";
const PROMPT_SELECT: &str = "id,user_id,title,description,prompt,created_at";
const KEYTERM_SELECT: &str = "id,user_id,name,description,content,created_at,updated_at";

// ─── Response row types ───────────────────────────────────────────────────────

/// Intermediate response type for PostgREST note deserialization.
/// Handles type differences: bool→i64 for is_flagged, serde_json::Value→String for metadata.
#[derive(Deserialize)]
struct NoteRow {
    id: String,
    user_id: String,
    r#type: String,
    status: String,
    title: Option<String>,
    content: Option<String>,
    summary: Option<String>,
    is_flagged: Option<bool>,
    project_id: Option<String>,
    metadata: Option<serde_json::Value>,
    source: Option<String>,
    external_id: Option<String>,
    created_at: Option<String>,
    updated_at: Option<String>,
    deleted_at: Option<String>,
}

impl From<NoteRow> for Note {
    fn from(r: NoteRow) -> Self {
        Self {
            id: r.id,
            user_id: r.user_id,
            r#type: r.r#type,
            status: r.status,
            title: r.title,
            content: r.content,
            summary: r.summary,
            is_flagged: r.is_flagged.map(|b| if b { 1 } else { 0 }),
            project_id: r.project_id,
            metadata: r.metadata.map(|v| v.to_string()),
            source: r.source,
            external_id: r.external_id,
            created_at: r.created_at,
            updated_at: r.updated_at,
            deleted_at: r.deleted_at,
        }
    }
}

/// Intermediate response type for PostgREST project deserialization.
/// Handles bool→i64 for is_archived.
#[derive(Deserialize)]
struct ProjectRow {
    id: String,
    user_id: String,
    name: String,
    color: Option<String>,
    prompt_id: Option<String>,
    keyterm_id: Option<String>,
    is_archived: Option<bool>,
    created_at: Option<String>,
}

impl From<ProjectRow> for Project {
    fn from(r: ProjectRow) -> Self {
        Self {
            id: r.id,
            user_id: r.user_id,
            name: r.name,
            color: r.color,
            prompt_id: r.prompt_id,
            keyterm_id: r.keyterm_id,
            is_archived: r.is_archived.map(|b| if b { 1 } else { 0 }),
            created_at: r.created_at,
        }
    }
}

#[derive(Deserialize)]
struct IdRow {
    id: String,
}

#[derive(Deserialize)]
struct NameRow {
    name: String,
}

#[derive(Deserialize)]
struct ContentRow {
    content: Option<String>,
}

/// PostgREST returns count as string: [{"count":"42"}]
#[derive(Deserialize)]
struct CountRow {
    count: String,
}

#[derive(Deserialize)]
struct TopicRow {
    note_id: String,
    value: String,
}

// ─── PostgRestBackend ─────────────────────────────────────────────────────────

pub struct PostgRestBackend {
    client: Client,
    base_url: String,
    anon_key: String,
    access_token: String,
    user_id: String,
}

/// Manual Debug impl to avoid leaking secrets in logs.
impl fmt::Debug for PostgRestBackend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PostgRestBackend")
            .field("base_url", &self.base_url)
            .field("anon_key", &"[redacted]")
            .field("access_token", &"[redacted]")
            .field("user_id", &self.user_id)
            .finish()
    }
}

impl PostgRestBackend {
    pub fn new(base_url: &str, anon_key: String, access_token: String, user_id: String) -> Self {
        Self {
            client: Client::new(),
            base_url: base_url.trim_end_matches('/').to_string(),
            anon_key,
            access_token,
            user_id,
        }
    }

    /// Build default headers for all requests.
    fn headers(&self) -> Result<HeaderMap, CliError> {
        let mut h = HeaderMap::new();
        h.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {}", self.access_token))
                .map_err(|_| CliError::Other("Invalid token bytes in session".into()))?,
        );
        h.insert(
            "apikey",
            HeaderValue::from_str(&self.anon_key)
                .map_err(|_| CliError::Other("Invalid anon key bytes".into()))?,
        );
        h.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        Ok(h)
    }

    /// GET a list of rows from a table with query params.
    fn get_rows<T: for<'de> Deserialize<'de>>(
        &self,
        table: &str,
        query: &[(&str, &str)],
    ) -> Result<Vec<T>, CliError> {
        let resp = self
            .client
            .get(format!("{}/{table}", self.base_url))
            .headers(self.headers()?)
            .query(query)
            .send()
            .map_err(|e| CliError::Http(e.to_string()))?;
        if !resp.status().is_success() {
            let body = resp
                .text()
                .unwrap_or_else(|e| format!("[body read failed: {e}]"));
            return Err(CliError::Http(format!("{table} query failed: {body}")));
        }
        resp.json().map_err(|e| CliError::Http(e.to_string()))
    }

    /// GET a single row. Returns `CliError::Http("not found in {table}")` on 406/404-PGRST116,
    /// propagates all other errors as-is.
    fn get_one<T: for<'de> Deserialize<'de>>(
        &self,
        table: &str,
        query: &[(&str, &str)],
    ) -> Result<T, CliError> {
        let mut headers = self.headers()?;
        headers.insert(
            "Accept",
            HeaderValue::from_static("application/vnd.pgrst.object+json"),
        );
        let resp = self
            .client
            .get(format!("{}/{table}", self.base_url))
            .headers(headers)
            .query(query)
            .send()
            .map_err(|e| CliError::Http(e.to_string()))?;

        let status = resp.status().as_u16();

        // PostgREST v10: 406 = no rows for single-object request.
        // PostgREST v11+: 404 with PGRST116 code = no rows.
        if status == 406 {
            return Err(CliError::Http(format!("not found in {table}")));
        }
        if status == 404 {
            let body = resp
                .text()
                .unwrap_or_else(|e| format!("[body read failed: {e}]"));
            if body.contains("PGRST116") {
                return Err(CliError::Http(format!("not found in {table}")));
            }
            return Err(CliError::Http(format!("{table} query failed: {body}")));
        }
        if !resp.status().is_success() {
            let body = resp
                .text()
                .unwrap_or_else(|e| format!("[body read failed: {e}]"));
            return Err(CliError::Http(format!("{table} query failed: {body}")));
        }
        resp.json().map_err(|e| CliError::Http(e.to_string()))
    }

    /// POST (insert) a row.
    fn post(&self, table: &str, body: &impl serde::Serialize) -> Result<(), CliError> {
        let mut headers = self.headers()?;
        headers.insert("Prefer", HeaderValue::from_static("return=minimal"));
        let resp = self
            .client
            .post(format!("{}/{table}", self.base_url))
            .headers(headers)
            .json(body)
            .send()
            .map_err(|e| CliError::Http(e.to_string()))?;
        if !resp.status().is_success() {
            let body = resp
                .text()
                .unwrap_or_else(|e| format!("[body read failed: {e}]"));
            return Err(CliError::Http(format!("{table} insert failed: {body}")));
        }
        Ok(())
    }

    /// PATCH (update) rows matching query. Returns count of affected rows.
    /// Errors if Content-Range header is absent or unparseable.
    fn patch(
        &self,
        table: &str,
        query: &[(&str, &str)],
        body: &impl serde::Serialize,
    ) -> Result<u64, CliError> {
        let mut headers = self.headers()?;
        headers.insert(
            "Prefer",
            HeaderValue::from_static("return=headers-only,count=exact"),
        );
        let resp = self
            .client
            .patch(format!("{}/{table}", self.base_url))
            .headers(headers)
            .query(query)
            .json(body)
            .send()
            .map_err(|e| CliError::Http(e.to_string()))?;
        if !resp.status().is_success() {
            let body = resp
                .text()
                .unwrap_or_else(|e| format!("[body read failed: {e}]"));
            return Err(CliError::Http(format!("{table} update failed: {body}")));
        }
        // Parse affected count from Content-Range header: "*/N"
        let range_hdr = resp
            .headers()
            .get("content-range")
            .and_then(|v| v.to_str().ok())
            .ok_or_else(|| {
                CliError::Http(format!(
                    "PATCH {table} response missing Content-Range header"
                ))
            })?
            .to_owned();
        let count = range_hdr
            .split('/')
            .next_back()
            .and_then(|n| n.parse::<u64>().ok())
            .ok_or_else(|| {
                CliError::Http(format!("unexpected Content-Range format: {range_hdr}"))
            })?;
        Ok(count)
    }

    /// DELETE rows matching query.
    fn delete(&self, table: &str, query: &[(&str, &str)]) -> Result<(), CliError> {
        let resp = self
            .client
            .delete(format!("{}/{table}", self.base_url))
            .headers(self.headers()?)
            .query(query)
            .send()
            .map_err(|e| CliError::Http(e.to_string()))?;
        if !resp.status().is_success() {
            let body = resp
                .text()
                .unwrap_or_else(|e| format!("[body read failed: {e}]"));
            return Err(CliError::Http(format!("{table} delete failed: {body}")));
        }
        Ok(())
    }

    /// Resolve an ID prefix to a full ID. Covers the common resolve pattern across all tables.
    /// `extra_filters` adds additional query params (e.g. `[("deleted_at", "is.null")]`).
    /// `not_found` produces the appropriate error when 0 rows match.
    fn resolve_id(
        &self,
        table: &str,
        prefix: &str,
        extra_filters: &[(&str, &str)],
        not_found: impl Fn(String) -> CliError,
    ) -> Result<String, CliError> {
        validate_id_prefix(prefix)?;
        let id_filter = format!("like.{prefix}*");
        let mut query: Vec<(&str, &str)> =
            vec![("select", "id"), ("id", &id_filter), ("limit", "2")];
        query.extend_from_slice(extra_filters);
        let rows: Vec<IdRow> = self.get_rows(table, &query)?;
        match rows.len() {
            0 => Err(not_found(prefix.to_string())),
            1 => Ok(rows[0].id.clone()),
            _ => Err(CliError::Other(format!(
                "Ambiguous {table} ID prefix: {prefix}"
            ))),
        }
    }

    /// Returns `true` if this error came from `get_one` finding no rows
    /// (PostgREST 406 v10 or 404-PGRST116 v11+).
    fn is_not_found_err(e: &CliError, table: &str) -> bool {
        matches!(e, CliError::Http(msg) if msg == &format!("not found in {table}"))
    }
}

// ─── NoteDb impl ─────────────────────────────────────────────────────────────

impl NoteDb for PostgRestBackend {
    fn user_id(&self) -> &str {
        &self.user_id
    }

    // ── Note resolution ───────────────────────────────────────────────────────

    fn resolve_note_id(&self, prefix: &str) -> Result<String, CliError> {
        self.resolve_id("notes", prefix, &[("deleted_at", "is.null")], |id| {
            CliError::NoteNotFound { id }
        })
    }

    fn resolve_archived_note_id(&self, prefix: &str) -> Result<String, CliError> {
        self.resolve_id("notes", prefix, &[("deleted_at", "not.is.null")], |id| {
            CliError::NoteNotFound { id }
        })
    }

    // ── Note reads ────────────────────────────────────────────────────────────

    fn find_note(&self, id: &str) -> Result<Note, CliError> {
        let row: NoteRow = self
            .get_one(
                "notes",
                &[
                    ("select", NOTE_SELECT),
                    ("id", &format!("eq.{id}")),
                    ("deleted_at", "is.null"),
                ],
            )
            .map_err(|e| {
                if Self::is_not_found_err(&e, "notes") {
                    CliError::NoteNotFound { id: id.to_string() }
                } else {
                    e
                }
            })?;
        Ok(row.into())
    }

    fn find_archived_note(&self, id: &str) -> Result<Note, CliError> {
        let row: NoteRow = self
            .get_one(
                "notes",
                &[
                    ("select", NOTE_SELECT),
                    ("id", &format!("eq.{id}")),
                    ("deleted_at", "not.is.null"),
                ],
            )
            .map_err(|e| {
                if Self::is_not_found_err(&e, "notes") {
                    CliError::NoteNotFound { id: id.to_string() }
                } else {
                    e
                }
            })?;
        Ok(row.into())
    }

    fn find_note_content(&self, id: &str) -> Result<Option<String>, CliError> {
        let row: ContentRow = self
            .get_one(
                "notes",
                &[
                    ("select", "content"),
                    ("id", &format!("eq.{id}")),
                    ("deleted_at", "is.null"),
                ],
            )
            .map_err(|e| {
                if Self::is_not_found_err(&e, "notes") {
                    CliError::NoteNotFound { id: id.to_string() }
                } else {
                    e
                }
            })?;
        Ok(row.content)
    }

    fn list_notes(&self, filter: &NoteFilter<'_>) -> Result<Vec<Note>, CliError> {
        let archive_filter = if filter.archived {
            "not.is.null"
        } else {
            "is.null"
        };
        let limit_str = filter.limit.to_string();
        let mut query: Vec<(&str, String)> = vec![
            ("select", NOTE_SELECT.to_string()),
            ("deleted_at", archive_filter.to_string()),
            ("order", "created_at.desc".to_string()),
            ("limit", limit_str),
        ];
        if let Some(t) = filter.note_type {
            query.push(("type", format!("eq.{t}")));
        }
        if let Some(p) = filter.project_id {
            query.push(("project_id", format!("eq.{p}")));
        }
        let query_refs: Vec<(&str, &str)> = query.iter().map(|(k, v)| (*k, v.as_str())).collect();
        let rows: Vec<NoteRow> = self.get_rows("notes", &query_refs)?;
        Ok(rows.into_iter().map(Into::into).collect())
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
        let archive_filter = if filter.archived {
            "not.is.null"
        } else {
            "is.null"
        };
        let limit_str = filter.limit.to_string();

        // Build OR clause. Keywords are double-quoted so PostgREST treats the ilike value as a
        // literal string — commas and parens inside the keyword are safe. Any literal `"` in the
        // keyword is stripped since it has no search value and would break the quoting.
        let or_parts: Vec<String> = keywords
            .iter()
            .flat_map(|kw| {
                let safe = kw.replace('"', "");
                vec![
                    format!("title.ilike.\"*{safe}*\""),
                    format!("content.ilike.\"*{safe}*\""),
                    format!("summary.ilike.\"*{safe}*\""),
                ]
            })
            .collect();
        let or_clause = format!("({})", or_parts.join(","));

        let mut query: Vec<(&str, String)> = vec![
            ("select", NOTE_SELECT.to_string()),
            ("deleted_at", archive_filter.to_string()),
            ("or", or_clause),
            ("order", "updated_at.desc".to_string()),
            ("limit", limit_str),
        ];
        if let Some(p) = filter.project_id {
            query.push(("project_id", format!("eq.{p}")));
        }
        let query_refs: Vec<(&str, &str)> = query.iter().map(|(k, v)| (*k, v.as_str())).collect();
        let rows: Vec<NoteRow> = self.get_rows("notes", &query_refs)?;
        Ok(rows.into_iter().map(Into::into).collect())
    }

    // ── Note writes ───────────────────────────────────────────────────────────

    fn insert_note(&self, req: &InsertNoteReq<'_>) -> Result<(), CliError> {
        // Validate metadata JSON upfront — bad JSON must not silently coerce to null.
        let metadata_value: Option<serde_json::Value> = req
            .metadata
            .map(|m| {
                serde_json::from_str::<serde_json::Value>(m)
                    .map_err(|e| CliError::Other(format!("invalid metadata JSON: {e}")))
            })
            .transpose()?;

        self.post(
            "notes",
            &serde_json::json!({
                "id": req.id,
                "user_id": self.user_id,
                "type": req.note_type,
                "status": req.status,
                "title": req.title,
                "content": req.content,
                "metadata": metadata_value,
                "project_id": req.project_id,
                "created_at": req.now,
                "updated_at": req.now,
            }),
        )
    }

    fn update_note_content(&self, id: &str, content: &str, requeue: bool) -> Result<(), CliError> {
        let now = chrono::Utc::now().to_rfc3339();
        let user_id = self.user_id.clone();
        let mut body = serde_json::json!({ "content": content, "updated_at": now });
        if requeue {
            body["status"] = serde_json::json!("ai_queued");
        }
        self.patch(
            "notes",
            &[
                ("id", &format!("eq.{id}")),
                ("user_id", &format!("eq.{user_id}")),
            ],
            &body,
        )?;
        Ok(())
    }

    fn set_note_deleted_at(
        &self,
        id: &str,
        deleted_at: Option<&str>,
        now: &str,
    ) -> Result<(), CliError> {
        let user_id = self.user_id.clone();
        let body = serde_json::json!({
            "deleted_at": deleted_at,
            "updated_at": now,
        });
        self.patch(
            "notes",
            &[
                ("id", &format!("eq.{id}")),
                ("user_id", &format!("eq.{user_id}")),
            ],
            &body,
        )?;
        Ok(())
    }

    fn undo_last_delete(&self) -> Result<(), CliError> {
        // Step 1: Find most recently deleted note for this user
        let rows: Vec<IdRow> = self.get_rows(
            "notes",
            &[
                ("select", "id"),
                ("deleted_at", "not.is.null"),
                ("user_id", &format!("eq.{}", self.user_id)),
                ("order", "deleted_at.desc"),
                ("limit", "1"),
            ],
        )?;
        let Some(row) = rows.first() else {
            return Ok(()); // Nothing to undo
        };
        // Step 2: Restore it (user_id filter as defense-in-depth)
        let now = chrono::Utc::now().to_rfc3339();
        self.patch(
            "notes",
            &[
                ("id", &format!("eq.{}", row.id)),
                ("user_id", &format!("eq.{}", self.user_id)),
            ],
            &serde_json::json!({ "deleted_at": null, "updated_at": now }),
        )?;
        Ok(())
    }

    // ── Project reads ─────────────────────────────────────────────────────────

    fn find_project_by_name(&self, name: &str) -> Result<Option<String>, CliError> {
        let rows: Vec<IdRow> = self.get_rows(
            "projects",
            &[
                ("select", "id"),
                ("name", &format!("eq.{name}")),
                ("is_archived", "eq.false"),
                ("limit", "1"),
            ],
        )?;
        Ok(rows.into_iter().next().map(|r| r.id))
    }

    fn find_project_name_by_id(&self, project_id: &str) -> Result<Option<String>, CliError> {
        let rows: Vec<NameRow> = self.get_rows(
            "projects",
            &[
                ("select", "name"),
                ("id", &format!("eq.{project_id}")),
                ("limit", "1"),
            ],
        )?;
        Ok(rows.into_iter().next().map(|r| r.name))
    }

    fn list_projects(&self, archived: bool) -> Result<Vec<Project>, CliError> {
        let archived_filter = if archived { "eq.true" } else { "eq.false" };
        let rows: Vec<ProjectRow> = self.get_rows(
            "projects",
            &[
                ("select", PROJECT_SELECT),
                ("is_archived", archived_filter),
                ("order", "name.asc"),
            ],
        )?;
        Ok(rows.into_iter().map(Into::into).collect())
    }

    fn find_project(&self, id: &str) -> Result<Project, CliError> {
        let row: ProjectRow = self
            .get_one(
                "projects",
                &[("select", PROJECT_SELECT), ("id", &format!("eq.{id}"))],
            )
            .map_err(|e| {
                if Self::is_not_found_err(&e, "projects") {
                    CliError::ProjectNotFound {
                        name: id.to_string(),
                    }
                } else {
                    e
                }
            })?;
        Ok(row.into())
    }

    fn resolve_project_id(&self, prefix: &str) -> Result<String, CliError> {
        self.resolve_id("projects", prefix, &[], |name| CliError::ProjectNotFound {
            name,
        })
    }

    // ── Project writes ────────────────────────────────────────────────────────

    fn create_project(&self, name: &str) -> Result<String, CliError> {
        let id = uuid::Uuid::new_v4().to_string();
        let now = chrono::Utc::now().to_rfc3339();
        self.post(
            "projects",
            &serde_json::json!({
                "id": id,
                "user_id": self.user_id,
                "name": name,
                "is_archived": false,
                "created_at": now,
            }),
        )?;
        Ok(id)
    }

    fn move_note_to_project(
        &self,
        note_id: &str,
        new_project_id: &str,
        _old_project_id: Option<&str>,
    ) -> Result<Option<String>, CliError> {
        let now = chrono::Utc::now().to_rfc3339();
        let user_id = self.user_id.clone();

        let affected = self.patch(
            "notes",
            &[
                ("id", &format!("eq.{note_id}")),
                ("user_id", &format!("eq.{user_id}")),
            ],
            &serde_json::json!({ "project_id": new_project_id, "updated_at": now }),
        )?;
        if affected == 0 {
            return Err(CliError::NoteNotFound {
                id: note_id.to_string(),
            });
        }
        // Empty-project cleanup is not performed over PostgREST (no multi-statement
        // transactions without RPC). Callers should treat Ok(None) as "no project deleted".
        Ok(None)
    }

    fn update_project(
        &self,
        id: &str,
        prompt_id: Option<Option<&str>>,
        keyterm_id: Option<Option<&str>>,
        color: Option<Option<&str>>,
    ) -> Result<(), CliError> {
        let user_id = self.user_id.clone();
        let mut body = serde_json::Map::new();
        if let Some(val) = prompt_id {
            body.insert(
                "prompt_id".to_string(),
                val.map_or(serde_json::Value::Null, |v| serde_json::json!(v)),
            );
        }
        if let Some(val) = keyterm_id {
            body.insert(
                "keyterm_id".to_string(),
                val.map_or(serde_json::Value::Null, |v| serde_json::json!(v)),
            );
        }
        if let Some(val) = color {
            body.insert(
                "color".to_string(),
                val.map_or(serde_json::Value::Null, |v| serde_json::json!(v)),
            );
        }
        if body.is_empty() {
            return Ok(());
        }
        self.patch(
            "projects",
            &[
                ("id", &format!("eq.{id}")),
                ("user_id", &format!("eq.{user_id}")),
            ],
            &serde_json::Value::Object(body),
        )?;
        Ok(())
    }

    fn delete_project(&self, id: &str) -> Result<(), CliError> {
        // This is an ARCHIVE, not a hard delete. Matches PG_ARCHIVE_PROJECT behaviour.
        let user_id = self.user_id.clone();
        let affected = self.patch(
            "projects",
            &[
                ("id", &format!("eq.{id}")),
                ("user_id", &format!("eq.{user_id}")),
            ],
            &serde_json::json!({ "is_archived": true }),
        )?;
        if affected == 0 {
            return Err(CliError::ProjectNotFound {
                name: id.to_string(),
            });
        }
        Ok(())
    }

    // ── Note metadata writes ──────────────────────────────────────────────────

    fn update_note_title(&self, id: &str, title: &str) -> Result<(), CliError> {
        let now = chrono::Utc::now().to_rfc3339();
        let user_id = self.user_id.clone();
        self.patch(
            "notes",
            &[
                ("id", &format!("eq.{id}")),
                ("user_id", &format!("eq.{user_id}")),
            ],
            &serde_json::json!({ "title": title, "updated_at": now }),
        )?;
        Ok(())
    }

    fn update_note_flagged(&self, id: &str, flagged: bool) -> Result<(), CliError> {
        let now = chrono::Utc::now().to_rfc3339();
        let user_id = self.user_id.clone();
        self.patch(
            "notes",
            &[
                ("id", &format!("eq.{id}")),
                ("user_id", &format!("eq.{user_id}")),
            ],
            &serde_json::json!({ "is_flagged": flagged, "updated_at": now }),
        )?;
        Ok(())
    }

    // ── Note reads (extended) ─────────────────────────────────────────────────

    fn count_notes(&self, filter: &NoteFilter<'_>) -> Result<u64, CliError> {
        let archive_filter = if filter.archived {
            "not.is.null"
        } else {
            "is.null"
        };
        let mut query: Vec<(&str, String)> = vec![
            ("select", "count".to_string()),
            ("deleted_at", archive_filter.to_string()),
        ];
        if let Some(t) = filter.note_type {
            query.push(("type", format!("eq.{t}")));
        }
        if let Some(pid) = filter.project_id {
            query.push(("project_id", format!("eq.{pid}")));
        }
        let query_refs: Vec<(&str, &str)> = query.iter().map(|(k, v)| (*k, v.as_str())).collect();
        let rows: Vec<CountRow> = self.get_rows("notes", &query_refs)?;
        let count_str = rows.first().map(|r| r.count.as_str()).unwrap_or("0");
        count_str
            .parse::<u64>()
            .map_err(|_| CliError::Other(format!("unexpected count value: {count_str}")))
    }

    fn list_note_topics(
        &self,
        note_ids: &[&str],
    ) -> Result<HashMap<String, Vec<String>>, CliError> {
        if note_ids.is_empty() {
            return Ok(HashMap::new());
        }
        let in_clause = format!("in.({})", note_ids.join(","));
        let rows: Vec<TopicRow> = self.get_rows(
            "note_extractions",
            &[
                ("select", "note_id,value"),
                ("note_id", &in_clause),
                ("type", "eq.topic"),
            ],
        )?;
        let mut map: HashMap<String, Vec<String>> = HashMap::new();
        for row in rows {
            map.entry(row.note_id).or_default().push(row.value);
        }
        Ok(map)
    }

    // ── Prompt operations ─────────────────────────────────────────────────────

    fn resolve_prompt_id(&self, prefix: &str) -> Result<String, CliError> {
        self.resolve_id("prompts", prefix, &[], |id| {
            CliError::Other(format!("Prompt not found: {id}"))
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
        self.post(
            "prompts",
            &serde_json::json!({
                "id": id,
                "user_id": self.user_id,
                "title": title,
                "description": description,
                "prompt": prompt,
                "created_at": now,
            }),
        )
    }

    fn find_prompt(&self, id: &str) -> Result<Prompt, CliError> {
        let row: Prompt = self
            .get_one(
                "prompts",
                &[("select", PROMPT_SELECT), ("id", &format!("eq.{id}"))],
            )
            .map_err(|e| {
                if Self::is_not_found_err(&e, "prompts") {
                    CliError::Other(format!("Prompt not found: {id}"))
                } else {
                    e
                }
            })?;
        Ok(row)
    }

    fn list_prompts(&self) -> Result<Vec<Prompt>, CliError> {
        let rows: Vec<Prompt> = self.get_rows(
            "prompts",
            &[("select", PROMPT_SELECT), ("order", "created_at.desc")],
        )?;
        Ok(rows)
    }

    fn update_prompt(
        &self,
        id: &str,
        title: Option<&str>,
        description: Option<&str>,
        prompt: Option<&str>,
    ) -> Result<(), CliError> {
        let user_id = self.user_id.clone();
        let mut body = serde_json::Map::new();
        if let Some(v) = title {
            body.insert("title".to_string(), serde_json::json!(v));
        }
        if let Some(v) = description {
            body.insert("description".to_string(), serde_json::json!(v));
        }
        if let Some(v) = prompt {
            body.insert("prompt".to_string(), serde_json::json!(v));
        }
        if body.is_empty() {
            return Ok(());
        }
        self.patch(
            "prompts",
            &[
                ("id", &format!("eq.{id}")),
                ("user_id", &format!("eq.{user_id}")),
            ],
            &serde_json::Value::Object(body),
        )?;
        Ok(())
    }

    fn delete_prompt(&self, id: &str) -> Result<(), CliError> {
        let user_id = self.user_id.clone();
        self.delete(
            "prompts",
            &[
                ("id", &format!("eq.{id}")),
                ("user_id", &format!("eq.{user_id}")),
            ],
        )
    }

    // ── Keyterm operations ────────────────────────────────────────────────────

    fn resolve_keyterm_id(&self, prefix: &str) -> Result<String, CliError> {
        self.resolve_id("keyterms", prefix, &[], |id| {
            CliError::Other(format!("Keyterm not found: {id}"))
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
        self.post(
            "keyterms",
            &serde_json::json!({
                "id": id,
                "user_id": self.user_id,
                "name": name,
                "description": description,
                "content": content,
                "created_at": now,
                "updated_at": now,
            }),
        )
    }

    fn find_keyterm(&self, id: &str) -> Result<Keyterm, CliError> {
        let row: Keyterm = self
            .get_one(
                "keyterms",
                &[("select", KEYTERM_SELECT), ("id", &format!("eq.{id}"))],
            )
            .map_err(|e| {
                if Self::is_not_found_err(&e, "keyterms") {
                    CliError::Other(format!("Keyterm not found: {id}"))
                } else {
                    e
                }
            })?;
        Ok(row)
    }

    fn list_keyterms(&self) -> Result<Vec<Keyterm>, CliError> {
        let rows: Vec<Keyterm> = self.get_rows(
            "keyterms",
            &[("select", KEYTERM_SELECT), ("order", "name.asc")],
        )?;
        Ok(rows)
    }

    fn update_keyterm(
        &self,
        id: &str,
        name: Option<&str>,
        description: Option<&str>,
        content: Option<&str>,
    ) -> Result<(), CliError> {
        // Guard: if no fields are changing, skip the PATCH.
        if name.is_none() && description.is_none() && content.is_none() {
            return Ok(());
        }
        let now = chrono::Utc::now().to_rfc3339();
        let user_id = self.user_id.clone();
        let mut body = serde_json::Map::new();
        body.insert("updated_at".to_string(), serde_json::json!(now));
        if let Some(v) = name {
            body.insert("name".to_string(), serde_json::json!(v));
        }
        if let Some(v) = description {
            body.insert("description".to_string(), serde_json::json!(v));
        }
        if let Some(v) = content {
            body.insert("content".to_string(), serde_json::json!(v));
        }
        self.patch(
            "keyterms",
            &[
                ("id", &format!("eq.{id}")),
                ("user_id", &format!("eq.{user_id}")),
            ],
            &serde_json::Value::Object(body),
        )?;
        Ok(())
    }

    fn delete_keyterm(&self, id: &str) -> Result<(), CliError> {
        let user_id = self.user_id.clone();
        self.delete(
            "keyterms",
            &[
                ("id", &format!("eq.{id}")),
                ("user_id", &format!("eq.{user_id}")),
            ],
        )
    }
}
