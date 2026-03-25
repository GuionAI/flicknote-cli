use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use flicknote_auth::client::GoTrueClient;
use flicknote_core::{config::Config, schema::app_schema};
use futures_lite::StreamExt;
mod http_adapter;
use http_adapter::ReqwestHttpClient;
use powersync::{
    BackendConnector, ConnectionPool, PowerSyncCredentials, PowerSyncDatabase, SyncOptions,
    UpdateType, env::PowerSyncEnvironment, error::PowerSyncError,
};

/// How often the polling loop checks for pending CRUD entries from the CLI process.
/// 5s balances latency against unnecessary auth round-trips on idle daemons.
const POLL_INTERVAL_SECS: u64 = 5;

/// After this many consecutive poll failures, escalate from warn to error.
const FAILURE_ESCALATE_THRESHOLD: u32 = 3;

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

/// Inner upload logic shared by the BackendConnector impl and the polling loop.
/// Caller is responsible for holding `upload_guard` before calling.
///
/// The token is fetched once per call by the caller. Supabase tokens are typically
/// valid for 1 hour, so any realistic upload batch completes well within the window.
async fn run_upload(
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
            continue; // next transaction
        }
        if let Some(msg) = transient_msg {
            return Err(ps_err(msg)); // retry on next poll cycle
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
        run_upload(
            &self.db,
            &self.http_client,
            &token,
            &self.supabase_url,
            &self.supabase_anon_key,
        )
        .await
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
    let poll_client = http_client.clone();

    let connector = FlickNoteConnector {
        db: db.clone(),
        auth: Arc::clone(&auth),
        upload_guard: Arc::clone(&upload_guard),
        http_client,
        powersync_url: config.powersync_url.clone(),
        supabase_url: config.supabase_url.clone(),
        supabase_anon_key: config.supabase_anon_key.clone(),
    };

    let poll_db = db.clone();
    let poll_supabase_url = config.supabase_url.clone();
    let poll_anon_key = config.supabase_anon_key.clone();
    let poll_guard = Arc::clone(&upload_guard);

    log::info!("Sync daemon connecting (pid {})", std::process::id());
    db.connect(SyncOptions::new(connector)).await;
    log::info!("Sync daemon connected (pid {})", std::process::id());

    let poll_auth = Arc::clone(&auth);
    let mut poll_handle = tokio::spawn(async move {
        let mut consecutive_failures: u32 = 0;
        // Skip first tick — Trigger B from stream establishment handles existing entries.
        let mut interval =
            tokio::time::interval(std::time::Duration::from_secs(POLL_INTERVAL_SECS));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        interval.tick().await;
        loop {
            interval.tick().await;
            let _guard = poll_guard.lock().await;
            let token = match poll_auth.get_session().await {
                Ok(s) => s.access_token,
                Err(e) => {
                    consecutive_failures += 1;
                    if consecutive_failures >= FAILURE_ESCALATE_THRESHOLD {
                        log::error!("Polling upload: auth error ({consecutive_failures}x): {e}");
                    } else {
                        log::warn!("Polling upload: auth error: {e}");
                    }
                    continue;
                }
            };
            if let Err(e) = run_upload(
                &poll_db,
                &poll_client,
                &token,
                &poll_supabase_url,
                &poll_anon_key,
            )
            .await
            {
                consecutive_failures += 1;
                if consecutive_failures >= FAILURE_ESCALATE_THRESHOLD {
                    log::error!("Polling upload failed ({consecutive_failures}x): {e}");
                } else {
                    log::warn!("Polling upload failed: {e}");
                }
            } else {
                consecutive_failures = 0;
                log::debug!("Polling upload: completed");
            }
        }
    });

    tokio::select! {
        _ = tokio::signal::ctrl_c() => {}
        res = &mut poll_handle => {
            let Err(e) = res;
            log::error!("Poll task exited unexpectedly: {e}");
        }
    }
    poll_handle.abort();
    db.disconnect().await;
    log::info!("Sync daemon stopped");

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
