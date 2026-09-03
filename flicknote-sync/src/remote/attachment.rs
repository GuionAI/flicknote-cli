use std::path::Path;

use flicknote_core::config::Config;
use serde::Deserialize;

use crate::ipc::DaemonError;

fn attachment_endpoint(base_url: &str, path: &str) -> String {
    let versioned_base = base_url
        .trim_end_matches('/')
        .trim_end_matches("/api/v1")
        .trim_end_matches('/');
    let path = path.trim_matches('/');
    format!("{versioned_base}/api/v1/attachments/{path}")
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UploadUrlResponse {
    upload_url: String,
    content_type: String,
}

pub(super) fn validate_gateway_url(config: &Config) -> Result<(), DaemonError> {
    if config.gateway_url.is_empty() {
        return Err(DaemonError::Other {
            message:
                "gatewayUrl is not configured — set it in config.json or FLICKNOTE_GATEWAY_URL"
                    .to_string(),
        });
    }
    Ok(())
}

pub(super) async fn upload_attachment(
    http: &reqwest::Client,
    config: &Config,
    access_token: &str,
    note_id: &str,
    file_path: &Path,
) -> Result<(), DaemonError> {
    validate_gateway_url(config)?;
    let filename = file_path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| DaemonError::Other {
            message: "Invalid filename".to_string(),
        })?
        .to_string();

    let resp = http
        .post(attachment_endpoint(&config.gateway_url, "upload-url"))
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

    let file_bytes = tokio::fs::read(file_path)
        .await
        .map_err(|e| DaemonError::Other {
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

pub(super) async fn delete_attachment(
    http: &reqwest::Client,
    config: &Config,
    access_token: &str,
    note_id: &str,
) -> Result<(), DaemonError> {
    validate_gateway_url(config)?;
    let resp = http
        .delete(attachment_endpoint(&config.gateway_url, note_id))
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

#[cfg(test)]
mod tests {
    use super::*;
    use flicknote_core::config::{Config, ConfigPaths};

    #[test]
    fn attachment_requests_use_the_gateway_url() {
        let config = Config {
            supabase_url: String::new(),
            supabase_anon_key: String::new(),
            powersync_url: String::new(),
            api_url: "https://api.example.test/api/v1".to_string(),
            gateway_url: "https://gateway.example.test".to_string(),
            web_url: None,
            paths: ConfigPaths {
                config_dir: Default::default(),
                data_dir: Default::default(),
                config_file: Default::default(),
                session_file: Default::default(),
                db_file: Default::default(),
                log_file: Default::default(),
            },
        };

        assert_eq!(
            attachment_endpoint(&config.gateway_url, "upload-url"),
            "https://gateway.example.test/api/v1/attachments/upload-url"
        );
    }
}
