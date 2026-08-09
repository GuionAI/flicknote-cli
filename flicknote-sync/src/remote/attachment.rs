use crate::*;

pub(crate) fn attachment_endpoint(base_url: &str, path: &str) -> String {
    let versioned_base = base_url
        .trim_end_matches('/')
        .trim_end_matches("/api/v1")
        .trim_end_matches('/');
    let path = path.trim_matches('/');
    format!("{versioned_base}/api/v1/attachments/{path}")
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UploadUrlResponse {
    pub(crate) upload_url: String,
    pub(crate) content_type: String,
}

pub(crate) fn validate_api_url(config: &Config) -> Result<(), DaemonError> {
    if config.api_url.is_empty() {
        return Err(DaemonError::Other {
            message: "apiUrl is not configured — set it in config.json or FLICKNOTE_API_URL"
                .to_string(),
        });
    }
    Ok(())
}

pub(crate) async fn upload_attachment(
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

pub(crate) async fn delete_attachment(
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
