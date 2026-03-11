use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use flicknote_auth::client::GoTrueClient;
use flicknote_core::{config::Config, schema::app_schema};
use futures_lite::StreamExt;
use http_client::isahc::IsahcClient;
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

struct FlickNoteConnector {
    db: PowerSyncDatabase,
    auth: GoTrueClient,
    upload_guard: Arc<tokio::sync::Mutex<()>>,
    http_client: reqwest::Client,
    powersync_url: String,
    supabase_url: String,
    supabase_anon_key: String,
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
        for crud in std::mem::take(&mut tx.crud) {
            let table = &crud.table;
            let id = &crud.id;

            match crud.update_type {
                UpdateType::Put => {
                    let mut data = crud.data.unwrap_or_default();
                    data.insert("id".into(), serde_json::Value::String(id.clone()));
                    let resp = client
                        .post(format!("{supabase_url}/rest/v1/{table}"))
                        .header("apikey", supabase_anon_key)
                        .header("Authorization", format!("Bearer {token}"))
                        .header("Prefer", "resolution=merge-duplicates")
                        .json(&data)
                        .send()
                        .await
                        .map_err(|e| ps_err(format!("Upload PUT failed: {e}")))?;
                    if !resp.status().is_success() {
                        let body = resp
                            .text()
                            .await
                            .unwrap_or_else(|e| format!("<body read error: {e}>"));
                        return Err(ps_err(format!("PUT {table}/{id} failed: {body}")));
                    }
                }
                UpdateType::Patch => {
                    let data = crud.data.unwrap_or_default();
                    let resp = client
                        .patch(format!("{supabase_url}/rest/v1/{table}?id=eq.{id}"))
                        .header("apikey", supabase_anon_key)
                        .header("Authorization", format!("Bearer {token}"))
                        .json(&data)
                        .send()
                        .await
                        .map_err(|e| ps_err(format!("Upload PATCH failed: {e}")))?;
                    if !resp.status().is_success() {
                        let body = resp
                            .text()
                            .await
                            .unwrap_or_else(|e| format!("<body read error: {e}>"));
                        return Err(ps_err(format!("PATCH {table}/{id} failed: {body}")));
                    }
                }
                UpdateType::Delete => {
                    let resp = client
                        .delete(format!("{supabase_url}/rest/v1/{table}?id=eq.{id}"))
                        .header("apikey", supabase_anon_key)
                        .header("Authorization", format!("Bearer {token}"))
                        .send()
                        .await
                        .map_err(|e| ps_err(format!("Upload DELETE failed: {e}")))?;
                    if !resp.status().is_success() {
                        let body = resp
                            .text()
                            .await
                            .unwrap_or_else(|e| format!("<body read error: {e}>"));
                        return Err(ps_err(format!("DELETE {table}/{id} failed: {body}")));
                    }
                }
            }
        }
        // Complete each transaction individually so successfully-uploaded entries
        // are removed from ps_crud before processing the next batch. Without this,
        // a mid-batch failure would re-upload all prior entries on the next cycle,
        // causing phantom DELETEs (404) and duplicate PUTs.
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
    let client = Arc::new(IsahcClient::new());
    let env =
        PowerSyncEnvironment::custom(client, pool, Box::new(PowerSyncEnvironment::tokio_timer()));

    let db = PowerSyncDatabase::new(env, app_schema());
    db.async_tasks().spawn_with_tokio();

    let auth = GoTrueClient::new(
        &config.supabase_url,
        &config.supabase_anon_key,
        &config.paths.session_file,
    );

    let upload_guard = Arc::new(tokio::sync::Mutex::new(()));
    let http_client = reqwest::Client::new();
    let poll_client = http_client.clone();

    let connector = FlickNoteConnector {
        db: db.clone(),
        auth,
        upload_guard: Arc::clone(&upload_guard),
        http_client,
        powersync_url: config.powersync_url.clone(),
        supabase_url: config.supabase_url.clone(),
        supabase_anon_key: config.supabase_anon_key.clone(),
    };

    let poll_db = db.clone();
    let poll_supabase_url = config.supabase_url.clone();
    let poll_anon_key = config.supabase_anon_key.clone();
    let poll_session_file = config.paths.session_file.clone();
    let poll_guard = Arc::clone(&upload_guard);

    log::info!("Sync daemon connecting (pid {})", std::process::id());
    db.connect(SyncOptions::new(connector)).await;
    log::info!("Sync daemon connected (pid {})", std::process::id());

    let poll_auth = GoTrueClient::new(
        &config.supabase_url,
        &config.supabase_anon_key,
        &poll_session_file,
    );
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
