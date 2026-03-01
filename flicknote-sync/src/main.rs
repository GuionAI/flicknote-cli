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
    powersync_url: String,
    supabase_url: String,
    supabase_anon_key: String,
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
        let client = reqwest::Client::new();
        let mut transactions = self.db.crud_transactions();
        let mut last_tx = None;

        while let Some(mut tx) = transactions.try_next().await? {
            let token = self.get_token().await?;

            for crud in std::mem::take(&mut tx.crud) {
                let table = &crud.table;
                let id = &crud.id;

                match crud.update_type {
                    UpdateType::Put => {
                        let mut data = crud.data.unwrap_or_default();
                        data.insert("id".into(), serde_json::Value::String(id.clone()));
                        let resp = client
                            .post(format!("{}/rest/v1/{table}", self.supabase_url))
                            .header("apikey", &self.supabase_anon_key)
                            .header("Authorization", format!("Bearer {token}"))
                            .header("Prefer", "resolution=merge-duplicates")
                            .json(&data)
                            .send()
                            .await
                            .map_err(|e| ps_err(format!("Upload PUT failed: {e}")))?;
                        if !resp.status().is_success() {
                            let body = resp.text().await.unwrap_or_default();
                            return Err(ps_err(format!("PUT {table}/{id} failed: {body}")));
                        }
                    }
                    UpdateType::Patch => {
                        let data = crud.data.unwrap_or_default();
                        let resp = client
                            .patch(format!("{}/rest/v1/{table}?id=eq.{id}", self.supabase_url))
                            .header("apikey", &self.supabase_anon_key)
                            .header("Authorization", format!("Bearer {token}"))
                            .json(&data)
                            .send()
                            .await
                            .map_err(|e| ps_err(format!("Upload PATCH failed: {e}")))?;
                        if !resp.status().is_success() {
                            let body = resp.text().await.unwrap_or_default();
                            return Err(ps_err(format!("PATCH {table}/{id} failed: {body}")));
                        }
                    }
                    UpdateType::Delete => {
                        let resp = client
                            .delete(format!("{}/rest/v1/{table}?id=eq.{id}", self.supabase_url))
                            .header("apikey", &self.supabase_anon_key)
                            .header("Authorization", format!("Bearer {token}"))
                            .send()
                            .await
                            .map_err(|e| ps_err(format!("Upload DELETE failed: {e}")))?;
                        if !resp.status().is_success() {
                            let body = resp.text().await.unwrap_or_default();
                            return Err(ps_err(format!("DELETE {table}/{id} failed: {body}")));
                        }
                    }
                }
            }
            last_tx = Some(tx);
        }

        if let Some(tx) = last_tx {
            tx.complete().await?;
        }

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

    let connector = FlickNoteConnector {
        db: db.clone(),
        auth,
        powersync_url: config.powersync_url.clone(),
        supabase_url: config.supabase_url.clone(),
        supabase_anon_key: config.supabase_anon_key.clone(),
    };

    log::info!("Sync daemon connecting (pid {})", std::process::id());
    db.connect(SyncOptions::new(connector)).await;
    log::info!("Sync daemon connected (pid {})", std::process::id());

    tokio::signal::ctrl_c().await?;
    db.disconnect().await;
    log::info!("Sync daemon stopped");

    Ok(())
}
