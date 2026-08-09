use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;
use flicknote_auth::client::GoTrueClient;
use flicknote_core::{
    REMOTE_COMMITTED_INSERT_METADATA, TOPIC_EXTRACTION_KEY,
    backend::InsertedNote,
    config::Config,
    services::ports::{CreateNote, CreatedNote, NoteCreator},
};
use powersync::PowerSyncDatabase;
use rusqlite::{OptionalExtension, params};
use serde::Deserialize;

use crate::ipc::DaemonError;
use crate::remote::attachment::{delete_attachment, upload_attachment};

#[cfg(test)]
mod tests;

#[derive(Debug, Default, PartialEq, Eq)]
struct ExtractionCreateOutcome {
    confirmed_ids: Vec<String>,
    pending_ids: Vec<String>,
    diagnostic: Option<String>,
    local_commit_error: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct RemoteNoteRow {
    id: String,
    short_id: Option<i64>,
    user_id: String,
    #[serde(rename = "type")]
    note_type: String,
    status: String,
    title: Option<String>,
    content: Option<String>,
    summary: Option<String>,
    #[serde(default)]
    is_flagged: bool,
    project_id: Option<String>,
    metadata: Option<serde_json::Value>,
    source: Option<serde_json::Value>,
    created_at: Option<String>,
    updated_at: Option<String>,
    deleted_at: Option<String>,
}

fn json_column(value: &Option<serde_json::Value>) -> Result<Option<String>, DaemonError> {
    value
        .as_ref()
        .map(serde_json::to_string)
        .transpose()
        .map_err(|error| DaemonError::Other {
            message: format!("Failed to serialize canonical remote JSON: {error}"),
        })
}

async fn commit_remote_note(
    db: &PowerSyncDatabase,
    note: &RemoteNoteRow,
) -> Result<bool, DaemonError> {
    let metadata = json_column(&note.metadata)?;
    let source = json_column(&note.source)?;
    let mut writer = db.writer().await.map_err(|error| DaemonError::Other {
        message: format!("Failed to open local PowerSync writer: {error}"),
    })?;
    let tx = writer
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(|error| DaemonError::Other {
            message: format!("Failed to begin local note transaction: {error}"),
        })?;
    let exists = tx
        .query_row(
            "SELECT 1 FROM notes WHERE id = ? LIMIT 1",
            params![note.id],
            |_| Ok(()),
        )
        .optional()
        .map_err(|error| DaemonError::Other {
            message: format!("Failed to check local note {}: {error}", note.id),
        })?
        .is_some();
    if exists {
        tx.commit().map_err(|error| DaemonError::Other {
            message: format!("Failed to finish local note transaction: {error}"),
        })?;
        return Ok(false);
    }

    tx.execute(
        r#"INSERT INTO notes (
            id, short_id, user_id, type, status, title, content, summary,
            is_flagged, project_id, metadata, source, created_at, updated_at,
            deleted_at, _metadata
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"#,
        params![
            note.id,
            note.short_id,
            note.user_id,
            note.note_type,
            note.status,
            note.title,
            note.content,
            note.summary,
            note.is_flagged,
            note.project_id,
            metadata,
            source,
            note.created_at,
            note.updated_at,
            note.deleted_at,
            REMOTE_COMMITTED_INSERT_METADATA,
        ],
    )
    .map_err(|error| DaemonError::Other {
        message: format!("Failed to commit remote note {} locally: {error}", note.id),
    })?;
    tx.commit().map_err(|error| DaemonError::Other {
        message: format!("Failed to finish local note transaction: {error}"),
    })?;
    Ok(true)
}

#[derive(Debug, Clone, serde::Serialize, Deserialize)]
struct RemoteExtractionRow {
    id: String,
    note_id: String,
    user_id: String,
    key: String,
    value: String,
}

async fn commit_remote_extractions(
    db: &PowerSyncDatabase,
    rows: &[RemoteExtractionRow],
) -> Result<usize, DaemonError> {
    if rows.is_empty() {
        return Ok(0);
    }
    let mut writer = db.writer().await.map_err(|error| DaemonError::Other {
        message: format!("Failed to open local PowerSync writer: {error}"),
    })?;
    let tx = writer
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(|error| DaemonError::Other {
            message: format!("Failed to begin local extraction transaction: {error}"),
        })?;
    let mut inserted = 0;
    for row in rows {
        let exists = tx
            .query_row(
                "SELECT 1 FROM note_extractions WHERE id = ? LIMIT 1",
                params![row.id],
                |_| Ok(()),
            )
            .optional()
            .map_err(|error| DaemonError::Other {
                message: format!("Failed to check local extraction {}: {error}", row.id),
            })?
            .is_some();
        if exists {
            continue;
        }
        tx.execute(
            r#"INSERT INTO note_extractions (
                id, note_id, user_id, key, value, _metadata
            ) VALUES (?, ?, ?, ?, ?, ?)"#,
            params![
                row.id,
                row.note_id,
                row.user_id,
                row.key,
                row.value,
                REMOTE_COMMITTED_INSERT_METADATA,
            ],
        )
        .map_err(|error| DaemonError::Other {
            message: format!(
                "Failed to commit remote extraction {} locally: {error}",
                row.id
            ),
        })?;
        inserted += 1;
    }
    tx.commit().map_err(|error| DaemonError::Other {
        message: format!("Failed to finish local extraction transaction: {error}"),
    })?;
    Ok(inserted)
}

async fn create_note_remotely(
    db: &PowerSyncDatabase,
    http: &reqwest::Client,
    auth: &GoTrueClient,
    config: &Config,
    req: CreateNote,
) -> Result<CreatedNote, DaemonError> {
    let session = auth.get_session().await.map_err(|e| DaemonError::Other {
        message: format!("Auth error: {e}"),
    })?;

    create_note_with_token(
        db,
        http,
        config,
        &session.access_token,
        &session.user.id,
        req,
    )
    .await
}

enum NoteCreateAttempt {
    Response(NoteCreateResponse),
    Recovered(RemoteNoteRow),
}

struct NoteCreateResponse {
    response: reqwest::Response,
    initial_error: Option<String>,
}

fn extraction_rows(request: &CreateNote, user_id: &str) -> Vec<RemoteExtractionRow> {
    request
        .topics
        .iter()
        .map(|value| RemoteExtractionRow {
            id: uuid::Uuid::new_v4().to_string(),
            note_id: request.id.clone(),
            user_id: user_id.to_string(),
            key: TOPIC_EXTRACTION_KEY.to_string(),
            value: value.clone(),
        })
        .collect()
}

fn note_payload(request: &CreateNote, user_id: &str) -> Result<serde_json::Value, DaemonError> {
    let metadata = request
        .metadata
        .as_deref()
        .map(serde_json::from_str::<serde_json::Value>)
        .transpose()
        .map_err(|error| DaemonError::Other {
            message: format!("Invalid note metadata JSON: {error}"),
        })?
        .unwrap_or(serde_json::Value::Null);
    Ok(serde_json::json!({
        "id": request.id,
        "user_id": user_id,
        "type": request.note_type,
        "status": request.status,
        "title": request.title,
        "content": request.content,
        "metadata": metadata,
        "project_id": request.project_id,
        "created_at": request.now,
        "updated_at": request.now,
    }))
}

fn note_create_request(
    http: &reqwest::Client,
    config: &Config,
    access_token: &str,
    payload: &serde_json::Value,
) -> reqwest::RequestBuilder {
    http.post(format!(
        "{}/rest/v1/notes?on_conflict=id",
        config.supabase_url
    ))
    .header("apikey", &config.supabase_anon_key)
    .bearer_auth(access_token)
    .header(
        "Prefer",
        "resolution=ignore-duplicates,return=representation",
    )
    .json(payload)
}

async fn recover_ambiguous_create(
    http: &reqwest::Client,
    config: &Config,
    access_token: &str,
    note_id: &str,
    extraction_rows: &[RemoteExtractionRow],
    initial_error: &str,
    retry_error: &str,
) -> Result<NoteCreateAttempt, DaemonError> {
    if let Ok(Some(row)) = lookup_remote_note(http, config, access_token, note_id).await {
        return Ok(NoteCreateAttempt::Recovered(row));
    }
    Err(ambiguous_create_error(
        format!(
            "Remote note create outcome is unknown for note {note_id} after retrying the same stable UUID ({initial_error}; retry: {retry_error}). The attachment was retained. Do not create it again."
        ),
        note_id.to_string(),
        extraction_rows,
    ))
}

async fn send_note_create(
    http: &reqwest::Client,
    config: &Config,
    access_token: &str,
    note_id: &str,
    extraction_rows: &[RemoteExtractionRow],
    payload: &serde_json::Value,
) -> Result<NoteCreateAttempt, DaemonError> {
    let first = note_create_request(http, config, access_token, payload)
        .send()
        .await;
    let (initial_error, initial_was_status) = match first {
        Ok(response) if !is_ambiguous_create_status(response.status()) => {
            return Ok(NoteCreateAttempt::Response(NoteCreateResponse {
                response,
                initial_error: None,
            }));
        }
        Ok(response) => {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            (format!("the first attempt returned {status}: {body}"), true)
        }
        Err(error) => (error.to_string(), false),
    };

    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    match note_create_request(http, config, access_token, payload)
        .send()
        .await
    {
        Ok(response) if !initial_was_status || !is_ambiguous_create_status(response.status()) => {
            Ok(NoteCreateAttempt::Response(NoteCreateResponse {
                response,
                initial_error: Some(initial_error),
            }))
        }
        Ok(response) => {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            recover_ambiguous_create(
                http,
                config,
                access_token,
                note_id,
                extraction_rows,
                &initial_error,
                &format!("returned {status}: {body}"),
            )
            .await
        }
        Err(error) => {
            recover_ambiguous_create(
                http,
                config,
                access_token,
                note_id,
                extraction_rows,
                &initial_error,
                &error.to_string(),
            )
            .await
        }
    }
}

async fn canonical_note_row(
    http: &reqwest::Client,
    config: &Config,
    access_token: &str,
    note_id: &str,
    extraction_rows: &[RemoteExtractionRow],
    attachment_uploaded: bool,
    create_response: NoteCreateResponse,
) -> Result<RemoteNoteRow, DaemonError> {
    let NoteCreateResponse {
        response,
        initial_error,
    } = create_response;
    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        if let Ok(Some(row)) = lookup_remote_note(http, config, access_token, note_id).await {
            return Ok(row);
        }
        if let Some(initial_error) = initial_error {
            return Err(ambiguous_create_error(
                format!(
                    "Remote note create outcome is unknown for note {note_id} after retrying the same stable UUID: {initial_error}; the retry returned {status}: {body}. The attachment was retained. Do not create it again."
                ),
                note_id.to_string(),
                extraction_rows,
            ));
        }
        if attachment_uploaded
            && let Err(error) = delete_attachment(http, config, access_token, note_id).await
        {
            log::warn!("Failed to clean up uploaded attachment after note create failure: {error}");
        }
        return Err(DaemonError::Other {
            message: format!("Remote note create failed ({status}): {body}"),
        });
    }

    match response.json::<Vec<RemoteNoteRow>>().await {
        Ok(mut rows) => match rows.pop() {
            Some(row) => Ok(row),
            None => {
                reconcile_confirmed_remote_note(
                    http,
                    config,
                    access_token,
                    note_id,
                    extraction_rows,
                    format!("Remote note create returned no row for note {note_id}"),
                )
                .await
            }
        },
        Err(error) => {
            reconcile_confirmed_remote_note(
                http,
                config,
                access_token,
                note_id,
                extraction_rows,
                format!("Failed to parse remote note create response: {error}"),
            )
            .await
        }
    }
}

async fn create_note_with_token(
    db: &PowerSyncDatabase,
    http: &reqwest::Client,
    config: &Config,
    access_token: &str,
    user_id: &str,
    req: CreateNote,
) -> Result<CreatedNote, DaemonError> {
    let extraction_rows = extraction_rows(&req, user_id);
    let payload = note_payload(&req, user_id)?;
    let attachment_path = req.attachment_path.as_deref().map(Path::new);
    if let Some(path) = attachment_path {
        upload_attachment(http, config, access_token, &req.id, path).await?;
    }
    let row = match send_note_create(
        http,
        config,
        access_token,
        &req.id,
        &extraction_rows,
        &payload,
    )
    .await?
    {
        NoteCreateAttempt::Recovered(row) => row,
        NoteCreateAttempt::Response(response) => {
            canonical_note_row(
                http,
                config,
                access_token,
                &req.id,
                &extraction_rows,
                attachment_path.is_some(),
                response,
            )
            .await?
        }
    };
    finish_remote_create(db, http, config, access_token, row, &extraction_rows).await
}

fn is_ambiguous_create_status(status: reqwest::StatusCode) -> bool {
    status.is_server_error()
        || status == reqwest::StatusCode::REQUEST_TIMEOUT
        || status == reqwest::StatusCode::TOO_MANY_REQUESTS
}

fn confirmed_create_error(
    message: String,
    note_id: String,
    short_id: Option<i64>,
    extraction_rows: &[RemoteExtractionRow],
) -> DaemonError {
    DaemonError::PartialCreate {
        message,
        note_id,
        short_id,
        confirmed_extraction_ids: Vec::new(),
        pending_extraction_ids: extraction_rows.iter().map(|row| row.id.clone()).collect(),
    }
}

fn partial_create_error(
    message: String,
    note_id: String,
    short_id: Option<i64>,
    confirmed_extraction_ids: Vec<String>,
    pending_extraction_ids: Vec<String>,
) -> DaemonError {
    DaemonError::PartialCreate {
        message,
        note_id,
        short_id,
        confirmed_extraction_ids,
        pending_extraction_ids,
    }
}

fn ambiguous_create_error(
    message: String,
    note_id: String,
    extraction_rows: &[RemoteExtractionRow],
) -> DaemonError {
    DaemonError::AmbiguousCreate {
        message,
        note_id,
        pending_extraction_ids: extraction_rows.iter().map(|row| row.id.clone()).collect(),
    }
}

async fn reconcile_confirmed_remote_note(
    http: &reqwest::Client,
    config: &Config,
    access_token: &str,
    note_id: &str,
    extraction_rows: &[RemoteExtractionRow],
    original_error: String,
) -> Result<RemoteNoteRow, DaemonError> {
    match lookup_remote_note(http, config, access_token, note_id).await {
        Ok(Some(row)) => Ok(row),
        Ok(None) => Err(confirmed_create_error(
            format!(
                "Note {note_id} was accepted remotely, but its canonical row could not be recovered: {original_error}. Do not create it again."
            ),
            note_id.to_string(),
            None,
            extraction_rows,
        )),
        Err(error) => Err(confirmed_create_error(
            format!(
                "Note {note_id} was accepted remotely, but its canonical row could not be recovered: {original_error}; reconciliation failed: {error}. Do not create it again."
            ),
            note_id.to_string(),
            None,
            extraction_rows,
        )),
    }
}

async fn finish_remote_create(
    db: &PowerSyncDatabase,
    http: &reqwest::Client,
    config: &Config,
    access_token: &str,
    row: RemoteNoteRow,
    extraction_rows: &[RemoteExtractionRow],
) -> Result<CreatedNote, DaemonError> {
    let short_id = match row.short_id {
        Some(short_id) => short_id,
        None => {
            return Err(confirmed_create_error(
                format!(
                    "Note {} was created remotely, but the backend returned no short id. Do not create it again.",
                    row.id
                ),
                row.id,
                None,
                extraction_rows,
            ));
        }
    };
    if let Err(error) = commit_remote_note(db, &row).await {
        return Err(confirmed_create_error(
            format!(
                "Note {short_id} was created remotely, but could not be committed locally: {error}. Do not create it again."
            ),
            row.id,
            Some(short_id),
            extraction_rows,
        ));
    }
    let extraction_outcome =
        create_extractions_with_token(db, http, config, access_token, extraction_rows).await;
    if !extraction_outcome.pending_ids.is_empty() || extraction_outcome.local_commit_error.is_some()
    {
        let reason = extraction_outcome
            .local_commit_error
            .as_deref()
            .or(extraction_outcome.diagnostic.as_deref())
            .unwrap_or("one or more extraction rows could not be confirmed");
        return Err(partial_create_error(
            format!(
                "Note {short_id} was created, but its topics were not fully committed: {reason}"
            ),
            row.id,
            Some(short_id),
            extraction_outcome.confirmed_ids,
            extraction_outcome.pending_ids,
        ));
    }
    Ok(CreatedNote {
        inserted: InsertedNote {
            uuid: row.id,
            short_id: Some(short_id),
        },
        confirmed_extraction_ids: extraction_outcome.confirmed_ids,
    })
}

async fn lookup_remote_note(
    http: &reqwest::Client,
    config: &Config,
    access_token: &str,
    id: &str,
) -> Result<Option<RemoteNoteRow>, DaemonError> {
    let response = http
        .get(format!(
            "{}/rest/v1/notes?id=eq.{id}&select=*",
            config.supabase_url
        ))
        .header("apikey", &config.supabase_anon_key)
        .bearer_auth(access_token)
        .send()
        .await
        .map_err(|error| DaemonError::Other {
            message: format!("Failed to reconcile remote note {id}: {error}"),
        })?;
    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(DaemonError::Other {
            message: format!("Failed to reconcile remote note {id} ({status}): {body}"),
        });
    }
    let mut rows = response
        .json::<Vec<RemoteNoteRow>>()
        .await
        .map_err(|error| DaemonError::Other {
            message: format!("Failed to parse remote note reconciliation response: {error}"),
        })?;
    Ok(rows.pop())
}

async fn create_extractions_with_token(
    db: &PowerSyncDatabase,
    http: &reqwest::Client,
    config: &Config,
    access_token: &str,
    requested: &[RemoteExtractionRow],
) -> ExtractionCreateOutcome {
    if requested.is_empty() {
        return ExtractionCreateOutcome::default();
    }

    let response = http
        .post(format!(
            "{}/rest/v1/note_extractions?on_conflict=id",
            config.supabase_url
        ))
        .header("apikey", &config.supabase_anon_key)
        .bearer_auth(access_token)
        .header(
            "Prefer",
            "resolution=ignore-duplicates,return=representation",
        )
        .json(requested)
        .send()
        .await;
    let (mut rows, mut diagnostics) = match response {
        Ok(response) if response.status().is_success() => {
            match response.json::<Vec<RemoteExtractionRow>>().await {
                Ok(rows) => (rows, Vec::new()),
                Err(error) => (
                    Vec::new(),
                    vec![format!(
                        "failed to parse remote extraction create response: {error}"
                    )],
                ),
            }
        }
        Ok(response) => {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            (
                Vec::new(),
                vec![format!(
                    "remote extraction create returned {status}: {body}"
                )],
            )
        }
        Err(error) => (
            Vec::new(),
            vec![format!(
                "remote extraction create failed in transport: {error}"
            )],
        ),
    };
    let mut confirmed_ids = rows
        .iter()
        .map(|row| row.id.clone())
        .collect::<std::collections::HashSet<_>>();
    for expected in requested {
        if confirmed_ids.contains(&expected.id) {
            continue;
        }
        match lookup_remote_extraction(http, config, access_token, &expected.id).await {
            Ok(Some(row)) => {
                rows.push(row);
                confirmed_ids.insert(expected.id.clone());
            }
            Ok(None) => {}
            Err(error) => diagnostics.push(error.to_string()),
        }
    }
    let confirmed_ids = requested
        .iter()
        .filter(|row| confirmed_ids.contains(&row.id))
        .map(|row| row.id.clone())
        .collect::<Vec<_>>();
    let pending_ids = requested
        .iter()
        .filter(|row| !confirmed_ids.contains(&row.id))
        .map(|row| row.id.clone())
        .collect::<Vec<_>>();
    let local_commit_error = if rows.is_empty() {
        None
    } else {
        commit_remote_extractions(db, &rows)
            .await
            .err()
            .map(|error| error.to_string())
    };
    ExtractionCreateOutcome {
        confirmed_ids,
        pending_ids,
        diagnostic: (!diagnostics.is_empty()).then(|| diagnostics.join("; ")),
        local_commit_error,
    }
}

async fn lookup_remote_extraction(
    http: &reqwest::Client,
    config: &Config,
    access_token: &str,
    id: &str,
) -> Result<Option<RemoteExtractionRow>, DaemonError> {
    let response = http
        .get(format!(
            "{}/rest/v1/note_extractions?id=eq.{id}&select=*",
            config.supabase_url
        ))
        .header("apikey", &config.supabase_anon_key)
        .bearer_auth(access_token)
        .send()
        .await
        .map_err(|error| DaemonError::Other {
            message: format!("Failed to reconcile remote extraction {id}: {error}"),
        })?;
    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(DaemonError::Other {
            message: format!("Failed to reconcile remote extraction {id} ({status}): {body}"),
        });
    }
    let mut rows = response
        .json::<Vec<RemoteExtractionRow>>()
        .await
        .map_err(|error| DaemonError::Other {
            message: format!("Failed to parse extraction reconciliation response: {error}"),
        })?;
    Ok(rows.pop())
}

pub(crate) struct RemoteNoteCreator {
    db: PowerSyncDatabase,
    auth: Arc<GoTrueClient>,
    http: reqwest::Client,
    config: Arc<Config>,
}

impl RemoteNoteCreator {
    pub(crate) fn new(
        db: PowerSyncDatabase,
        auth: Arc<GoTrueClient>,
        http: reqwest::Client,
        config: Arc<Config>,
    ) -> Self {
        Self {
            db,
            auth,
            http,
            config,
        }
    }
}

fn remote_create_service_error(
    error: DaemonError,
) -> flicknote_core::services::error::ServiceError {
    match error {
        DaemonError::PartialCreate {
            message,
            note_id,
            short_id,
            confirmed_extraction_ids,
            pending_extraction_ids,
        } => flicknote_core::services::error::ServiceError::Remote {
            code: "note_create_partial".to_string(),
            message,
            retryable: false,
            details: Some(serde_json::json!({
                "created": true,
                "note_id": note_id,
                "short_id": short_id,
                "confirmed_extraction_ids": confirmed_extraction_ids,
                "pending_extraction_ids": pending_extraction_ids,
            })),
        },
        DaemonError::AmbiguousCreate {
            message,
            note_id,
            pending_extraction_ids,
        } => flicknote_core::services::error::ServiceError::Remote {
            code: "note_create_unknown".to_string(),
            message,
            retryable: false,
            details: Some(serde_json::json!({
                "created": serde_json::Value::Null,
                "note_id": note_id,
                "short_id": serde_json::Value::Null,
                "pending_extraction_ids": pending_extraction_ids,
            })),
        },
        error => flicknote_core::services::error::ServiceError::Daemon(error.to_string()),
    }
}

#[async_trait]
impl NoteCreator for RemoteNoteCreator {
    async fn create(
        &self,
        request: CreateNote,
    ) -> Result<
        flicknote_core::services::ports::CreatedNote,
        flicknote_core::services::error::ServiceError,
    > {
        create_note_remotely(&self.db, &self.http, &self.auth, &self.config, request)
            .await
            .map_err(remote_create_service_error)
    }
}
