use std::fmt;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use flicknote_auth::client::GoTrueClient;
use flicknote_core::{
    REMOTE_COMMITTED_INSERT_METADATA, TOPIC_EXTRACTION_KEY,
    backend::{NoteDb, SqliteBackend},
    config::Config,
    db::Database,
    schema::app_schema,
    services::ports::{CreateNote, NoteCreator, ShareGateway, ShareResource as CoreShareResource},
};
use futures_lite::StreamExt;
use powersync::{
    BackendConnector, ConnectionPool, PowerSyncCredentials, PowerSyncDatabase, SyncOptions,
    UpdateType, env::PowerSyncEnvironment, error::PowerSyncError,
};
use rusqlite::{OptionalExtension, params};
use serde::Deserialize;
use tokio::{net::UnixListener, sync::mpsc};

pub mod app;
pub mod ipc;
use app::Application;
use ipc::DaemonError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ShareResource {
    Note,
    Project,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ShareRequest {
    resource: ShareResource,
    id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CreateNoteRequest {
    id: String,
    note_type: String,
    status: String,
    title: Option<String>,
    content: Option<String>,
    metadata: Option<String>,
    project_id: Option<String>,
    now: String,
    topics: Vec<String>,
    attachment_path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RemoteCreatedNote {
    uuid: String,
    short_id: i64,
    confirmed_extraction_ids: Vec<String>,
}

#[derive(Debug, Default, PartialEq, Eq)]
struct ExtractionCreateOutcome {
    confirmed_ids: Vec<String>,
    pending_ids: Vec<String>,
    diagnostic: Option<String>,
    local_commit_error: Option<String>,
}

/// Helper to convert arbitrary errors into PowerSyncError.
fn ps_err(msg: impl std::fmt::Display) -> PowerSyncError {
    std::io::Error::other(msg.to_string()).into()
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

struct FlickNoteConnector {
    db: PowerSyncDatabase,
    auth: Arc<GoTrueClient>,
    upload_guard: Arc<tokio::sync::Mutex<()>>,
    http_client: reqwest::Client,
    powersync_url: String,
    supabase_url: String,
    supabase_anon_key: String,
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

/// Inner upload logic shared by the BackendConnector and application-triggered drain.
/// Caller is responsible for holding `upload_guard` before calling.
///
/// Returns `true` if at least one CRUD transaction was processed and committed,
/// `false` if ps_crud was empty. Callers may use this to decide whether to
/// run a WAL checkpoint after upload.
///
/// The token is fetched once per call by the caller. Supabase tokens are typically
/// valid for 1 hour, so any realistic upload batch completes well within the window.
async fn run_upload(
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

        for crud in std::mem::take(&mut tx.crud) {
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

/// WAL checkpoint mode passed to [`checkpoint_wal_standalone`].
#[derive(Clone, Copy)]
enum WalCheckpointMode {
    /// Checkpoints frames up to the oldest active reader's mark. Never acquires
    /// PENDING or EXCLUSIVE locks — returns immediately. Safe at any time alongside
    /// active pool connections. Returns `busy=1` when readers constrain the
    /// checkpoint to an earlier WAL position (normal during runtime).
    Passive,
    /// Acquires a PENDING lock while waiting for readers to finish, then resets
    /// the WAL to zero length. Use only when no pool connections exist (startup,
    /// shutdown) to avoid the PENDING lock blocking pool writers.
    Truncate,
}

impl fmt::Display for WalCheckpointMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Passive => write!(f, "PASSIVE"),
            Self::Truncate => write!(f, "TRUNCATE"),
        }
    }
}

/// Run a WAL checkpoint using a standalone rusqlite connection.
///
/// Opens its own connection to the DB file, bypassing PowerSync's writer mutex
/// entirely — competes only at the SQLite file-lock level, not the Rust mutex level.
///
/// `mode` controls the checkpoint type — see [`WalCheckpointMode`] for semantics.
///
/// `busy_timeout` is set to 5 000 ms for TRUNCATE so it retries at the SQLite level
/// while pool readers finish their short transactions. It is irrelevant for PASSIVE
/// (which never waits) but harmless to keep set.
///
/// Reads the `(busy, log, checkpointed)` return tuple from PRAGMA so failures
/// are never silently swallowed. For PASSIVE, `busy=1` when active readers
/// constrain the checkpoint to an earlier WAL position (normal and expected during
/// runtime). For TRUNCATE, `busy=1` means the reset could not complete.
///
/// This function is **synchronous** (blocking rusqlite I/O). Async callers must
/// wrap it with `tokio::task::spawn_blocking`.
///
/// `label` identifies the call site in log output (e.g. `"startup"`, `"post-upload"`,
/// `"periodic"`, `"shutdown"`) so production logs are unambiguous.
fn checkpoint_wal_standalone(db_path: &Path, label: &str, mode: WalCheckpointMode) {
    let conn = match rusqlite::Connection::open(db_path) {
        Ok(c) => c,
        Err(e) => {
            log::warn!("WAL checkpoint [{label}]: could not open db: {e}");
            return;
        }
    };
    if let Err(e) = conn.pragma_update(None, "busy_timeout", 5_000i64) {
        log::warn!("WAL checkpoint [{label}]: could not set busy_timeout: {e}");
        return;
    }
    let pragma = format!("PRAGMA wal_checkpoint({})", mode);
    match conn.query_row(&pragma, [], |row| {
        Ok((
            row.get::<_, i32>(0)?,
            row.get::<_, i32>(1)?,
            row.get::<_, i32>(2)?,
        ))
    }) {
        Ok((busy, log, checkpointed)) => {
            if busy == 0 {
                log::info!(
                    "WAL checkpoint [{label}] ({mode}): {log} pages, {checkpointed} checkpointed"
                );
            } else {
                log::warn!(
                    "WAL checkpoint [{label}]: incomplete (busy={busy}, {log} log pages, {checkpointed} checkpointed)"
                );
            }
        }
        Err(e) => log::warn!("WAL checkpoint [{label}]: failed: {e}"),
    }
    // Connection dropped here — no persistent state
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
async fn try_upload_and_checkpoint(
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

async fn retry_with_backoff<F, Fut>(
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
async fn retry_upload_until_success(
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

#[async_trait]
impl BackendConnector for FlickNoteConnector {
    async fn fetch_credentials(&self) -> Result<PowerSyncCredentials, PowerSyncError> {
        let session = self
            .auth
            .get_session()
            .await
            .map_err(|e| ps_err(format!("Auth error: {e}")))?;

        Ok(PowerSyncCredentials {
            endpoint: self.powersync_url.clone(),
            token: session.access_token,
        })
    }

    async fn upload_data(&self) -> Result<(), PowerSyncError> {
        let _guard = self.upload_guard.lock().await;
        let token = self.get_token().await?;
        // Ignore the bool — checkpoint is only safe to call from the serialized drain path,
        // not here (SDK callback fires during active sync alongside the download actor).
        run_upload(
            &self.db,
            &self.http_client,
            &token,
            &self.supabase_url,
            &self.supabase_anon_key,
        )
        .await?;
        Ok(())
    }
}

impl FlickNoteConnector {
    async fn get_token(&self) -> Result<String, PowerSyncError> {
        let session = self
            .auth
            .get_session()
            .await
            .map_err(|e| ps_err(format!("Auth error: {e}")))?;
        Ok(session.access_token)
    }
}

fn pid_path(config: &Config) -> PathBuf {
    PathBuf::from(&config.paths.data_dir).join("sync.pid")
}

struct PidGuard(PathBuf);

impl Drop for PidGuard {
    fn drop(&mut self) {
        if let Err(e) = std::fs::remove_file(&self.0) {
            log::warn!("Failed to remove PID file: {}", e);
        }
    }
}

struct SocketGuard(PathBuf);

impl Drop for SocketGuard {
    fn drop(&mut self) {
        if let Err(e) = std::fs::remove_file(&self.0) {
            log::warn!("Failed to remove socket file: {}", e);
        }
    }
}

fn bind_socket(config: &Config) -> Result<(UnixListener, SocketGuard), Box<dyn std::error::Error>> {
    let path = ipc::socket_path(config);
    if path.exists() {
        std::fs::remove_file(&path)?;
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let listener = UnixListener::bind(&path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok((listener, SocketGuard(path)))
}

/// Check for an existing sync daemon and write our PID file.
///
/// Note: there is a small TOCTOU window between the `kill(pid, 0)` liveness
/// check and writing the new PID file. Two daemons launched simultaneously
/// could both pass. For a CLI daemon this is acceptable; use `flock` or
/// `O_CREAT|O_EXCL` if stronger guarantees are ever needed.
#[allow(unsafe_code)]
fn check_and_write_pid(path: &Path) -> Result<PidGuard, Box<dyn std::error::Error>> {
    if let Ok(contents) = std::fs::read_to_string(path)
        && let Ok(pid) = contents.trim().parse::<i32>()
    {
        let result = unsafe { libc::kill(pid, 0) };
        if result == 0
            || (result == -1 && std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM))
        {
            return Err(format!(
                "Sync daemon already running (pid={}). Kill it first or delete {}",
                pid,
                path.display()
            )
            .into());
        }
        log::info!("Removing stale PID file (pid={} no longer running)", pid);
    }

    std::fs::write(path, std::process::id().to_string())
        .map_err(|e| format!("Failed to write PID file {}: {}", path.display(), e))?;
    Ok(PidGuard(path.to_path_buf()))
}

/// Tear down all async actors, disconnect the database, and run a final TRUNCATE
/// checkpoint.
///
/// Called from every shutdown path (ctrl-c, task panic, normal exit). The pool
/// is fully gone after `db.disconnect().await`, so TRUNCATE succeeds without
/// contention. Uses `spawn_blocking` to keep the blocking rusqlite I/O off the
/// async executor thread per [`checkpoint_wal_standalone`]'s contract.
async fn shutdown_daemon(
    upload_handle: &mut tokio::task::JoinHandle<()>,
    checkpoint_handle: &mut tokio::task::JoinHandle<()>,
    socket_handle: &mut tokio::task::JoinHandle<()>,
    db: &PowerSyncDatabase,
    db_path: PathBuf,
) {
    upload_handle.abort();
    checkpoint_handle.abort();
    socket_handle.abort();
    db.disconnect().await;
    if let Err(e) = tokio::task::spawn_blocking(move || {
        checkpoint_wal_standalone(&db_path, "shutdown", WalCheckpointMode::Truncate)
    })
    .await
    {
        log::error!("Shutdown WAL checkpoint task panicked: {e}");
    }
    log::info!("Sync daemon stopped");
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

fn attachment_endpoint(base_url: &str, path: &str) -> String {
    let versioned_base = base_url
        .trim_end_matches('/')
        .trim_end_matches("/api/v1")
        .trim_end_matches('/');
    let path = path.trim_matches('/');
    format!("{versioned_base}/api/v1/attachments/{path}")
}

#[derive(Deserialize)]
struct ShareResponse {
    url: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ShareApiError {
    error_code: Option<String>,
    message: Option<String>,
}

#[derive(Default)]
struct ShareRequestLock {
    mutex: tokio::sync::Mutex<()>,
}

impl ShareRequestLock {
    async fn run<T>(&self, operation: impl Future<Output = T>) -> T {
        let _guard = self.mutex.lock().await;
        operation.await
    }
}

impl ShareResource {
    fn path_segment(self) -> &'static str {
        match self {
            Self::Note => "notes",
            Self::Project => "projects",
        }
    }

    fn missing_error_code(self) -> &'static str {
        match self {
            Self::Note => "SHARE_NOT_FOUND",
            Self::Project => "PROJECT_SHARE_NOT_FOUND",
        }
    }
}

fn share_endpoint(api_url: &str, request: &ShareRequest) -> String {
    let versioned_base = api_url
        .trim_end_matches('/')
        .trim_end_matches("/api/v1")
        .trim_end_matches('/');
    format!(
        "{versioned_base}/api/v1/{}/{}/share",
        request.resource.path_segment(),
        request.id
    )
}

fn share_api_error(status: reqwest::StatusCode, body: String) -> DaemonError {
    let message = serde_json::from_str::<ShareApiError>(&body)
        .ok()
        .and_then(|error| error.message)
        .unwrap_or(body);
    DaemonError::Other {
        message: format!("Share API returned {status}: {message}"),
    }
}

async fn parse_share_url(response: reqwest::Response) -> Result<String, DaemonError> {
    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(share_api_error(status, body));
    }
    response
        .json::<ShareResponse>()
        .await
        .map(|share| share.url)
        .map_err(|error| DaemonError::Other {
            message: format!("Failed to parse share API response: {error}"),
        })
}

async fn get_or_create_share_with_token(
    http: &reqwest::Client,
    config: &Config,
    access_token: &str,
    request: &ShareRequest,
) -> Result<String, DaemonError> {
    validate_api_url(config)?;
    let endpoint = share_endpoint(&config.api_url, request);
    let response = http
        .get(&endpoint)
        .bearer_auth(access_token)
        .send()
        .await
        .map_err(|error| DaemonError::Other {
            message: format!("Share request failed: {error}"),
        })?;

    if response.status().is_success() {
        return parse_share_url(response).await;
    }

    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    let is_missing_share = status == reqwest::StatusCode::NOT_FOUND
        && serde_json::from_str::<ShareApiError>(&body)
            .ok()
            .and_then(|error| error.error_code)
            .is_some_and(|code| code == request.resource.missing_error_code());
    if !is_missing_share {
        return Err(share_api_error(status, body));
    }

    let response = http
        .post(endpoint)
        .bearer_auth(access_token)
        .json(&serde_json::json!({}))
        .send()
        .await
        .map_err(|error| DaemonError::Other {
            message: format!("Share create request failed: {error}"),
        })?;
    parse_share_url(response).await
}

async fn revoke_share_with_token(
    http: &reqwest::Client,
    config: &Config,
    access_token: &str,
    request: &ShareRequest,
) -> Result<(), DaemonError> {
    validate_api_url(config)?;
    let response = http
        .delete(share_endpoint(&config.api_url, request))
        .bearer_auth(access_token)
        .send()
        .await
        .map_err(|error| DaemonError::Other {
            message: format!("Share revoke request failed: {error}"),
        })?;
    let status = response.status();
    if status.is_success() {
        return Ok(());
    }
    let body = response.text().await.unwrap_or_default();
    Err(share_api_error(status, body))
}

async fn get_or_create_share(
    http: &reqwest::Client,
    auth: &GoTrueClient,
    config: &Config,
    request: &ShareRequest,
) -> Result<String, DaemonError> {
    let session = auth
        .get_session()
        .await
        .map_err(|error| DaemonError::Other {
            message: format!("Auth error: {error}"),
        })?;
    get_or_create_share_with_token(http, config, &session.access_token, request).await
}

async fn revoke_share(
    http: &reqwest::Client,
    auth: &GoTrueClient,
    config: &Config,
    request: &ShareRequest,
) -> Result<(), DaemonError> {
    let session = auth
        .get_session()
        .await
        .map_err(|error| DaemonError::Other {
            message: format!("Auth error: {error}"),
        })?;
    revoke_share_with_token(http, config, &session.access_token, request).await
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UploadUrlResponse {
    upload_url: String,
    content_type: String,
}

fn validate_api_url(config: &Config) -> Result<(), DaemonError> {
    if config.api_url.is_empty() {
        return Err(DaemonError::Other {
            message: "apiUrl is not configured — set it in config.json or FLICKNOTE_API_URL"
                .to_string(),
        });
    }
    Ok(())
}

async fn upload_attachment(
    http: &reqwest::Client,
    config: &Config,
    access_token: &str,
    note_id: &str,
    file_path: &Path,
) -> Result<(), DaemonError> {
    validate_api_url(config)?;
    let filename = file_path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| DaemonError::Other {
            message: "Invalid filename".to_string(),
        })?
        .to_string();

    let resp = http
        .post(attachment_endpoint(&config.api_url, "upload-url"))
        .bearer_auth(access_token)
        .json(&serde_json::json!({ "noteId": note_id, "filename": filename }))
        .send()
        .await
        .map_err(|e| DaemonError::Other {
            message: format!("Upload URL request failed: {e}"),
        })?;

    if !resp.status().is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(DaemonError::Other {
            message: format!("Upload URL request failed: {body}"),
        });
    }

    let upload_resp: UploadUrlResponse = resp.json().await.map_err(|e| DaemonError::Other {
        message: format!("Failed to parse upload URL response: {e}"),
    })?;

    let file_bytes = std::fs::read(file_path).map_err(|e| DaemonError::Other {
        message: format!("Failed to read {}: {e}", file_path.display()),
    })?;
    let put_resp = http
        .put(&upload_resp.upload_url)
        .header("Content-Type", &upload_resp.content_type)
        .body(file_bytes)
        .send()
        .await
        .map_err(|e| DaemonError::Other {
            message: format!("File upload failed: {e}"),
        })?;

    if !put_resp.status().is_success() {
        let body = put_resp.text().await.unwrap_or_default();
        return Err(DaemonError::Other {
            message: format!("File upload to R2 failed: {body}"),
        });
    }

    Ok(())
}

async fn delete_attachment(
    http: &reqwest::Client,
    config: &Config,
    access_token: &str,
    note_id: &str,
) -> Result<(), DaemonError> {
    validate_api_url(config)?;
    let resp = http
        .delete(attachment_endpoint(&config.api_url, note_id))
        .bearer_auth(access_token)
        .send()
        .await
        .map_err(|e| DaemonError::Other {
            message: format!("Delete request failed: {e}"),
        })?;

    if resp.status().is_success() {
        return Ok(());
    }

    let body = resp.text().await.unwrap_or_default();
    Err(DaemonError::Other {
        message: format!("Delete failed: {body}"),
    })
}

async fn create_note_remotely(
    db: &PowerSyncDatabase,
    http: &reqwest::Client,
    auth: &GoTrueClient,
    config: &Config,
    req: CreateNoteRequest,
) -> Result<RemoteCreatedNote, DaemonError> {
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

async fn create_note_with_token(
    db: &PowerSyncDatabase,
    http: &reqwest::Client,
    config: &Config,
    access_token: &str,
    user_id: &str,
    req: CreateNoteRequest,
) -> Result<RemoteCreatedNote, DaemonError> {
    let extraction_rows = req
        .topics
        .iter()
        .map(|value| RemoteExtractionRow {
            id: uuid::Uuid::new_v4().to_string(),
            note_id: req.id.clone(),
            user_id: user_id.to_string(),
            key: TOPIC_EXTRACTION_KEY.to_string(),
            value: value.clone(),
        })
        .collect::<Vec<_>>();
    let metadata = match req.metadata.as_deref() {
        Some(raw) => {
            serde_json::from_str::<serde_json::Value>(raw).map_err(|e| DaemonError::Other {
                message: format!("Invalid note metadata JSON: {e}"),
            })?
        }
        None => serde_json::Value::Null,
    };

    let attachment_path = req.attachment_path.as_deref().map(Path::new);
    if let Some(path) = attachment_path {
        upload_attachment(http, config, access_token, &req.id, path).await?;
    }

    let payload = serde_json::json!({
        "id": req.id,
        "user_id": user_id,
        "type": req.note_type,
        "status": req.status,
        "title": req.title,
        "content": req.content,
        "metadata": metadata,
        "project_id": req.project_id,
        "created_at": req.now,
        "updated_at": req.now,
    });

    let send_create = || {
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
        .json(&payload)
        .send()
    };
    let (resp, initial_ambiguous_error) = match send_create().await {
        Ok(resp) if !is_ambiguous_create_status(resp.status()) => (resp, None),
        Ok(resp) => {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            let initial_error = format!("the first attempt returned {status}: {body}");
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            match send_create().await {
                Ok(resp) if !is_ambiguous_create_status(resp.status()) => {
                    (resp, Some(initial_error))
                }
                Ok(resp) => {
                    let status = resp.status();
                    let body = resp.text().await.unwrap_or_default();
                    if let Ok(Some(row)) =
                        lookup_remote_note(http, config, access_token, &req.id).await
                    {
                        return finish_remote_create(
                            db,
                            http,
                            config,
                            access_token,
                            row,
                            &extraction_rows,
                        )
                        .await;
                    }
                    return Err(ambiguous_create_error(
                        format!(
                            "Remote note create outcome is unknown for note {} after retrying the same stable UUID ({initial_error}; retry returned {status}: {body}). The attachment was retained. Do not create it again.",
                            req.id
                        ),
                        req.id,
                        &extraction_rows,
                    ));
                }
                Err(retry_error) => {
                    if let Ok(Some(row)) =
                        lookup_remote_note(http, config, access_token, &req.id).await
                    {
                        return finish_remote_create(
                            db,
                            http,
                            config,
                            access_token,
                            row,
                            &extraction_rows,
                        )
                        .await;
                    }
                    return Err(ambiguous_create_error(
                        format!(
                            "Remote note create outcome is unknown for note {} after retrying the same stable UUID ({initial_error}; retry: {retry_error}). The attachment was retained. Do not create it again.",
                            req.id
                        ),
                        req.id,
                        &extraction_rows,
                    ));
                }
            }
        }
        Err(initial_error) => {
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            match send_create().await {
                Ok(resp) => (resp, Some(initial_error.to_string())),
                Err(retry_error) => {
                    if let Ok(Some(row)) =
                        lookup_remote_note(http, config, access_token, &req.id).await
                    {
                        return finish_remote_create(
                            db,
                            http,
                            config,
                            access_token,
                            row,
                            &extraction_rows,
                        )
                        .await;
                    }
                    return Err(ambiguous_create_error(
                        format!(
                            "Remote note create outcome is unknown for note {} after retrying the same stable UUID ({initial_error}; retry: {retry_error}). The attachment was retained. Do not create it again.",
                            req.id
                        ),
                        req.id,
                        &extraction_rows,
                    ));
                }
            }
        }
    };

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        if let Ok(Some(row)) = lookup_remote_note(http, config, access_token, &req.id).await {
            return finish_remote_create(db, http, config, access_token, row, &extraction_rows)
                .await;
        }
        if let Some(initial_error) = initial_ambiguous_error {
            return Err(ambiguous_create_error(
                format!(
                    "Remote note create outcome is unknown for note {} after retrying the same stable UUID: {initial_error}; the retry returned {status}: {body}. The attachment was retained. Do not create it again.",
                    req.id
                ),
                req.id,
                &extraction_rows,
            ));
        }
        if attachment_path.is_some()
            && let Err(e) = delete_attachment(http, config, access_token, &req.id).await
        {
            log::warn!("Failed to clean up uploaded attachment after note create failure: {e}");
        }
        return Err(DaemonError::Other {
            message: format!("Remote note create failed ({status}): {body}"),
        });
    }

    let row = match resp.json::<Vec<RemoteNoteRow>>().await {
        Ok(mut rows) => match rows.pop() {
            Some(row) => row,
            None => {
                reconcile_confirmed_remote_note(
                    http,
                    config,
                    access_token,
                    &req.id,
                    &extraction_rows,
                    format!("Remote note create returned no row for note {}", req.id),
                )
                .await?
            }
        },
        Err(error) => {
            reconcile_confirmed_remote_note(
                http,
                config,
                access_token,
                &req.id,
                &extraction_rows,
                format!("Failed to parse remote note create response: {error}"),
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
) -> Result<RemoteCreatedNote, DaemonError> {
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
    Ok(RemoteCreatedNote {
        uuid: row.id,
        short_id,
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

struct RemoteNoteCreator {
    db: PowerSyncDatabase,
    auth: Arc<GoTrueClient>,
    http: reqwest::Client,
    config: Arc<Config>,
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
        let created = create_note_remotely(
            &self.db,
            &self.http,
            &self.auth,
            &self.config,
            CreateNoteRequest {
                id: request.id,
                note_type: request.note_type,
                status: request.status,
                title: request.title,
                content: request.content,
                metadata: request.metadata,
                project_id: request.project_id,
                now: request.now,
                topics: request.topics,
                attachment_path: request.attachment_path,
            },
        )
        .await
        .map_err(remote_create_service_error)?;
        Ok(flicknote_core::services::ports::CreatedNote {
            inserted: flicknote_core::backend::InsertedNote {
                uuid: created.uuid,
                short_id: Some(created.short_id),
            },
            confirmed_extraction_ids: created.confirmed_extraction_ids,
        })
    }
}

struct RemoteShareGateway {
    http: reqwest::Client,
    auth: Arc<GoTrueClient>,
    config: Arc<Config>,
    lock: Arc<ShareRequestLock>,
}

#[async_trait]
impl ShareGateway for RemoteShareGateway {
    async fn share(
        &self,
        resource: CoreShareResource,
        id: &str,
    ) -> Result<String, flicknote_core::services::error::ServiceError> {
        let request = ShareRequest {
            resource: match resource {
                CoreShareResource::Note => ShareResource::Note,
                CoreShareResource::Project => ShareResource::Project,
            },
            id: id.to_string(),
        };
        self.lock
            .run(get_or_create_share(
                &self.http,
                &self.auth,
                &self.config,
                &request,
            ))
            .await
            .map_err(|error| {
                flicknote_core::services::error::ServiceError::Daemon(error.to_string())
            })
    }

    async fn unshare(
        &self,
        resource: CoreShareResource,
        id: &str,
    ) -> Result<(), flicknote_core::services::error::ServiceError> {
        let request = ShareRequest {
            resource: match resource {
                CoreShareResource::Note => ShareResource::Note,
                CoreShareResource::Project => ShareResource::Project,
            },
            id: id.to_string(),
        };
        self.lock
            .run(revoke_share(&self.http, &self.auth, &self.config, &request))
            .await
            .map_err(|error| {
                flicknote_core::services::error::ServiceError::Daemon(error.to_string())
            })
    }
}

pub async fn run() -> Result<(), Box<dyn std::error::Error>> {
    let config = Arc::new(Config::load()?);

    let pid_file = pid_path(&config);
    let _pid_guard = check_and_write_pid(&pid_file)?;
    let (socket_listener, _socket_guard) = bind_socket(&config)?;

    if let Ok(database_url) = std::env::var("DATABASE_URL") {
        return run_managed(socket_listener, database_url).await;
    }

    config.validate()?;

    PowerSyncEnvironment::powersync_auto_extension()?;

    let pool = ConnectionPool::open(&config.paths.db_file)?;
    let env = PowerSyncEnvironment::custom(
        reqwest::Client::new(),
        pool,
        PowerSyncEnvironment::tokio_timer(),
    );

    let db = PowerSyncDatabase::new(env, app_schema());
    db.async_tasks().spawn_with_tokio();

    let auth = Arc::new(GoTrueClient::new(
        &config.supabase_url,
        &config.supabase_anon_key,
        &config.paths.session_file,
    ));

    let upload_guard = Arc::new(tokio::sync::Mutex::new(()));
    let http_client = reqwest::Client::new();
    let upload_client = http_client.clone();

    let connector = FlickNoteConnector {
        db: db.clone(),
        auth: Arc::clone(&auth),
        upload_guard: Arc::clone(&upload_guard),
        http_client,
        powersync_url: config.powersync_url.clone(),
        supabase_url: config.supabase_url.clone(),
        supabase_anon_key: config.supabase_anon_key.clone(),
    };

    // Reclaim leftover WAL from previous sessions BEFORE connecting sync actors.
    // TRUNCATE is safe here because no pool connections exist yet — db.connect()
    // hasn't started the download actor. A bloated WAL inherited from a crashed
    // session is reset to zero so incremental PASSIVE checkpoints start from a
    // clean baseline.
    // spawn_blocking keeps blocking rusqlite I/O off the async executor thread.
    log::info!("Running startup WAL checkpoint");
    let startup_db_path = config.paths.db_file.clone();
    if let Err(e) = tokio::task::spawn_blocking(move || {
        checkpoint_wal_standalone(&startup_db_path, "startup", WalCheckpointMode::Truncate)
    })
    .await
    {
        log::error!("Startup WAL checkpoint task panicked: {e}");
    }

    // Finish schema replacement through the application pool before PowerSync
    // starts its download/upload actors. Replacing tracking views after connect
    // races the actor-held SQLite connections and can fail with SQLITE_BUSY on
    // an existing database.
    let user_id = flicknote_core::session::get_user_id(&config)?;
    let backend: Arc<dyn NoteDb> = Arc::new(SqliteBackend {
        db: Database::open_local(&config).await?,
        user_id,
    });

    log::info!("Sync daemon connecting (pid {})", std::process::id());
    db.connect(SyncOptions::new(connector)).await;
    log::info!("Sync daemon connected (pid {})", std::process::id());

    // Application writes happen in this process. Each may-write request sends a
    // best-effort trigger; the startup drain recovers committed writes whose signal
    // was lost because of a crash or a full channel.
    let (trigger_tx, mut trigger_rx) = mpsc::channel::<()>(16);

    let upload_db = db.clone();
    let upload_supabase_url = config.supabase_url.clone();
    let upload_anon_key = config.supabase_anon_key.clone();
    let upload_guard_clone = Arc::clone(&upload_guard);
    let upload_auth_clone = Arc::clone(&auth);
    let upload_db_path = config.paths.db_file.clone();

    let mut upload_handle = tokio::spawn(async move {
        // Initial upload on startup recovers committed CRUD left by a crash,
        // a lost in-process signal, or a pre-upgrade CLI writer.
        retry_upload_until_success(
            &upload_db,
            &upload_client,
            &upload_auth_clone,
            &upload_guard_clone,
            &upload_supabase_url,
            &upload_anon_key,
            "Startup upload",
            &upload_db_path,
        )
        .await;

        loop {
            // Block until the application host reports a may-write request.
            if trigger_rx.recv().await.is_none() {
                break;
            }

            // Trailing debounce: collapse burst writes (e.g. bulk import) into a
            // single upload attempt. Fire only after 200ms of silence.
            loop {
                tokio::select! {
                    _ = tokio::time::sleep(std::time::Duration::from_millis(200)) => break,
                    v = trigger_rx.recv() => {
                        if v.is_none() { return; } // channel closed
                        // more events arrived — reset the silence window
                    }
                }
            }

            retry_upload_until_success(
                &upload_db,
                &upload_client,
                &upload_auth_clone,
                &upload_guard_clone,
                &upload_supabase_url,
                &upload_anon_key,
                "Upload",
                &upload_db_path,
            )
            .await;
        }
    });

    // Periodic PASSIVE checkpoint every 30s — independent of upload success or
    // download actor state. Makes incremental progress draining the WAL without
    // acquiring PENDING/EXCLUSIVE locks, so it never contends with pool writers.
    let checkpoint_db_path = config.paths.db_file.clone();
    let mut checkpoint_handle = tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(30));
        interval.tick().await; // skip the immediate first tick
        loop {
            interval.tick().await;
            let path = checkpoint_db_path.clone();
            if let Err(e) = tokio::task::spawn_blocking(move || {
                checkpoint_wal_standalone(&path, "periodic", WalCheckpointMode::Passive)
            })
            .await
            {
                log::error!("Periodic WAL checkpoint task panicked: {e}");
            }
        }
    });

    let socket_config = Arc::clone(&config);
    let socket_http = reqwest::Client::new();
    let socket_share_lock = Arc::new(ShareRequestLock::default());
    let creator: Arc<dyn NoteCreator> = Arc::new(RemoteNoteCreator {
        db: db.clone(),
        auth: Arc::clone(&auth),
        http: socket_http.clone(),
        config: Arc::clone(&config),
    });
    let gateway: Arc<dyn ShareGateway> = Arc::new(RemoteShareGateway {
        http: socket_http.clone(),
        auth: Arc::clone(&auth),
        config: Arc::clone(&config),
        lock: socket_share_lock,
    });
    let app = Arc::new(
        Application::new(backend, ipc::BackendMode::Local)
            .with_creator(creator)
            .with_share_gateway(gateway)
            .with_web_url(config.web_url.clone())
            .with_write_signal(trigger_tx),
    );
    let mut socket_handle = tokio::spawn(async move {
        if let Err(error) = ipc::serve_app(socket_listener, app, ipc::ServerInfo::local()).await {
            log::error!("Application socket server failed: {error}");
        }
    });

    tokio::select! {
        _ = tokio::signal::ctrl_c() => {}
        res = &mut upload_handle => {
            if let Err(e) = res {
                log::error!("Upload task panicked: {e}");
                shutdown_daemon(&mut upload_handle, &mut checkpoint_handle, &mut socket_handle, &db, socket_config.paths.db_file.clone()).await;
                return Err(e.into());
            }
        }
        res = &mut checkpoint_handle => {
            match res {
                Ok(_) => log::error!("Checkpoint task exited unexpectedly"),
                Err(ref e) => log::error!("Checkpoint task panicked: {e}"),
            }
            let err_msg = format!("Checkpoint task exited: {res:?}");
            shutdown_daemon(&mut upload_handle, &mut checkpoint_handle, &mut socket_handle, &db, socket_config.paths.db_file.clone()).await;
            return Err(err_msg.into());
        }
        res = &mut socket_handle => {
            match res {
                Ok(_) => log::error!("Socket task exited unexpectedly"),
                Err(ref e) => log::error!("Socket task panicked: {e}"),
            }
            let err_msg = format!("Socket task exited: {res:?}");
            shutdown_daemon(&mut upload_handle, &mut checkpoint_handle, &mut socket_handle, &db, socket_config.paths.db_file.clone()).await;
            return Err(err_msg.into());
        }
    }
    shutdown_daemon(
        &mut upload_handle,
        &mut checkpoint_handle,
        &mut socket_handle,
        &db,
        socket_config.paths.db_file.clone(),
    )
    .await;

    Ok(())
}

async fn run_managed(
    listener: UnixListener,
    database_url: String,
) -> Result<(), Box<dyn std::error::Error>> {
    let backend: Arc<dyn NoteDb> =
        Arc::new(flicknote_core::pgwire::PgWireBackend::connect(&database_url).await?);
    let app = Arc::new(Application::new(backend, ipc::BackendMode::Managed));
    log::info!("Managed daemon ready (pid {})", std::process::id());
    tokio::select! {
        _ = tokio::signal::ctrl_c() => Ok(()),
        result = ipc::serve_app(listener, app, ipc::ServerInfo::managed()) => {
            result.map_err(Into::into)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::thread;

    use flicknote_core::config::ConfigPaths;

    use super::*;

    #[tokio::test]
    async fn failed_upload_is_retried_without_a_second_write_trigger() {
        let attempts = Arc::new(AtomicUsize::new(0));
        let attempt_counter = Arc::clone(&attempts);

        retry_with_backoff(
            move || {
                let attempt_counter = Arc::clone(&attempt_counter);
                async move { attempt_counter.fetch_add(1, Ordering::SeqCst) > 0 }
            },
            std::time::Duration::from_millis(1),
            std::time::Duration::from_millis(2),
        )
        .await;

        assert_eq!(attempts.load(Ordering::SeqCst), 2);
    }

    async fn test_powersync_db() -> (tempfile::TempDir, PowerSyncDatabase) {
        PowerSyncEnvironment::powersync_auto_extension().unwrap();
        let directory = tempfile::tempdir().unwrap();
        let db = test_powersync_db_at(directory.path().join("test.db"), app_schema());
        db.writer().await.unwrap();
        (directory, db)
    }

    fn test_powersync_db_at(
        path: impl AsRef<std::path::Path>,
        schema: powersync::schema::Schema,
    ) -> PowerSyncDatabase {
        PowerSyncEnvironment::powersync_auto_extension().unwrap();
        let pool = ConnectionPool::open(path).unwrap();
        let env = PowerSyncEnvironment::custom(
            reqwest::Client::new(),
            pool,
            PowerSyncEnvironment::tokio_timer(),
        );
        PowerSyncDatabase::new(env, schema)
    }

    async fn insert_note_with_metadata(db: &PowerSyncDatabase, metadata: &str) {
        let writer = db.writer().await.unwrap();
        writer
            .execute(
                r#"INSERT INTO notes (
                    id, short_id, user_id, type, status, title, content,
                    is_flagged, created_at, updated_at, _metadata
                ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"#,
                params![
                    "note-1",
                    42,
                    "user-1",
                    "normal",
                    "ai_queued",
                    "Title",
                    "Body",
                    0,
                    "2026-08-09T00:00:00Z",
                    "2026-08-09T00:00:00Z",
                    metadata,
                ],
            )
            .unwrap();
    }

    async fn insert_marked_note(db: &PowerSyncDatabase) {
        insert_note_with_metadata(db, REMOTE_COMMITTED_INSERT_METADATA).await;
    }

    fn remote_note(id: &str, title: &str) -> RemoteNoteRow {
        RemoteNoteRow {
            id: id.to_string(),
            short_id: Some(42),
            user_id: "user-1".to_string(),
            note_type: "normal".to_string(),
            status: "ai_queued".to_string(),
            title: Some(title.to_string()),
            content: Some("Canonical body".to_string()),
            summary: Some("Canonical summary".to_string()),
            is_flagged: false,
            project_id: Some("project-1".to_string()),
            metadata: Some(serde_json::json!({"source": "remote"})),
            source: Some(serde_json::json!({"kind": "plain"})),
            created_at: Some("2026-08-09T00:00:00Z".to_string()),
            updated_at: Some("2026-08-09T00:00:01Z".to_string()),
            deleted_at: None,
        }
    }

    #[tokio::test]
    async fn remote_committed_note_is_fully_visible_before_return() {
        let (_directory, db) = test_powersync_db().await;
        let inserted = commit_remote_note(&db, &remote_note("note-full", "Remote title"))
            .await
            .unwrap();

        assert!(inserted);
        let reader = db.reader().await.unwrap();
        let row = reader
            .query_row(
                "SELECT short_id, title, summary, metadata, source FROM notes WHERE id = ?",
                params!["note-full"],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                    ))
                },
            )
            .unwrap();
        assert_eq!(row.0, 42);
        assert_eq!(row.1, "Remote title");
        assert_eq!(row.2, "Canonical summary");
        assert_eq!(row.3, r#"{"source":"remote"}"#);
        assert_eq!(row.4, r#"{"kind":"plain"}"#);

        let transaction = db.next_crud_transaction().await.unwrap().unwrap();
        assert_eq!(
            transaction.crud[0].metadata.as_deref(),
            Some(REMOTE_COMMITTED_INSERT_METADATA)
        );
    }

    #[tokio::test]
    async fn remote_committed_note_does_not_replace_row_downloaded_first() {
        let (_directory, db) = test_powersync_db().await;
        {
            let writer = db.writer().await.unwrap();
            writer
                .execute(
                    "INSERT INTO notes (id, short_id, user_id, type, status, title) VALUES (?, ?, ?, ?, ?, ?)",
                    params!["note-race", 42, "user-1", "normal", "ready", "Newer title"],
                )
                .unwrap();
            writer.execute("DELETE FROM ps_crud", []).unwrap();
        }

        let inserted = commit_remote_note(&db, &remote_note("note-race", "Older title"))
            .await
            .unwrap();

        assert!(!inserted);
        let reader = db.reader().await.unwrap();
        let title: String = reader
            .query_row(
                "SELECT title FROM notes WHERE id = ?",
                params!["note-race"],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(title, "Newer title");
        assert!(db.next_crud_transaction().await.unwrap().is_none());
    }

    #[tokio::test]
    async fn remote_committed_insert_records_marker_in_crud() {
        let (_directory, db) = test_powersync_db().await;
        insert_marked_note(&db).await;

        let transaction = db.next_crud_transaction().await.unwrap().unwrap();
        assert_eq!(transaction.crud.len(), 1);
        assert_eq!(transaction.crud[0].table, "notes");
        assert!(matches!(
            transaction.crud.first().map(|entry| &entry.update_type),
            Some(UpdateType::Put)
        ));
        assert_eq!(
            transaction.crud[0].metadata.as_deref(),
            Some(r#"{"flicknote":"remote_committed_insert_v1"}"#)
        );
    }

    #[tokio::test]
    async fn existing_database_upgrades_to_metadata_tracking_without_losing_rows() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("upgrade.db");
        let mut legacy_schema = app_schema();
        for table in &mut legacy_schema.tables {
            if matches!(table.name.as_ref(), "notes" | "note_extractions") {
                table.options.track_metadata = false;
            }
        }
        {
            let legacy_db = test_powersync_db_at(&path, legacy_schema);
            let writer = legacy_db.writer().await.unwrap();
            writer
                .execute(
                    "INSERT INTO notes (id, user_id, type, status, title) VALUES (?, ?, ?, ?, ?)",
                    params!["existing-note", "user-1", "normal", "ready", "Preserved"],
                )
                .unwrap();
            writer.execute("DELETE FROM ps_crud", []).unwrap();
        }

        let upgraded_db = test_powersync_db_at(&path, app_schema());
        {
            let writer = upgraded_db.writer().await.unwrap();
            let title: String = writer
                .query_row(
                    "SELECT title FROM notes WHERE id = ?",
                    params!["existing-note"],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(title, "Preserved");
            writer
                .execute(
                    "INSERT INTO notes (id, user_id, type, status, title, _metadata) VALUES (?, ?, ?, ?, ?, ?)",
                    params![
                        "marked-after-upgrade",
                        "user-1",
                        "normal",
                        "ready",
                        "Marked",
                        REMOTE_COMMITTED_INSERT_METADATA,
                    ],
                )
                .unwrap();
        }

        let transaction = upgraded_db.next_crud_transaction().await.unwrap().unwrap();
        assert_eq!(transaction.crud.len(), 1);
        assert_eq!(transaction.crud[0].id, "marked-after-upgrade");
        assert_eq!(
            transaction.crud[0].metadata.as_deref(),
            Some(REMOTE_COMMITTED_INSERT_METADATA)
        );
    }

    #[tokio::test]
    async fn existing_database_retires_keyterm_schema_without_losing_projects() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("keyterm-retirement.db");
        let mut legacy_schema = app_schema();
        let projects = legacy_schema
            .tables
            .iter_mut()
            .find(|table| table.name.as_ref() == "projects")
            .unwrap();
        if !projects
            .columns
            .iter()
            .any(|column| column.name.as_ref() == "keyterm_id")
        {
            projects
                .columns
                .push(powersync::schema::Column::text("keyterm_id"));
        }
        if !legacy_schema
            .tables
            .iter()
            .any(|table| table.name.as_ref() == "keyterms")
        {
            legacy_schema.tables.push(powersync::schema::Table::create(
                "keyterms",
                vec![
                    powersync::schema::Column::text("user_id"),
                    powersync::schema::Column::text("name"),
                    powersync::schema::Column::text("description"),
                    powersync::schema::Column::text("content"),
                    powersync::schema::Column::text("created_at"),
                    powersync::schema::Column::text("updated_at"),
                ],
                |_| {},
            ));
        }

        {
            let legacy_db = test_powersync_db_at(&path, legacy_schema);
            let writer = legacy_db.writer().await.unwrap();
            writer
                .execute(
                    "INSERT INTO keyterms (id, user_id, name) VALUES (?, ?, ?)",
                    params!["retired-keyterm", "user-1", "Retired"],
                )
                .unwrap();
            writer
                .execute(
                    "INSERT INTO projects (id, user_id, name, keyterm_id) VALUES (?, ?, ?, ?)",
                    params![
                        "preserved-project",
                        "user-1",
                        "Preserved",
                        "retired-keyterm"
                    ],
                )
                .unwrap();
            writer.execute("DELETE FROM ps_crud", []).unwrap();
        }

        let upgraded_db = test_powersync_db_at(&path, app_schema());
        let writer = upgraded_db.writer().await.unwrap();
        let project_name: String = writer
            .query_row(
                "SELECT name FROM projects WHERE id = ?",
                params!["preserved-project"],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(project_name, "Preserved");
        let retired_view_count: i64 = writer
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'view' AND name = 'keyterms'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(retired_view_count, 0);
        let retired_column_count: i64 = writer
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('projects') WHERE name = 'keyterm_id'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(retired_column_count, 0);
    }

    #[tokio::test]
    async fn remote_committed_put_completes_without_http_request() {
        let (_directory, db) = test_powersync_db().await;
        insert_marked_note(&db).await;

        let uploaded = run_upload(
            &db,
            &reqwest::Client::new(),
            "token",
            "http://127.0.0.1:1",
            "anon-key",
        )
        .await
        .unwrap();

        assert!(uploaded);
        assert!(db.next_crud_transaction().await.unwrap().is_none());
    }

    #[tokio::test]
    async fn remote_committed_marker_is_matched_as_json_not_raw_text() {
        let (_directory, db) = test_powersync_db().await;
        insert_note_with_metadata(&db, r#"{ "flicknote" : "remote_committed_insert_v1" }"#).await;

        run_upload(
            &db,
            &reqwest::Client::new(),
            "token",
            "http://127.0.0.1:1",
            "anon-key",
        )
        .await
        .unwrap();

        assert!(db.next_crud_transaction().await.unwrap().is_none());
    }

    #[tokio::test]
    async fn remote_committed_marker_rejects_extra_metadata_fields() {
        let (_directory, db) = test_powersync_db().await;
        insert_note_with_metadata(
            &db,
            r#"{"flicknote":"remote_committed_insert_v1","other":true}"#,
        )
        .await;

        let error = run_upload(
            &db,
            &reqwest::Client::new(),
            "token",
            "http://127.0.0.1:1",
            "anon",
        )
        .await
        .unwrap_err();

        assert!(
            error
                .to_string()
                .contains("invalid FlickNote CRUD metadata")
        );
        assert!(db.crud_transactions().try_next().await.unwrap().is_some());
    }

    #[tokio::test]
    async fn unsupported_flicknote_marker_is_rejected_and_retained() {
        let (_directory, db) = test_powersync_db().await;
        insert_note_with_metadata(&db, r#"{"flicknote":"remote_committed_insert_v2"}"#).await;

        let error = run_upload(
            &db,
            &reqwest::Client::new(),
            "token",
            "http://127.0.0.1:1",
            "anon-key",
        )
        .await
        .unwrap_err();

        assert!(
            error
                .to_string()
                .contains("unsupported FlickNote CRUD marker")
        );
        assert!(db.next_crud_transaction().await.unwrap().is_some());
    }

    #[tokio::test]
    async fn malformed_crud_metadata_is_rejected_and_retained() {
        let (_directory, db) = test_powersync_db().await;
        insert_note_with_metadata(&db, r#"{"flicknote":"remote_committed_insert_v1""#).await;

        let error = run_upload(
            &db,
            &reqwest::Client::new(),
            "token",
            "http://127.0.0.1:1",
            "anon-key",
        )
        .await
        .unwrap_err();

        assert!(error.to_string().contains("invalid CRUD metadata"));
        assert!(db.next_crud_transaction().await.unwrap().is_some());
    }

    #[tokio::test]
    async fn remote_committed_marker_on_patch_is_rejected_and_retained() {
        let (_directory, db) = test_powersync_db().await;
        insert_marked_note(&db).await;
        db.next_crud_transaction()
            .await
            .unwrap()
            .unwrap()
            .complete()
            .await
            .unwrap();
        {
            let writer = db.writer().await.unwrap();
            writer
                .execute(
                    "UPDATE notes SET title = ?, _metadata = ? WHERE id = ?",
                    params!["Changed", REMOTE_COMMITTED_INSERT_METADATA, "note-1"],
                )
                .unwrap();
        }

        let error = run_upload(
            &db,
            &reqwest::Client::new(),
            "token",
            "http://127.0.0.1:1",
            "anon-key",
        )
        .await
        .unwrap_err();

        assert!(
            error
                .to_string()
                .contains("invalid remote-committed marker")
        );
        assert!(db.next_crud_transaction().await.unwrap().is_some());
    }

    #[tokio::test]
    async fn remote_committed_extractions_are_visible_before_return() {
        let (_directory, db) = test_powersync_db().await;
        let rows = vec![RemoteExtractionRow {
            id: "extraction-1".to_string(),
            note_id: "note-1".to_string(),
            user_id: "user-1".to_string(),
            key: TOPIC_EXTRACTION_KEY.to_string(),
            value: "rust".to_string(),
        }];

        let inserted = commit_remote_extractions(&db, &rows).await.unwrap();

        assert_eq!(inserted, 1);
        let reader = db.reader().await.unwrap();
        let value: String = reader
            .query_row(
                "SELECT value FROM note_extractions WHERE id = ?",
                params!["extraction-1"],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(value, "rust");
        let transaction = db.next_crud_transaction().await.unwrap().unwrap();
        assert_eq!(
            transaction.crud[0].metadata.as_deref(),
            Some(REMOTE_COMMITTED_INSERT_METADATA)
        );
    }

    #[tokio::test]
    async fn share_request_lock_serializes_operations() {
        let lock = Arc::new(ShareRequestLock::default());
        let active = Arc::new(AtomicUsize::new(0));
        let max_active = Arc::new(AtomicUsize::new(0));

        let operation = || {
            let lock = Arc::clone(&lock);
            let active = Arc::clone(&active);
            let max_active = Arc::clone(&max_active);
            async move {
                lock.run(async {
                    let current = active.fetch_add(1, Ordering::SeqCst) + 1;
                    max_active.fetch_max(current, Ordering::SeqCst);
                    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                    active.fetch_sub(1, Ordering::SeqCst);
                })
                .await;
            }
        };

        tokio::join!(operation(), operation());

        assert_eq!(max_active.load(Ordering::SeqCst), 1);
    }

    fn test_config(api_url: String) -> Config {
        Config {
            supabase_url: String::new(),
            supabase_anon_key: String::new(),
            powersync_url: String::new(),
            api_url,
            web_url: None,
            paths: ConfigPaths {
                config_dir: PathBuf::new(),
                data_dir: PathBuf::new(),
                config_file: PathBuf::new(),
                session_file: PathBuf::new(),
                db_file: PathBuf::new(),
                log_file: PathBuf::new(),
            },
        }
    }

    fn spawn_server(
        responses: Vec<(&'static str, &'static str)>,
    ) -> (String, thread::JoinHandle<Vec<String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let handle = thread::spawn(move || {
            let mut requests = Vec::new();
            for (status, body) in responses {
                let (mut stream, _) = listener.accept().unwrap();
                let mut buffer = [0_u8; 4096];
                let count = stream.read(&mut buffer).unwrap();
                let request = String::from_utf8_lossy(&buffer[..count]);
                requests.push(request.lines().next().unwrap_or_default().to_string());
                let response = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                stream.write_all(response.as_bytes()).unwrap();
            }
            requests
        });
        (format!("http://{address}"), handle)
    }

    fn spawn_disconnected_response_then_server(
        status: &'static str,
        body: &'static str,
    ) -> (String, thread::JoinHandle<Vec<String>>) {
        spawn_disconnected_then_retry_responses(vec![(status, body)])
    }

    fn spawn_disconnected_then_retry_responses(
        responses: Vec<(&'static str, &'static str)>,
    ) -> (String, thread::JoinHandle<Vec<String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let handle = thread::spawn(move || {
            let (mut first, _) = listener.accept().unwrap();
            listener.set_nonblocking(true).unwrap();
            let accept = || {
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
                loop {
                    match listener.accept() {
                        Ok(pair) => return Some(pair),
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            if std::time::Instant::now() >= deadline {
                                return None;
                            }
                            thread::sleep(std::time::Duration::from_millis(5));
                        }
                        Err(error) => panic!("accept failed: {error}"),
                    }
                }
            };

            let mut requests = Vec::new();
            let mut buffer = [0_u8; 4096];
            let count = first.read(&mut buffer).unwrap();
            requests.push(
                String::from_utf8_lossy(&buffer[..count])
                    .lines()
                    .next()
                    .unwrap_or_default()
                    .to_string(),
            );
            drop(first);

            for (status, body) in responses {
                let Some((mut stream, _)) = accept() else {
                    break;
                };
                let count = stream.read(&mut buffer).unwrap();
                requests.push(
                    String::from_utf8_lossy(&buffer[..count])
                        .lines()
                        .next()
                        .unwrap_or_default()
                        .to_string(),
                );
                let response = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                stream.write_all(response.as_bytes()).unwrap();
            }
            requests
        });
        (format!("http://{address}"), handle)
    }

    #[test]
    fn partial_remote_create_maps_to_non_retryable_structured_service_error() {
        let error = remote_create_service_error(DaemonError::PartialCreate {
            message: "note created; topics pending".to_string(),
            note_id: "note-partial".to_string(),
            short_id: Some(80),
            confirmed_extraction_ids: vec!["extraction-confirmed".to_string()],
            pending_extraction_ids: vec!["extraction-1".to_string()],
        });

        assert_eq!(error.code(), "note_create_partial");
        assert!(!error.retryable());
        let flicknote_core::services::error::ServiceError::Remote { details, .. } = error else {
            panic!("expected remote service error")
        };
        let details = details.unwrap();
        assert_eq!(details["short_id"], 80);
        assert_eq!(
            details["confirmed_extraction_ids"],
            serde_json::json!(["extraction-confirmed"])
        );
    }

    #[tokio::test]
    async fn remote_create_returns_after_canonical_note_is_committed_locally() {
        let body = r#"[{"id":"note-create","short_id":77,"user_id":"user-1","type":"normal","status":"ai_queued","title":"Remote title","content":"Body","summary":null,"is_flagged":false,"project_id":null,"metadata":null,"source":null,"created_at":"2026-08-09T00:00:00Z","updated_at":"2026-08-09T00:00:00Z","deleted_at":null}]"#;
        let (origin, server) = spawn_server(vec![("201 Created", body)]);
        let mut config = test_config(String::new());
        config.supabase_url = origin;
        config.supabase_anon_key = "anon-key".to_string();
        let (_directory, db) = test_powersync_db().await;
        let request = CreateNoteRequest {
            id: "note-create".to_string(),
            note_type: "normal".to_string(),
            status: "ai_queued".to_string(),
            title: Some("Requested title".to_string()),
            content: Some("Body".to_string()),
            metadata: None,
            project_id: None,
            now: "2026-08-09T00:00:00Z".to_string(),
            topics: Vec::new(),
            attachment_path: None,
        };

        let created = create_note_with_token(
            &db,
            &reqwest::Client::new(),
            &config,
            "access-token",
            "user-1",
            request,
        )
        .await
        .unwrap();

        assert_eq!(created.uuid, "note-create");
        assert_eq!(created.short_id, 77);
        let reader = db.reader().await.unwrap();
        let title: String = reader
            .query_row(
                "SELECT title FROM notes WHERE id = ?",
                params!["note-create"],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(title, "Remote title");
        assert_eq!(
            server.join().unwrap(),
            ["POST /rest/v1/notes?on_conflict=id HTTP/1.1"]
        );
    }

    #[tokio::test]
    async fn remote_create_reports_typed_partial_success_after_note_commit() {
        let note = r#"[{"id":"note-partial","short_id":80,"user_id":"user-1","type":"normal","status":"ai_queued","title":"Remote title","content":"Body","summary":null,"is_flagged":false,"project_id":null,"metadata":null,"source":null,"created_at":"2026-08-09T00:00:00Z","updated_at":"2026-08-09T00:00:00Z","deleted_at":null}]"#;
        let (origin, server) = spawn_server(vec![
            ("201 Created", note),
            (
                "500 Internal Server Error",
                r#"{"message":"topic failure"}"#,
            ),
        ]);
        let mut config = test_config(String::new());
        config.supabase_url = origin;
        config.supabase_anon_key = "anon-key".to_string();
        let (_directory, db) = test_powersync_db().await;

        let error = create_note_with_token(
            &db,
            &reqwest::Client::new(),
            &config,
            "access-token",
            "user-1",
            CreateNoteRequest {
                id: "note-partial".to_string(),
                note_type: "normal".to_string(),
                status: "ai_queued".to_string(),
                title: Some("Requested title".to_string()),
                content: Some("Body".to_string()),
                metadata: None,
                project_id: None,
                now: "2026-08-09T00:00:00Z".to_string(),
                topics: vec!["rust".to_string()],
                attachment_path: None,
            },
        )
        .await
        .unwrap_err();

        let DaemonError::PartialCreate {
            note_id,
            short_id,
            pending_extraction_ids,
            ..
        } = error
        else {
            panic!("expected partial create error")
        };
        assert_eq!(note_id, "note-partial");
        assert_eq!(short_id, Some(80));
        assert_eq!(pending_extraction_ids.len(), 1);
        let reader = db.reader().await.unwrap();
        let count: i64 = reader
            .query_row(
                "SELECT COUNT(*) FROM notes WHERE id = ?",
                params!["note-partial"],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
        assert_eq!(server.join().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn remote_create_recovers_empty_idempotent_response_by_stable_uuid() {
        let body = r#"[{"id":"note-retry","short_id":78,"user_id":"user-1","type":"normal","status":"ai_queued","title":"Recovered","content":"Body","summary":null,"is_flagged":false,"project_id":null,"metadata":null,"source":null,"created_at":"2026-08-09T00:00:00Z","updated_at":"2026-08-09T00:00:00Z","deleted_at":null}]"#;
        let (origin, server) = spawn_server(vec![("200 OK", "[]"), ("200 OK", body)]);
        let mut config = test_config(String::new());
        config.supabase_url = origin;
        config.supabase_anon_key = "anon-key".to_string();
        let (_directory, db) = test_powersync_db().await;
        let request = CreateNoteRequest {
            id: "note-retry".to_string(),
            note_type: "normal".to_string(),
            status: "ai_queued".to_string(),
            title: Some("Requested".to_string()),
            content: Some("Body".to_string()),
            metadata: None,
            project_id: None,
            now: "2026-08-09T00:00:00Z".to_string(),
            topics: Vec::new(),
            attachment_path: None,
        };

        let created = create_note_with_token(
            &db,
            &reqwest::Client::new(),
            &config,
            "access-token",
            "user-1",
            request,
        )
        .await
        .unwrap();

        assert_eq!(created.short_id, 78);
        assert_eq!(
            server.join().unwrap(),
            [
                "POST /rest/v1/notes?on_conflict=id HTTP/1.1",
                "GET /rest/v1/notes?id=eq.note-retry&select=* HTTP/1.1",
            ]
        );
    }

    #[tokio::test]
    async fn remote_create_recovers_malformed_success_response_by_stable_uuid() {
        let body = r#"[{"id":"note-malformed","short_id":81,"user_id":"user-1","type":"normal","status":"ai_queued","title":"Recovered","content":"Body","summary":null,"is_flagged":false,"project_id":null,"metadata":null,"source":null,"created_at":"2026-08-09T00:00:00Z","updated_at":"2026-08-09T00:00:00Z","deleted_at":null}]"#;
        let (origin, server) = spawn_server(vec![("201 Created", "{"), ("200 OK", body)]);
        let mut config = test_config(String::new());
        config.supabase_url = origin;
        config.supabase_anon_key = "anon-key".to_string();
        let (_directory, db) = test_powersync_db().await;

        let created = create_note_with_token(
            &db,
            &reqwest::Client::new(),
            &config,
            "access-token",
            "user-1",
            CreateNoteRequest {
                id: "note-malformed".to_string(),
                note_type: "normal".to_string(),
                status: "ai_queued".to_string(),
                title: Some("Requested".to_string()),
                content: Some("Body".to_string()),
                metadata: None,
                project_id: None,
                now: "2026-08-09T00:00:00Z".to_string(),
                topics: Vec::new(),
                attachment_path: None,
            },
        )
        .await
        .unwrap();

        assert_eq!(created.short_id, 81);
        assert_eq!(
            server.join().unwrap(),
            [
                "POST /rest/v1/notes?on_conflict=id HTTP/1.1",
                "GET /rest/v1/notes?id=eq.note-malformed&select=* HTTP/1.1",
            ]
        );
    }

    #[tokio::test]
    async fn malformed_success_with_failed_reconciliation_reports_confirmed_create() {
        let (origin, server) = spawn_server(vec![
            ("201 Created", "{"),
            ("503 Service Unavailable", r#"{"message":"try later"}"#),
        ]);
        let mut config = test_config(String::new());
        config.supabase_url = origin;
        config.supabase_anon_key = "anon-key".to_string();
        let (_directory, db) = test_powersync_db().await;

        let error = create_note_with_token(
            &db,
            &reqwest::Client::new(),
            &config,
            "access-token",
            "user-1",
            CreateNoteRequest {
                id: "note-confirmed".to_string(),
                note_type: "normal".to_string(),
                status: "ai_queued".to_string(),
                title: Some("Requested".to_string()),
                content: Some("Body".to_string()),
                metadata: None,
                project_id: None,
                now: "2026-08-09T00:00:00Z".to_string(),
                topics: Vec::new(),
                attachment_path: None,
            },
        )
        .await
        .unwrap_err();
        let service_error = remote_create_service_error(error);

        assert_eq!(service_error.code(), "note_create_partial");
        let flicknote_core::services::error::ServiceError::Remote { details, .. } = service_error
        else {
            panic!("expected structured remote error")
        };
        let details = details.unwrap();
        assert_eq!(details["created"], true);
        assert_eq!(details["note_id"], "note-confirmed");
        assert!(details["short_id"].is_null());
        assert_eq!(server.join().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn local_commit_failure_after_remote_create_reports_partial_success() {
        let note = r#"[{"id":"note-local-failure","short_id":82,"user_id":"user-1","type":"normal","status":"ai_queued","title":"Remote title","content":"Body","summary":null,"is_flagged":false,"project_id":null,"metadata":null,"source":null,"created_at":"2026-08-09T00:00:00Z","updated_at":"2026-08-09T00:00:00Z","deleted_at":null}]"#;
        let (origin, server) = spawn_server(vec![("201 Created", note)]);
        let mut config = test_config(String::new());
        config.supabase_url = origin;
        config.supabase_anon_key = "anon-key".to_string();
        let (_directory, db) = test_powersync_db().await;
        db.writer()
            .await
            .unwrap()
            .execute("DROP VIEW notes", [])
            .unwrap();

        let error = create_note_with_token(
            &db,
            &reqwest::Client::new(),
            &config,
            "access-token",
            "user-1",
            CreateNoteRequest {
                id: "note-local-failure".to_string(),
                note_type: "normal".to_string(),
                status: "ai_queued".to_string(),
                title: Some("Requested".to_string()),
                content: Some("Body".to_string()),
                metadata: None,
                project_id: None,
                now: "2026-08-09T00:00:00Z".to_string(),
                topics: Vec::new(),
                attachment_path: None,
            },
        )
        .await
        .unwrap_err();
        let service_error = remote_create_service_error(error);

        assert_eq!(service_error.code(), "note_create_partial");
        let flicknote_core::services::error::ServiceError::Remote { details, .. } = service_error
        else {
            panic!("expected structured remote error")
        };
        let details = details.unwrap();
        assert_eq!(details["created"], true);
        assert_eq!(details["note_id"], "note-local-failure");
        assert_eq!(details["short_id"], 82);
        assert_eq!(server.join().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn remote_create_recovers_lost_response_by_stable_uuid() {
        let body = r#"[{"id":"note-lost","short_id":79,"user_id":"user-1","type":"normal","status":"ai_queued","title":"Recovered","content":"Body","summary":null,"is_flagged":false,"project_id":null,"metadata":null,"source":null,"created_at":"2026-08-09T00:00:00Z","updated_at":"2026-08-09T00:00:00Z","deleted_at":null}]"#;
        let (origin, server) = spawn_disconnected_response_then_server("200 OK", body);
        let mut config = test_config(String::new());
        config.supabase_url = origin;
        config.supabase_anon_key = "anon-key".to_string();
        let (_directory, db) = test_powersync_db().await;
        let request = CreateNoteRequest {
            id: "note-lost".to_string(),
            note_type: "normal".to_string(),
            status: "ai_queued".to_string(),
            title: Some("Requested".to_string()),
            content: Some("Body".to_string()),
            metadata: None,
            project_id: None,
            now: "2026-08-09T00:00:00Z".to_string(),
            topics: Vec::new(),
            attachment_path: None,
        };

        let created = create_note_with_token(
            &db,
            &reqwest::Client::new(),
            &config,
            "access-token",
            "user-1",
            request,
        )
        .await
        .unwrap();

        assert_eq!(created.short_id, 79);
        assert_eq!(server.join().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn ambiguous_transport_failure_reports_stable_unknown_outcome() {
        let (origin, server) = spawn_disconnected_response_then_server(
            "503 Service Unavailable",
            r#"{"message":"try later"}"#,
        );
        let mut config = test_config(String::new());
        config.supabase_url = origin;
        config.supabase_anon_key = "anon-key".to_string();
        let (_directory, db) = test_powersync_db().await;

        let error = create_note_with_token(
            &db,
            &reqwest::Client::new(),
            &config,
            "access-token",
            "user-1",
            CreateNoteRequest {
                id: "note-unknown".to_string(),
                note_type: "normal".to_string(),
                status: "ai_queued".to_string(),
                title: Some("Requested".to_string()),
                content: Some("Body".to_string()),
                metadata: None,
                project_id: None,
                now: "2026-08-09T00:00:00Z".to_string(),
                topics: Vec::new(),
                attachment_path: None,
            },
        )
        .await
        .unwrap_err();
        let service_error = remote_create_service_error(error);

        assert_eq!(service_error.code(), "note_create_unknown");
        assert!(!service_error.retryable());
        let flicknote_core::services::error::ServiceError::Remote { details, .. } = service_error
        else {
            panic!("expected structured remote error")
        };
        let details = details.unwrap();
        assert!(details["created"].is_null());
        assert_eq!(details["note_id"], "note-unknown");
        assert_eq!(server.join().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn ambiguous_transport_failure_retries_create_with_the_same_stable_uuid() {
        let body = r#"[{"id":"note-recovered-after-retry","short_id":83,"user_id":"user-1","type":"normal","status":"ai_queued","title":"Recovered","content":"Body","summary":null,"is_flagged":false,"project_id":null,"metadata":null,"source":null,"created_at":"2026-08-09T00:00:00Z","updated_at":"2026-08-09T00:00:00Z","deleted_at":null}]"#;
        let (origin, server) = spawn_disconnected_then_retry_responses(vec![("201 Created", body)]);
        let mut config = test_config(String::new());
        config.supabase_url = origin;
        config.supabase_anon_key = "anon-key".to_string();
        let (_directory, db) = test_powersync_db().await;

        let result = create_note_with_token(
            &db,
            &reqwest::Client::new(),
            &config,
            "access-token",
            "user-1",
            CreateNoteRequest {
                id: "note-recovered-after-retry".to_string(),
                note_type: "normal".to_string(),
                status: "ai_queued".to_string(),
                title: Some("Requested".to_string()),
                content: Some("Body".to_string()),
                metadata: None,
                project_id: None,
                now: "2026-08-09T00:00:00Z".to_string(),
                topics: Vec::new(),
                attachment_path: None,
            },
        )
        .await;
        let requests = server.join().unwrap();

        let created = result.unwrap();
        assert_eq!(created.short_id, 83);
        assert_eq!(requests.len(), 2);
        assert!(requests[0].starts_with("POST /rest/v1/notes"));
        assert!(requests[1].starts_with("POST /rest/v1/notes"));
    }

    #[tokio::test]
    async fn retryable_status_retries_create_with_the_same_stable_uuid() {
        let body = r#"[{"id":"note-retryable-status","short_id":84,"user_id":"user-1","type":"normal","status":"ai_queued","title":"Recovered","content":"Body","summary":null,"is_flagged":false,"project_id":null,"metadata":null,"source":null,"created_at":"2026-08-09T00:00:00Z","updated_at":"2026-08-09T00:00:00Z","deleted_at":null}]"#;
        let (origin, server) = spawn_server(vec![
            ("503 Service Unavailable", r#"{"message":"try later"}"#),
            ("201 Created", body),
        ]);
        let mut config = test_config(String::new());
        config.supabase_url = origin;
        config.supabase_anon_key = "anon-key".to_string();
        let (_directory, db) = test_powersync_db().await;

        let created = create_note_with_token(
            &db,
            &reqwest::Client::new(),
            &config,
            "access-token",
            "user-1",
            CreateNoteRequest {
                id: "note-retryable-status".to_string(),
                note_type: "normal".to_string(),
                status: "ai_queued".to_string(),
                title: Some("Requested".to_string()),
                content: Some("Body".to_string()),
                metadata: None,
                project_id: None,
                now: "2026-08-09T00:00:00Z".to_string(),
                topics: Vec::new(),
                attachment_path: None,
            },
        )
        .await
        .unwrap();
        let requests = server.join().unwrap();

        assert_eq!(created.short_id, 84);
        assert_eq!(requests.len(), 2);
        assert!(
            requests
                .iter()
                .all(|request| request.starts_with("POST /rest/v1/notes"))
        );
    }

    #[tokio::test]
    async fn remote_extraction_create_commits_confirmed_rows_locally() {
        let body = r#"[{"id":"extraction-create","note_id":"note-create","user_id":"user-1","key":"::topic","value":"rust"}]"#;
        let (origin, server) = spawn_server(vec![("201 Created", body)]);
        let mut config = test_config(String::new());
        config.supabase_url = origin;
        config.supabase_anon_key = "anon-key".to_string();
        let (_directory, db) = test_powersync_db().await;
        let requested = vec![RemoteExtractionRow {
            id: "extraction-create".to_string(),
            note_id: "note-create".to_string(),
            user_id: "user-1".to_string(),
            key: TOPIC_EXTRACTION_KEY.to_string(),
            value: "rust".to_string(),
        }];

        let outcome = create_extractions_with_token(
            &db,
            &reqwest::Client::new(),
            &config,
            "access-token",
            &requested,
        )
        .await;

        assert_eq!(outcome.confirmed_ids, ["extraction-create"]);
        assert!(outcome.pending_ids.is_empty());
        let reader = db.reader().await.unwrap();
        let count: i64 = reader
            .query_row(
                "SELECT COUNT(*) FROM note_extractions WHERE id = ?",
                params!["extraction-create"],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
        assert_eq!(
            server.join().unwrap(),
            ["POST /rest/v1/note_extractions?on_conflict=id HTTP/1.1"]
        );
    }

    #[tokio::test]
    async fn remote_extraction_create_recovers_by_stable_uuid() {
        let body = r#"[{"id":"extraction-retry","note_id":"note-create","user_id":"user-1","key":"::topic","value":"rust"}]"#;
        let (origin, server) = spawn_server(vec![("200 OK", "[]"), ("200 OK", body)]);
        let mut config = test_config(String::new());
        config.supabase_url = origin;
        config.supabase_anon_key = "anon-key".to_string();
        let (_directory, db) = test_powersync_db().await;
        let requested = vec![RemoteExtractionRow {
            id: "extraction-retry".to_string(),
            note_id: "note-create".to_string(),
            user_id: "user-1".to_string(),
            key: TOPIC_EXTRACTION_KEY.to_string(),
            value: "rust".to_string(),
        }];

        let outcome = create_extractions_with_token(
            &db,
            &reqwest::Client::new(),
            &config,
            "access-token",
            &requested,
        )
        .await;

        assert_eq!(outcome.confirmed_ids, ["extraction-retry"]);
        assert!(outcome.pending_ids.is_empty());
        assert_eq!(
            server.join().unwrap(),
            [
                "POST /rest/v1/note_extractions?on_conflict=id HTTP/1.1",
                "GET /rest/v1/note_extractions?id=eq.extraction-retry&select=* HTTP/1.1",
            ]
        );
    }

    #[tokio::test]
    async fn remote_extraction_create_commits_confirmed_subset_and_reports_exact_pending_ids() {
        let body = r#"[{"id":"extraction-confirmed","note_id":"note-create","user_id":"user-1","key":"::topic","value":"rust"}]"#;
        let (origin, server) = spawn_server(vec![("201 Created", body), ("200 OK", "[]")]);
        let mut config = test_config(String::new());
        config.supabase_url = origin;
        config.supabase_anon_key = "anon-key".to_string();
        let (_directory, db) = test_powersync_db().await;
        let requested = vec![
            RemoteExtractionRow {
                id: "extraction-confirmed".to_string(),
                note_id: "note-create".to_string(),
                user_id: "user-1".to_string(),
                key: TOPIC_EXTRACTION_KEY.to_string(),
                value: "rust".to_string(),
            },
            RemoteExtractionRow {
                id: "extraction-pending".to_string(),
                note_id: "note-create".to_string(),
                user_id: "user-1".to_string(),
                key: TOPIC_EXTRACTION_KEY.to_string(),
                value: "sqlite".to_string(),
            },
        ];

        let outcome = create_extractions_with_token(
            &db,
            &reqwest::Client::new(),
            &config,
            "access-token",
            &requested,
        )
        .await;

        assert_eq!(outcome.confirmed_ids, ["extraction-confirmed"]);
        assert_eq!(outcome.pending_ids, ["extraction-pending"]);
        let reader = db.reader().await.unwrap();
        let count: i64 = reader
            .query_row(
                "SELECT COUNT(*) FROM note_extractions WHERE id = ?",
                params!["extraction-confirmed"],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
        assert_eq!(server.join().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn returns_existing_note_share_without_replacing_it() {
        let (api_origin, server) = spawn_server(vec![(
            "200 OK",
            r#"{"token":"existing","url":"https://flicknote.app/s/existing"}"#,
        )]);
        let config = test_config(format!("{api_origin}/api/v1"));
        let request = ShareRequest {
            resource: ShareResource::Note,
            id: "550e8400-e29b-41d4-a716-446655440000".to_string(),
        };

        let url = get_or_create_share_with_token(
            &reqwest::Client::new(),
            &config,
            "access-token",
            &request,
        )
        .await
        .unwrap();

        assert_eq!(url, "https://flicknote.app/s/existing");
        assert_eq!(
            server.join().unwrap(),
            ["GET /api/v1/notes/550e8400-e29b-41d4-a716-446655440000/share HTTP/1.1"]
        );
    }

    #[tokio::test]
    async fn creates_project_share_when_none_exists() {
        let (api_url, server) = spawn_server(vec![
            (
                "404 Not Found",
                r#"{"_tag":"NotFoundError","message":"No project share link exists for this project","errorCode":"PROJECT_SHARE_NOT_FOUND"}"#,
            ),
            (
                "200 OK",
                r#"{"token":"new-token","url":"https://flicknote.app/p/new-token"}"#,
            ),
        ]);
        let config = test_config(api_url);
        let request = ShareRequest {
            resource: ShareResource::Project,
            id: "550e8400-e29b-41d4-a716-446655440000".to_string(),
        };

        let url = get_or_create_share_with_token(
            &reqwest::Client::new(),
            &config,
            "access-token",
            &request,
        )
        .await
        .unwrap();

        assert_eq!(url, "https://flicknote.app/p/new-token");
        assert_eq!(
            server.join().unwrap(),
            [
                "GET /api/v1/projects/550e8400-e29b-41d4-a716-446655440000/share HTTP/1.1",
                "POST /api/v1/projects/550e8400-e29b-41d4-a716-446655440000/share HTTP/1.1",
            ]
        );
    }

    #[tokio::test]
    async fn revokes_existing_note_share() {
        let (api_url, server) = spawn_server(vec![("200 OK", r#"{"success":true}"#)]);
        let config = test_config(api_url);
        let request = ShareRequest {
            resource: ShareResource::Note,
            id: "550e8400-e29b-41d4-a716-446655440000".to_string(),
        };

        revoke_share_with_token(&reqwest::Client::new(), &config, "access-token", &request)
            .await
            .unwrap();

        assert_eq!(
            server.join().unwrap(),
            ["DELETE /api/v1/notes/550e8400-e29b-41d4-a716-446655440000/share HTTP/1.1"]
        );
    }

    #[test]
    fn test_extract_fatal_code_fk_violation() {
        let body = r#"{"code":"23503","details":"Key is not present in table \"projects\".","hint":null,"message":"insert or update on table \"notes\" violates foreign key constraint"}"#;
        assert_eq!(extract_fatal_code(body), Some("23503".to_string()));
    }

    #[test]
    fn test_extract_fatal_code_rls_violation() {
        let body = r#"{"code":"42501","message":"new row violates row-level security policy"}"#;
        assert_eq!(extract_fatal_code(body), Some("42501".to_string()));
    }

    #[test]
    fn test_extract_fatal_code_transient() {
        let body = r#"{"code":"08006","message":"connection failure"}"#;
        assert_eq!(extract_fatal_code(body), None);
    }

    #[test]
    fn test_extract_fatal_code_not_json() {
        assert_eq!(extract_fatal_code("Internal Server Error"), None);
    }

    #[test]
    fn test_extract_fatal_code_postgrest() {
        let body = r#"{"code":"PGRST204","message":"column not found"}"#;
        assert_eq!(extract_fatal_code(body), Some("PGRST204".to_string()));
    }

    #[test]
    fn test_extract_fatal_code_class22_data_exception() {
        let body = r#"{"code":"22001","message":"value too long for type character varying(255)"}"#;
        assert_eq!(extract_fatal_code(body), Some("22001".to_string()));
    }

    #[test]
    fn test_extract_fatal_code_missing_code_field() {
        // Supabase auth-layer errors omit "code" — should be treated as unknown (transient)
        let body = r#"{"error":"invalid_grant","error_description":"Refresh Token Not Found"}"#;
        assert_eq!(extract_fatal_code(body), None);
    }

    #[test]
    fn test_unwrap_json_strings() {
        let mut data = serde_json::Map::new();
        data.insert("title".into(), serde_json::Value::String("Hello".into()));
        data.insert(
            "metadata".into(),
            serde_json::Value::String(r#"{"file":{"name":"photo.jpg"}}"#.into()),
        );
        data.insert(
            "tags".into(),
            serde_json::Value::String(r#"["rust","cli"]"#.into()),
        );
        // Primitive JSON values ("42", "true") must stay as strings — guard is is_object()||is_array().
        data.insert("count".into(), serde_json::Value::String("42".into()));
        data.insert("flag".into(), serde_json::Value::String("true".into()));
        data.insert("source".into(), serde_json::Value::Null);
        unwrap_json_strings(&mut data);
        assert_eq!(data["title"], serde_json::Value::String("Hello".into())); // plain string unchanged
        assert!(data["metadata"].is_object()); // JSON object string → Value::Object
        assert!(data["tags"].is_array()); // JSON array string → Value::Array
        assert_eq!(data["count"], serde_json::Value::String("42".into())); // primitive JSON unchanged
        assert_eq!(data["flag"], serde_json::Value::String("true".into())); // primitive JSON unchanged
        assert!(data["source"].is_null()); // null unchanged
    }
}
