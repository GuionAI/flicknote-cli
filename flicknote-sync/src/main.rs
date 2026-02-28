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

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init();

    let config = Config::load()?;
    config.validate()?;

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
