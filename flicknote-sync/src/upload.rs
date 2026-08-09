use crate::*;

#[cfg(test)]
mod tests;

/// Helper to convert arbitrary errors into PowerSyncError.
pub(crate) fn ps_err(msg: impl std::fmt::Display) -> PowerSyncError {
    std::io::Error::other(msg.to_string()).into()
}

/// Postgres/PostgREST error codes that will never succeed on retry.
/// Mirrors the iOS PostgresFatalCodes pattern (PowerSyncService.swift).
pub(crate) const FATAL_PG_PREFIXES: &[&str] = &[
    "22", // Class 22 — Data Exception
    "23", // Class 23 — Integrity Constraint Violation (FK, unique, not-null)
];

pub(crate) const FATAL_PG_CODES: &[&str] = &[
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
pub(crate) fn extract_fatal_code(body: &str) -> Option<String> {
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
pub(crate) enum UploadOutcome {
    Success,
    Fatal(String),
    Transient(String),
}

pub(crate) async fn classify_response(
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
    pub(crate) upload_guard: Arc<tokio::sync::Mutex<()>>,
    pub(crate) http_client: reqwest::Client,
    pub(crate) powersync_url: String,
    pub(crate) supabase_url: String,
    pub(crate) supabase_anon_key: String,
}

/// Un-wrap JSON strings that contain objects/arrays (fixes double-marshal for jsonb columns).
/// PowerSync stores jsonb as text, so crud.data has them as Value::String.
/// Supabase expects Value::Object for jsonb columns.
pub(crate) fn unwrap_json_strings(data: &mut serde_json::Map<String, serde_json::Value>) {
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
pub(crate) enum FlickNoteCrudMarker {
    RemoteCommittedInsert,
}

pub(crate) fn parse_flicknote_crud_marker(
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

/// Inner upload logic shared by the BackendConnector and application-triggered drain.
/// Caller is responsible for holding `upload_guard` before calling.
///
/// Returns `true` if at least one CRUD transaction was processed and committed,
/// `false` if ps_crud was empty. Callers may use this to decide whether to
/// run a WAL checkpoint after upload.
///
/// The token is fetched once per call by the caller. Supabase tokens are typically
/// valid for 1 hour, so any realistic upload batch completes well within the window.
pub(crate) async fn run_upload(
    db: &PowerSyncDatabase,
    client: &reqwest::Client,
    token: &str,
    supabase_url: &str,
    supabase_anon_key: &str,
) -> Result<bool, PowerSyncError> {
    let mut transactions = db.crud_transactions();
    let mut did_upload = false;

    while let Some(mut tx) = transactions.try_next().await? {
        let mut fatal_msg: Option<String> = None;
        let mut transient_msg: Option<String> = None;

        for mut crud in std::mem::take(&mut tx.crud) {
            // The backend retired the keyterm domain. Old offline databases may still
            // have queued writes for the removed table or the removed project column.
            // Consume those retired fields locally so they cannot block the FIFO or
            // cause an otherwise valid project mutation to be discarded by PostgREST.
            if crud.table == "keyterms" {
                log::info!(
                    "Discarding queued CRUD for retired keyterms row {}",
                    crud.id
                );
                continue;
            }
            if crud.table == "projects"
                && let Some(data) = crud.data.as_mut()
            {
                data.remove("keyterm_id");
            }
            if parse_flicknote_crud_marker(crud.metadata.as_deref())?
                == Some(FlickNoteCrudMarker::RemoteCommittedInsert)
            {
                let allowed_table = matches!(crud.table.as_str(), "notes" | "note_extractions");
                let is_put = matches!(&crud.update_type, UpdateType::Put);
                if !allowed_table || !is_put {
                    let operation = match &crud.update_type {
                        UpdateType::Put => "PUT",
                        UpdateType::Patch => "PATCH",
                        UpdateType::Delete => "DELETE",
                    };
                    return Err(ps_err(format!(
                        "invalid remote-committed marker on {operation} operation for table {}",
                        crud.table,
                    )));
                }
                continue;
            }
            let table = &crud.table;
            let id = &crud.id;

            // Single match on crud.update_type — UpdateType is not Copy,
            // so we derive both op and resp in one match to avoid use-after-move.
            let (op, resp) = match crud.update_type {
                UpdateType::Put => {
                    let mut data = crud.data.unwrap_or_default();
                    data.insert("id".into(), serde_json::Value::String(id.clone()));
                    unwrap_json_strings(&mut data);
                    let r = client
                        .post(format!("{supabase_url}/rest/v1/{table}"))
                        .header("apikey", supabase_anon_key)
                        .header("Authorization", format!("Bearer {token}"))
                        .header("Prefer", "resolution=merge-duplicates")
                        .json(&data)
                        .send()
                        .await
                        .map_err(|e| ps_err(format!("Upload PUT failed: {e}")))?;
                    ("PUT", r)
                }
                UpdateType::Patch => {
                    let mut data = crud.data.unwrap_or_default();
                    unwrap_json_strings(&mut data);
                    let r = client
                        .patch(format!("{supabase_url}/rest/v1/{table}?id=eq.{id}"))
                        .header("apikey", supabase_anon_key)
                        .header("Authorization", format!("Bearer {token}"))
                        .json(&data)
                        .send()
                        .await
                        .map_err(|e| ps_err(format!("Upload PATCH failed: {e}")))?;
                    ("PATCH", r)
                }
                UpdateType::Delete => {
                    // No payload — unwrap_json_strings not needed.
                    let r = client
                        .delete(format!("{supabase_url}/rest/v1/{table}?id=eq.{id}"))
                        .header("apikey", supabase_anon_key)
                        .header("Authorization", format!("Bearer {token}"))
                        .send()
                        .await
                        .map_err(|e| ps_err(format!("Upload DELETE failed: {e}")))?;
                    ("DELETE", r)
                }
            };

            match classify_response(resp, op, table, id).await {
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
            did_upload = true;
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
        did_upload = true;
    }

    Ok(did_upload)
}

/// Acquire the upload guard, get a fresh token, run_upload, and checkpoint.
/// Shared by the startup drain and application-triggered drain.
/// `context` is used as a log prefix (e.g. "Startup upload", "Upload").
///
/// A PASSIVE checkpoint is run after a successful upload to reclaim WAL space
/// freed by crud deletions. PASSIVE never acquires PENDING/EXCLUSIVE locks so it
/// is safe to call alongside active pool connections and the download actor.
///
/// The checkpoint call uses `spawn_blocking` since `checkpoint_wal_standalone`
/// does blocking I/O (rusqlite open).
#[allow(clippy::too_many_arguments)]
pub(crate) async fn try_upload_and_checkpoint(
    db: &PowerSyncDatabase,
    client: &reqwest::Client,
    auth: &GoTrueClient,
    guard: &tokio::sync::Mutex<()>,
    supabase_url: &str,
    supabase_anon_key: &str,
    context: &str,
    db_path: &Path,
) -> bool {
    let _guard = guard.lock().await;

    let token = match auth.get_session().await {
        Ok(s) => s.access_token,
        Err(e) => {
            log::warn!("{context}: auth error: {e}");
            return false;
        }
    };
    match run_upload(db, client, &token, supabase_url, supabase_anon_key).await {
        Ok(_) => {
            // Post-upload PASSIVE checkpoint: reclaim crud deletion frames without
            // acquiring any locks that could contend with active pool connections.
            let post_path = db_path.to_path_buf();
            if let Err(e) = tokio::task::spawn_blocking(move || {
                checkpoint_wal_standalone(&post_path, "post-upload", WalCheckpointMode::Passive)
            })
            .await
            {
                log::error!("Post-upload WAL checkpoint task panicked: {e}");
            }
            true
        }
        Err(e) => {
            log::warn!("{context}: upload failed: {e}");
            false
        }
    }
}

pub(crate) async fn retry_with_backoff<F, Fut>(
    mut attempt: F,
    initial_delay: std::time::Duration,
    maximum_delay: std::time::Duration,
) where
    F: FnMut() -> Fut,
    Fut: Future<Output = bool>,
{
    let mut delay = initial_delay;
    while !attempt().await {
        tokio::time::sleep(delay).await;
        delay = delay.saturating_mul(2).min(maximum_delay);
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn retry_upload_until_success(
    db: &PowerSyncDatabase,
    client: &reqwest::Client,
    auth: &GoTrueClient,
    guard: &tokio::sync::Mutex<()>,
    supabase_url: &str,
    supabase_anon_key: &str,
    context: &str,
    db_path: &Path,
) {
    retry_with_backoff(
        || {
            try_upload_and_checkpoint(
                db,
                client,
                auth,
                guard,
                supabase_url,
                supabase_anon_key,
                context,
                db_path,
            )
        },
        std::time::Duration::from_secs(1),
        std::time::Duration::from_secs(30),
    )
    .await;
}
