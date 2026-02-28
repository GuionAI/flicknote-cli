use flicknote_auth::session::load_session;
use flicknote_core::config::Config;
use flicknote_core::error::CliError;
use futures_util::StreamExt;
use reqwest::Client;
use serde::Deserialize;
use tokio::io::AsyncWriteExt;

pub struct ApiClient {
    http: Client,
    base_url: String,
    access_token: String,
}

#[derive(Debug, Deserialize)]
pub struct ApiResponse {
    pub id: Option<String>,
    pub message: Option<String>,
}

impl ApiClient {
    pub fn new(config: &Config) -> Result<Self, CliError> {
        config.validate_api()?;
        let session =
            load_session(&config.paths.session_file).map_err(|_| CliError::NotAuthenticated)?;

        Ok(Self {
            http: Client::new(),
            base_url: config.api_url.trim_end_matches('/').to_string(),
            access_token: session.access_token,
        })
    }

    pub async fn upload_file(&self, file_path: &std::path::Path) -> Result<ApiResponse, CliError> {
        let file_name = file_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("file")
            .to_string();

        let file_bytes = std::fs::read(file_path)?;
        let mime = mime_from_extension(file_path);

        let part = reqwest::multipart::Part::bytes(file_bytes)
            .file_name(file_name)
            .mime_str(&mime)
            .map_err(|e| CliError::Other(format!("Invalid MIME type: {e}")))?;

        let form = reqwest::multipart::Form::new().part("file", part);

        let resp = self
            .http
            .post(format!("{}/api/upload", self.base_url))
            .bearer_auth(&self.access_token)
            .multipart(form)
            .send()
            .await
            .map_err(|e| CliError::Other(format!("Upload request failed: {e}")))?;

        parse_response(resp).await
    }

    /// Downloads an attachment, streaming it to the given output path.
    /// Returns the number of bytes written.
    pub async fn download_attachment(
        &self,
        attachment_id: &str,
        output_path: &std::path::Path,
    ) -> Result<u64, CliError> {
        let resp = self
            .http
            .get(format!(
                "{}/api/attachments/{}/download",
                self.base_url, attachment_id
            ))
            .bearer_auth(&self.access_token)
            .send()
            .await
            .map_err(|e| CliError::Other(format!("Download request failed: {e}")))?;

        if !resp.status().is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(CliError::Other(format!("Download failed: {body}")));
        }

        let mut file = tokio::fs::File::create(output_path)
            .await
            .map_err(|e| CliError::Other(format!("Failed to create output file: {e}")))?;

        let mut stream = resp.bytes_stream();
        let mut total: u64 = 0;
        while let Some(chunk) = stream.next().await {
            let chunk =
                chunk.map_err(|e| CliError::Other(format!("Error reading response: {e}")))?;
            file.write_all(&chunk)
                .await
                .map_err(|e| CliError::Other(format!("Error writing file: {e}")))?;
            total += chunk.len() as u64;
        }

        file.flush()
            .await
            .map_err(|e| CliError::Other(format!("Error flushing file: {e}")))?;

        Ok(total)
    }

    /// Fetches the original filename from the download endpoint's Content-Disposition header.
    pub async fn get_attachment_filename(
        &self,
        attachment_id: &str,
    ) -> Result<Option<String>, CliError> {
        let resp = self
            .http
            .head(format!(
                "{}/api/attachments/{}/download",
                self.base_url, attachment_id
            ))
            .bearer_auth(&self.access_token)
            .send()
            .await
            .map_err(|e| CliError::Other(format!("HEAD request failed: {e}")))?;

        Ok(parse_content_disposition_filename(&resp))
    }

    pub async fn delete_attachment(&self, attachment_id: &str) -> Result<ApiResponse, CliError> {
        let resp = self
            .http
            .delete(format!(
                "{}/api/attachments/{}",
                self.base_url, attachment_id
            ))
            .bearer_auth(&self.access_token)
            .send()
            .await
            .map_err(|e| CliError::Other(format!("Delete request failed: {e}")))?;

        parse_response(resp).await
    }

    pub async fn create_link(
        &self,
        url: &str,
        title: Option<&str>,
    ) -> Result<ApiResponse, CliError> {
        let mut body = serde_json::json!({ "url": url });
        if let Some(t) = title {
            body["title"] = serde_json::Value::String(t.to_string());
        }

        let resp = self
            .http
            .post(format!("{}/api/links", self.base_url))
            .bearer_auth(&self.access_token)
            .json(&body)
            .send()
            .await
            .map_err(|e| CliError::Other(format!("Link request failed: {e}")))?;

        parse_response(resp).await
    }

    pub async fn create_note(
        &self,
        content: &str,
        title: Option<&str>,
    ) -> Result<ApiResponse, CliError> {
        let mut body = serde_json::json!({ "content": content });
        if let Some(t) = title {
            body["title"] = serde_json::Value::String(t.to_string());
        }

        let resp = self
            .http
            .post(format!("{}/api/notes", self.base_url))
            .bearer_auth(&self.access_token)
            .json(&body)
            .send()
            .await
            .map_err(|e| CliError::Other(format!("Note request failed: {e}")))?;

        parse_response(resp).await
    }
}

async fn parse_response(resp: reqwest::Response) -> Result<ApiResponse, CliError> {
    if !resp.status().is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(CliError::Other(format!("API error: {body}")));
    }
    resp.json::<ApiResponse>()
        .await
        .map_err(|e| CliError::Other(format!("Failed to parse API response: {e}")))
}

fn mime_from_extension(path: &std::path::Path) -> String {
    mime_guess::from_path(path)
        .first_or_octet_stream()
        .to_string()
}

fn parse_content_disposition_filename(resp: &reqwest::Response) -> Option<String> {
    let header = resp.headers().get("content-disposition")?.to_str().ok()?;
    // Parse: attachment; filename="name.ext" or filename=name.ext
    for part in header.split(';') {
        let part = part.trim();
        if let Some(rest) = part.strip_prefix("filename=") {
            let name = rest.trim_matches('"').trim();
            if !name.is_empty() {
                return Some(name.to_string());
            }
        }
    }
    None
}
