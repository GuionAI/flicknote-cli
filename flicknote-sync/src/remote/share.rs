use crate::*;

#[cfg(test)]
mod tests;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ShareResource {
    Note,
    Project,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ShareRequest {
    pub(crate) resource: ShareResource,
    pub(crate) id: String,
}

#[derive(Deserialize)]
pub(crate) struct ShareResponse {
    pub(crate) url: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ShareApiError {
    pub(crate) error_code: Option<String>,
    pub(crate) message: Option<String>,
}

#[derive(Default)]
pub(crate) struct ShareRequestLock {
    pub(crate) mutex: tokio::sync::Mutex<()>,
}

impl ShareRequestLock {
    pub(crate) async fn run<T>(&self, operation: impl Future<Output = T>) -> T {
        let _guard = self.mutex.lock().await;
        operation.await
    }
}

impl ShareResource {
    fn path_segment(self) -> &'static str {
        match self {
            Self::Note => "notes",
            Self::Project => "projects",
        }
    }

    fn missing_error_code(self) -> &'static str {
        match self {
            Self::Note => "SHARE_NOT_FOUND",
            Self::Project => "PROJECT_SHARE_NOT_FOUND",
        }
    }
}

pub(crate) fn share_endpoint(api_url: &str, request: &ShareRequest) -> String {
    let versioned_base = api_url
        .trim_end_matches('/')
        .trim_end_matches("/api/v1")
        .trim_end_matches('/');
    format!(
        "{versioned_base}/api/v1/{}/{}/share",
        request.resource.path_segment(),
        request.id
    )
}

pub(crate) fn share_api_error(status: reqwest::StatusCode, body: String) -> DaemonError {
    let message = serde_json::from_str::<ShareApiError>(&body)
        .ok()
        .and_then(|error| error.message)
        .unwrap_or(body);
    DaemonError::Other {
        message: format!("Share API returned {status}: {message}"),
    }
}

pub(crate) async fn parse_share_url(response: reqwest::Response) -> Result<String, DaemonError> {
    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(share_api_error(status, body));
    }
    response
        .json::<ShareResponse>()
        .await
        .map(|share| share.url)
        .map_err(|error| DaemonError::Other {
            message: format!("Failed to parse share API response: {error}"),
        })
}

pub(crate) async fn get_or_create_share_with_token(
    http: &reqwest::Client,
    config: &Config,
    access_token: &str,
    request: &ShareRequest,
) -> Result<String, DaemonError> {
    validate_api_url(config)?;
    let endpoint = share_endpoint(&config.api_url, request);
    let response = http
        .get(&endpoint)
        .bearer_auth(access_token)
        .send()
        .await
        .map_err(|error| DaemonError::Other {
            message: format!("Share request failed: {error}"),
        })?;

    if response.status().is_success() {
        return parse_share_url(response).await;
    }

    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    let is_missing_share = status == reqwest::StatusCode::NOT_FOUND
        && serde_json::from_str::<ShareApiError>(&body)
            .ok()
            .and_then(|error| error.error_code)
            .is_some_and(|code| code == request.resource.missing_error_code());
    if !is_missing_share {
        return Err(share_api_error(status, body));
    }

    let response = http
        .post(endpoint)
        .bearer_auth(access_token)
        .json(&serde_json::json!({}))
        .send()
        .await
        .map_err(|error| DaemonError::Other {
            message: format!("Share create request failed: {error}"),
        })?;
    parse_share_url(response).await
}

pub(crate) async fn revoke_share_with_token(
    http: &reqwest::Client,
    config: &Config,
    access_token: &str,
    request: &ShareRequest,
) -> Result<(), DaemonError> {
    validate_api_url(config)?;
    let response = http
        .delete(share_endpoint(&config.api_url, request))
        .bearer_auth(access_token)
        .send()
        .await
        .map_err(|error| DaemonError::Other {
            message: format!("Share revoke request failed: {error}"),
        })?;
    let status = response.status();
    if status.is_success() {
        return Ok(());
    }
    let body = response.text().await.unwrap_or_default();
    Err(share_api_error(status, body))
}

pub(crate) async fn get_or_create_share(
    http: &reqwest::Client,
    auth: &GoTrueClient,
    config: &Config,
    request: &ShareRequest,
) -> Result<String, DaemonError> {
    let session = auth
        .get_session()
        .await
        .map_err(|error| DaemonError::Other {
            message: format!("Auth error: {error}"),
        })?;
    get_or_create_share_with_token(http, config, &session.access_token, request).await
}

pub(crate) async fn revoke_share(
    http: &reqwest::Client,
    auth: &GoTrueClient,
    config: &Config,
    request: &ShareRequest,
) -> Result<(), DaemonError> {
    let session = auth
        .get_session()
        .await
        .map_err(|error| DaemonError::Other {
            message: format!("Auth error: {error}"),
        })?;
    revoke_share_with_token(http, config, &session.access_token, request).await
}

pub(crate) struct RemoteShareGateway {
    pub(crate) http: reqwest::Client,
    pub(crate) auth: Arc<GoTrueClient>,
    pub(crate) config: Arc<Config>,
    pub(crate) lock: Arc<ShareRequestLock>,
}

#[async_trait]
impl ShareGateway for RemoteShareGateway {
    async fn share(
        &self,
        resource: CoreShareResource,
        id: &str,
    ) -> Result<String, flicknote_core::services::error::ServiceError> {
        let request = ShareRequest {
            resource: match resource {
                CoreShareResource::Note => ShareResource::Note,
                CoreShareResource::Project => ShareResource::Project,
            },
            id: id.to_string(),
        };
        self.lock
            .run(get_or_create_share(
                &self.http,
                &self.auth,
                &self.config,
                &request,
            ))
            .await
            .map_err(|error| {
                flicknote_core::services::error::ServiceError::Daemon(error.to_string())
            })
    }

    async fn unshare(
        &self,
        resource: CoreShareResource,
        id: &str,
    ) -> Result<(), flicknote_core::services::error::ServiceError> {
        let request = ShareRequest {
            resource: match resource {
                CoreShareResource::Note => ShareResource::Note,
                CoreShareResource::Project => ShareResource::Project,
            },
            id: id.to_string(),
        };
        self.lock
            .run(revoke_share(&self.http, &self.auth, &self.config, &request))
            .await
            .map_err(|error| {
                flicknote_core::services::error::ServiceError::Daemon(error.to_string())
            })
    }
}
