use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use flicknote_auth::client::GoTrueClient;
use flicknote_core::{config::Config, schema::app_schema};
use futures_lite::StreamExt;
mod http_adapter;
use http_adapter::ReqwestHttpClient;
use notify::{Config as NotifyConfig, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use powersync::{
    BackendConnector, ConnectionPool, PowerSyncCredentials, PowerSyncDatabase, SyncOptions,
    UpdateType, env::PowerSyncEnvironment, error::PowerSyncError,
};
use tokio::sync::mpsc;

/// Helper to convert arbitrary errors into PowerSyncError via http_client::Error
fn ps_err(msg: impl std::fmt::Display) -> PowerSyncError {
    http_client::http_types::Error::from_str(
        http_client::http_types::StatusCode::InternalServerError,
        msg.to_string(),
    )
    .into()
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

/// Inner upload logic shared by the BackendConnector impl and the fsnotify watcher.
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
/// Shared by both the startup path and the watcher loop to avoid divergence.
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
) {
    let _guard = guard.lock().await;

    let token = match auth.get_session().await {
        Ok(s) => s.access_token,
        Err(e) => {
            log::warn!("{context}: auth error: {e}");
            return;
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
        }
        Err(e) => log::warn!("{context}: upload failed: {e}"),
    }
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
        // Ignore the bool — checkpoint is only safe to call from the watcher path,
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
    db: &PowerSyncDatabase,
    db_path: PathBuf,
) {
    upload_handle.abort();
    checkpoint_handle.abort();
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

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init();

    let config = Config::load()?;
    config.validate()?;

    let pid_file = pid_path(&config);
    let _pid_guard = check_and_write_pid(&pid_file)?;

    PowerSyncEnvironment::powersync_auto_extension()?;

    let pool = ConnectionPool::open(&config.paths.db_file)?;
    let client = Arc::new(ReqwestHttpClient::new());
    let env =
        PowerSyncEnvironment::custom(client, pool, Box::new(PowerSyncEnvironment::tokio_timer()));

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

    log::info!("Sync daemon connecting (pid {})", std::process::id());
    db.connect(SyncOptions::new(connector)).await;
    log::info!("Sync daemon connected (pid {})", std::process::id());

    // Watch the WAL file for cross-process writes from the CLI.
    // PowerSync's in-process ps_crud watch can't detect writes from a separate process
    // (e.g. `flicknote add`). Watching the WAL file catches any SQLite write regardless
    // of which process wrote it, with ~200ms trailing-debounce latency.
    let (trigger_tx, mut trigger_rx) = mpsc::channel::<()>(16);

    // Build the WAL filename ("<db_file>-wal") for event filtering.
    // The WAL may not exist yet on a fresh DB, so watch the parent dir and
    // filter by filename — handles both cases without runtime switching.
    let wal_filename = {
        let mut name = config
            .paths
            .db_file
            .file_name()
            .ok_or("db_file path has no filename component")?
            .to_os_string();
        name.push("-wal");
        name
    };
    let db_dir = config
        .paths
        .db_file
        .parent()
        .ok_or("db_file path has no parent directory")?
        .to_path_buf();
    let wal_fname_clone = wal_filename.clone();

    let mut fs_watcher = RecommendedWatcher::new(
        move |res: Result<notify::Event, notify::Error>| match res {
            Err(e) => log::error!("fs_watcher error (uploads may stall): {e}"),
            Ok(event) => {
                if !matches!(event.kind, EventKind::Modify(_)) {
                    return;
                }
                let is_wal = event
                    .paths
                    .iter()
                    .any(|p| p.file_name().is_some_and(|f| f == wal_fname_clone));
                if is_wal && trigger_tx.try_send(()).is_err() {
                    log::debug!("WAL trigger channel full — event dropped (burst in progress)");
                }
            }
        },
        NotifyConfig::default(),
    )?;
    fs_watcher.watch(&db_dir, RecursiveMode::NonRecursive)?;

    let upload_db = db.clone();
    let upload_supabase_url = config.supabase_url.clone();
    let upload_anon_key = config.supabase_anon_key.clone();
    let upload_guard_clone = Arc::clone(&upload_guard);
    let upload_auth_clone = Arc::clone(&auth);
    let upload_db_path = config.paths.db_file.clone();

    let mut upload_handle = tokio::spawn(async move {
        // Initial upload on startup — pick up any ps_crud entries written before
        // the daemon started (e.g. CLI ran while daemon was down).
        try_upload_and_checkpoint(
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
            // Block until a WAL change is detected.
            if trigger_rx.recv().await.is_none() {
                break; // watcher dropped — daemon shutting down
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

            try_upload_and_checkpoint(
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

    tokio::select! {
        _ = tokio::signal::ctrl_c() => {}
        res = &mut upload_handle => {
            if let Err(e) = res {
                log::error!("Upload task panicked: {e}");
                shutdown_daemon(&mut upload_handle, &mut checkpoint_handle, &db, config.paths.db_file.clone()).await;
                return Err(e.into());
            }
        }
        res = &mut checkpoint_handle => {
            match res {
                Ok(_) => log::error!("Checkpoint task exited unexpectedly"),
                Err(ref e) => log::error!("Checkpoint task panicked: {e}"),
            }
            let err_msg = format!("Checkpoint task exited: {res:?}");
            shutdown_daemon(&mut upload_handle, &mut checkpoint_handle, &db, config.paths.db_file.clone()).await;
            return Err(err_msg.into());
        }
    }
    shutdown_daemon(
        &mut upload_handle,
        &mut checkpoint_handle,
        &db,
        config.paths.db_file.clone(),
    )
    .await;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

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
