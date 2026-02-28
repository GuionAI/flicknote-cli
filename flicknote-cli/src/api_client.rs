use flicknote_auth::client::GoTrueClient;
use flicknote_core::config::Config;
use flicknote_core::error::CliError;
use reqwest::Client;
use serde::Deserialize;

pub struct ApiClient {
    http: Client,
    base_url: String,
    access_token: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UploadUrlResponse {
    pub upload_url: String,
    pub content_type: String,
}

#[derive(Debug, Deserialize)]
pub struct DownloadUrlResponse {
    pub url: String,
}

#[derive(Debug, Deserialize)]
pub struct DeleteResponse {
    pub deleted: bool,
}

impl ApiClient {
    /// Create API client with a fresh (auto-refreshed) token.
    /// Uses GoTrueClient::get_session() which refreshes if near expiry.
    pub async fn new(config: &Config) -> Result<Self, CliError> {
        config.validate_api()?;
        let auth = GoTrueClient::new(
            &config.supabase_url,
            &config.supabase_anon_key,
            &config.paths.session_file,
        );
        let session = auth
            .get_session()
            .await
            .map_err(|e| CliError::Other(format!("Auth error: {e}")))?;

        Ok(Self {
            http: Client::new(),
            base_url: config.api_url.trim_end_matches('/').to_string(),
            access_token: session.access_token,
        })
    }

    /// Step 1: Get presigned upload URL from API
    /// Step 2: PUT file directly to R2 via presigned URL
    pub async fn upload_file(
        &self,
        note_id: &str,
        file_path: &std::path::Path,
    ) -> Result<(), CliError> {
        let filename = file_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("file")
            .to_string();

        // Get presigned upload URL
        let resp = self
            .http
            .post(format!("{}/api/v1/attachments/upload-url", self.base_url))
            .bearer_auth(&self.access_token)
            .json(&serde_json::json!({ "noteId": note_id, "filename": filename }))
            .send()
            .await
            .map_err(|e| CliError::Other(format!("Upload URL request failed: {e}")))?;

        if !resp.status().is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(CliError::Other(format!("Upload URL request failed: {body}")));
        }

        let upload_resp: UploadUrlResponse = resp
            .json()
            .await
            .map_err(|e| CliError::Other(format!("Failed to parse upload URL response: {e}")))?;

        // Upload file directly to R2
        let file_bytes = std::fs::read(file_path)?;
        let put_resp = self
            .http
            .put(&upload_resp.upload_url)
            .header("Content-Type", &upload_resp.content_type)
            .body(file_bytes)
            .send()
            .await
            .map_err(|e| CliError::Other(format!("File upload failed: {e}")))?;

        if !put_resp.status().is_success() {
            let body = put_resp.text().await.unwrap_or_default();
            return Err(CliError::Other(format!("File upload to R2 failed: {body}")));
        }

        Ok(())
    }

    /// Get presigned download URL, then download file content
    pub async fn download_attachment(
        &self,
        note_id: &str,
        output_path: &std::path::Path,
    ) -> Result<u64, CliError> {
        let resp = self
            .http
            .post(format!(
                "{}/api/v1/attachments/download-url",
                self.base_url
            ))
            .bearer_auth(&self.access_token)
            .json(&serde_json::json!({ "noteId": note_id }))
            .send()
            .await
            .map_err(|e| CliError::Other(format!("Download URL request failed: {e}")))?;

        if !resp.status().is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(CliError::Other(format!(
                "Download URL request failed: {body}"
            )));
        }

        let download_resp: DownloadUrlResponse = resp.json().await.map_err(|e| {
            CliError::Other(format!("Failed to parse download URL response: {e}"))
        })?;

        // Download from R2 presigned URL
        let file_resp = self
            .http
            .get(&download_resp.url)
            .send()
            .await
            .map_err(|e| CliError::Other(format!("File download failed: {e}")))?;

        if !file_resp.status().is_success() {
            return Err(CliError::Other(format!(
                "File download failed: HTTP {}",
                file_resp.status()
            )));
        }

        let bytes = file_resp
            .bytes()
            .await
            .map_err(|e| CliError::Other(format!("Error reading response: {e}")))?;

        std::fs::write(output_path, &bytes)?;
        Ok(bytes.len() as u64)
    }

    /// DELETE /api/v1/attachments/:noteId
    pub async fn delete_attachment(&self, note_id: &str) -> Result<DeleteResponse, CliError> {
        let resp = self
            .http
            .delete(format!(
                "{}/api/v1/attachments/{}",
                self.base_url, note_id
            ))
            .bearer_auth(&self.access_token)
            .send()
            .await
            .map_err(|e| CliError::Other(format!("Delete request failed: {e}")))?;

        if !resp.status().is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(CliError::Other(format!("Delete failed: {body}")));
        }

        resp.json::<DeleteResponse>()
            .await
            .map_err(|e| CliError::Other(format!("Failed to parse delete response: {e}")))
    }
}
