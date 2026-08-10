use async_trait::async_trait;
use powersync::{BackendConnector, PowerSyncCredentials, error::PowerSyncError};

use crate::upload::{FlickNoteConnector, ps_err, run_upload};

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
        let token = self.get_token().await?;
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
