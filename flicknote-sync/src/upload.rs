use std::sync::Arc;

use flicknote_auth::client::GoTrueClient;
use futures_lite::StreamExt;
use powersync::{CrudEntry, PowerSyncDatabase, UpdateType, error::PowerSyncError};

#[cfg(test)]
mod tests;

/// Helper to convert arbitrary errors into PowerSyncError.
pub(crate) fn ps_err(msg: impl std::fmt::Display) -> PowerSyncError {
    PowerSyncError::upload_error(std::io::Error::other(msg.to_string()))
}

/// Postgres/PostgREST error codes that will never succeed on retry.
/// Mirrors the iOS PostgresFatalCodes pattern (PowerSyncService.swift).
const FATAL_PG_PREFIXES: &[&str] = &[
    "22", // Class 22 — Data Exception
    "23", // Class 23 — Integrity Constraint Violation (FK, unique, not-null)
];

const FATAL_PG_CODES: &[&str] = &[
    "42501",    // INSUFFICIENT PRIVILEGE (RLS violation)
    "42703",    // undefined column
    "42P01",    // undefined table
    "PGRST203", // PostgREST: table not found
    "PGRST204", // PostgREST: column not found
];

/// Check if a Supabase/PostgREST error body contains a non-transient PG error.
/// Returns `Some(code)` if the error is fatal (will never succeed on retry),
/// or `None` if the code is unrecognised, missing, or the body is not JSON.
/// `None` does not mean the error is confirmed transient — it means unknown.
fn extract_fatal_code(body: &str) -> Option<String> {
    let parsed: serde_json::Value = serde_json::from_str(body).ok().or_else(|| {
        log::debug!("extract_fatal_code: body is not JSON, treating as unknown: {body}");
        None
    })?;
    let code = parsed.get("code").and_then(|v| v.as_str()).or_else(|| {
        log::debug!("extract_fatal_code: no `code` field in body, treating as unknown");
        None
    })?;

    for prefix in FATAL_PG_PREFIXES {
        if code.starts_with(prefix) {
            return Some(code.to_string());
        }
    }
    if FATAL_PG_CODES.contains(&code) {
        return Some(code.to_string());
    }
    None
}

/// Classify an HTTP response as success, fatal (discard), or transient (retry).
enum UploadOutcome {
    Success,
    Fatal(String),
    Transient(String),
}

async fn classify_response(
    resp: reqwest::Response,
    op: &str,
    table: &str,
    id: &str,
) -> UploadOutcome {
    let status = resp.status();
    if status.is_success() {
        return UploadOutcome::Success;
    }
    let body = resp
        .text()
        .await
        .unwrap_or_else(|e| format!("<body read error: {e}>"));
    if let Some(code) = extract_fatal_code(&body) {
        UploadOutcome::Fatal(format!(
            "HTTP {status} PG {code}: {op} {table}/{id} — {body}"
        ))
    } else {
        UploadOutcome::Transient(format!("HTTP {status}: {op} {table}/{id} failed: {body}"))
    }
}

pub(crate) struct FlickNoteConnector {
    pub(crate) db: PowerSyncDatabase,
    pub(crate) auth: Arc<GoTrueClient>,
    pub(crate) http_client: reqwest::Client,
    pub(crate) powersync_url: String,
    pub(crate) supabase_url: String,
    pub(crate) supabase_anon_key: String,
}

/// Un-wrap JSON strings that contain objects/arrays (fixes double-marshal for jsonb columns).
/// PowerSync stores jsonb as text, so crud.data has them as Value::String.
/// Supabase expects Value::Object for jsonb columns.
fn unwrap_json_strings(data: &mut serde_json::Map<String, serde_json::Value>) {
    for (key, value) in data.iter_mut() {
        if let serde_json::Value::String(s) = value {
            match serde_json::from_str::<serde_json::Value>(s) {
                Ok(parsed) if parsed.is_object() || parsed.is_array() => {
                    *value = parsed;
                }
                Err(e) if s.starts_with('{') || s.starts_with('[') => {
                    log::debug!(
                        "unwrap_json_strings: field `{key}` looks like JSON but failed to parse: {e}"
                    );
                }
                _ => {}
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FlickNoteCrudMarker {
    RemoteCommittedInsert,
}

fn parse_flicknote_crud_marker(
    metadata: Option<&str>,
) -> Result<Option<FlickNoteCrudMarker>, PowerSyncError> {
    let Some(metadata) = metadata else {
        return Ok(None);
    };
    let value: serde_json::Value = serde_json::from_str(metadata)
        .map_err(|error| ps_err(format!("invalid CRUD metadata: {error}")))?;
    let Some(object) = value.as_object() else {
        return Ok(None);
    };
    let Some(marker) = object.get("flicknote") else {
        return Ok(None);
    };
    if object.len() != 1 {
        return Err(ps_err(
            "invalid FlickNote CRUD metadata: expected exactly one marker field",
        ));
    }
    match marker.as_str() {
        Some("remote_committed_insert_v1") => Ok(Some(FlickNoteCrudMarker::RemoteCommittedInsert)),
        _ => Err(ps_err(format!(
            "unsupported FlickNote CRUD marker: {marker}"
        ))),
    }
}

fn prepare_crud(crud: &mut CrudEntry) -> Result<bool, PowerSyncError> {
    if crud.table == "keyterms" {
        log::info!(
            "Discarding queued CRUD for retired keyterms row {}",
            crud.id
        );
        return Ok(true);
    }
    if crud.table == "projects"
        && let Some(data) = crud.data.as_mut()
    {
        data.remove("keyterm_id");
    }
    if parse_flicknote_crud_marker(crud.metadata.as_deref())?
        != Some(FlickNoteCrudMarker::RemoteCommittedInsert)
    {
        return Ok(false);
    }

    let operation = match &crud.update_type {
        UpdateType::Put => "PUT",
        UpdateType::Patch => "PATCH",
        UpdateType::Delete => "DELETE",
    };
    let allowed_table = matches!(crud.table.as_str(), "notes" | "note_extractions");
    if !allowed_table || !matches!(&crud.update_type, UpdateType::Put) {
        return Err(ps_err(format!(
            "invalid remote-committed marker on {operation} operation for table {}",
            crud.table,
        )));
    }
    Ok(true)
}

async fn upload_crud(
    client: &reqwest::Client,
    token: &str,
    supabase_url: &str,
    supabase_anon_key: &str,
    crud: CrudEntry,
) -> Result<UploadOutcome, PowerSyncError> {
    let table = crud.table;
    let id = crud.id;
    let (operation, response) = match crud.update_type {
        UpdateType::Put => {
            let mut data = crud.data.unwrap_or_default();
            data.insert("id".into(), serde_json::Value::String(id.clone()));
            unwrap_json_strings(&mut data);
            let response = client
                .post(format!("{supabase_url}/rest/v1/{table}"))
                .header("apikey", supabase_anon_key)
                .header("Authorization", format!("Bearer {token}"))
                .header("Prefer", "resolution=merge-duplicates")
                .json(&data)
                .send()
                .await
                .map_err(|error| ps_err(format!("Upload PUT failed: {error}")))?;
            ("PUT", response)
        }
        UpdateType::Patch => {
            let mut data = crud.data.unwrap_or_default();
            unwrap_json_strings(&mut data);
            let response = client
                .patch(format!("{supabase_url}/rest/v1/{table}?id=eq.{id}"))
                .header("apikey", supabase_anon_key)
                .header("Authorization", format!("Bearer {token}"))
                .json(&data)
                .send()
                .await
                .map_err(|error| ps_err(format!("Upload PATCH failed: {error}")))?;
            ("PATCH", response)
        }
        UpdateType::Delete => {
            let response = client
                .delete(format!("{supabase_url}/rest/v1/{table}?id=eq.{id}"))
                .header("apikey", supabase_anon_key)
                .header("Authorization", format!("Bearer {token}"))
                .send()
                .await
                .map_err(|error| ps_err(format!("Upload DELETE failed: {error}")))?;
            ("DELETE", response)
        }
    };
    Ok(classify_response(response, operation, &table, &id).await)
}

/// The token is fetched once per call by the caller. Supabase tokens are typically
/// valid for 1 hour, so any realistic upload batch completes well within the window.
pub(crate) async fn run_upload(
    db: &PowerSyncDatabase,
    client: &reqwest::Client,
    token: &str,
    supabase_url: &str,
    supabase_anon_key: &str,
) -> Result<(), PowerSyncError> {
    let mut transactions = db.crud_transactions();

    while let Some(mut tx) = transactions.try_next().await? {
        let mut fatal_msg: Option<String> = None;
        let mut transient_msg: Option<String> = None;

        for mut crud in std::mem::take(&mut tx.crud) {
            if prepare_crud(&mut crud)? {
                continue;
            }
            match upload_crud(client, token, supabase_url, supabase_anon_key, crud).await? {
                UploadOutcome::Success => {}
                UploadOutcome::Fatal(msg) => {
                    fatal_msg = Some(msg);
                    break; // stop processing this transaction's entries
                }
                UploadOutcome::Transient(msg) => {
                    transient_msg = Some(msg);
                    break; // stop processing, will retry
                }
            }
        }

        // Handle outcome AFTER the for loop (tx is not moved inside the loop)
        if let Some(msg) = fatal_msg {
            log::error!("Non-transient error, discarding transaction: {msg}");
            tx.complete().await.map_err(|e| {
                ps_err(format!(
                    "Failed to discard fatal transaction (original: {msg}): {e}"
                ))
            })?; // discard entire transaction atomically
            continue; // next transaction
        }
        if let Some(msg) = transient_msg {
            return Err(ps_err(msg)); // retry on next cycle
        }

        // All entries succeeded — complete each transaction individually so
        // successfully-uploaded entries are removed from ps_crud before processing
        // the next batch. Without this, a mid-batch failure would re-upload all
        // prior entries on the next cycle, causing phantom DELETEs (404) and
        // duplicate PUTs.
        tx.complete().await?;
    }

    Ok(())
}
