use crate::*;

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
