use super::*;

const IPC_CONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(1);
const IPC_WRITE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(1);
const IPC_HEALTH_RESPONSE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(1);
const IPC_APP_RESPONSE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(300);

pub fn socket_path(config: &Config) -> PathBuf {
    config.paths.data_dir.join("daemon.sock")
}

pub(crate) fn unavailable(path: &std::path::Path, stage: &str) -> DaemonError {
    DaemonError::Unavailable {
        path: path.display().to_string(),
        message: format!("timed out while {stage}"),
    }
}

pub(crate) fn request_timeout_error(
    request: &DaemonRequest,
    path: &std::path::Path,
    stage: &str,
) -> DaemonError {
    if !is_mutating_app_request(request) {
        return unavailable(path, stage);
    }
    DaemonError::Other {
        message: format!(
            "Timed out while {stage}; the application request outcome is unknown. Do not retry it automatically."
        ),
    }
}

pub(crate) fn is_mutating_app_request(request: &DaemonRequest) -> bool {
    matches!(request, DaemonRequest::App { request, .. } if request.may_write())
}

pub(crate) fn response_timeout_for(request: &DaemonRequest) -> Option<std::time::Duration> {
    match request {
        DaemonRequest::Health { .. } => Some(IPC_HEALTH_RESPONSE_TIMEOUT),
        // Once a write request may have reached the daemon, a transport timeout cannot tell
        // whether it committed. Keep waiting for the authoritative response until the protocol
        // has durable operation IDs and status reconciliation (tracked as FlickNote #1785).
        DaemonRequest::App { request, .. } if request.may_write() => None,
        DaemonRequest::App { .. } => Some(IPC_APP_RESPONSE_TIMEOUT),
    }
}

pub async fn send_request(
    config: &Config,
    request: &DaemonRequest,
) -> Result<DaemonResponse, DaemonError> {
    let path = socket_path(config);
    let request_bytes = serde_json::to_vec(request).map_err(|error| DaemonError::Other {
        message: format!("Failed to serialize daemon request: {error}"),
    })?;
    let mut stream = tokio::time::timeout(IPC_CONNECT_TIMEOUT, UnixStream::connect(&path))
        .await
        .map_err(|_| unavailable(&path, "connecting"))?
        .map_err(|error| DaemonError::Unavailable {
            path: path.display().to_string(),
            message: error.to_string(),
        })?;
    let write_request = async {
        stream.write_all(&request_bytes).await?;
        stream.shutdown().await
    };
    if is_mutating_app_request(request) {
        write_request
            .await
            .map_err(|error| DaemonError::PostConnectTransport {
                message: format!("Failed to send daemon request: {error}"),
            })?;
    } else {
        tokio::time::timeout(IPC_WRITE_TIMEOUT, write_request)
            .await
            .map_err(|_| request_timeout_error(request, &path, "sending a request"))?
            .map_err(|error| DaemonError::PostConnectTransport {
                message: format!("Failed to send daemon request: {error}"),
            })?;
    }
    let mut buf = Vec::new();
    match response_timeout_for(request) {
        Some(response_timeout) => {
            tokio::time::timeout(response_timeout, stream.read_to_end(&mut buf))
                .await
                .map_err(|_| request_timeout_error(request, &path, "waiting for a response"))?
        }
        None => stream.read_to_end(&mut buf).await,
    }
    .map_err(|error| DaemonError::PostConnectTransport {
        message: format!("Failed to read daemon response: {error}"),
    })?;
    serde_json::from_slice(&buf).map_err(|error| {
        if error.is_eof() {
            return DaemonError::IncompleteResponse {
                message: format!(
                    "Daemon closed the connection before a complete response: {error}"
                ),
            };
        }
        match serde_json::from_slice::<serde_json::Value>(&buf) {
            Ok(_) => DaemonError::InvalidResponse {
                message: format!("Daemon returned an incompatible response: {error}"),
            },
            Err(raw_error) => DaemonError::MalformedResponse {
                message: format!("Daemon returned a malformed response: {raw_error}"),
            },
        }
    })
}

pub struct DaemonClient<'a> {
    config: &'a Config,
}

impl<'a> DaemonClient<'a> {
    pub fn new(config: &'a Config) -> Self {
        Self { config }
    }

    async fn request(&self, request: DaemonRequest) -> Result<DaemonResponse, ServiceError> {
        let is_mutating = is_mutating_app_request(&request);
        send_request(self.config, &request)
            .await
            .map_err(|error| match error {
                DaemonError::Unavailable { .. } => ServiceError::DaemonUnavailable(
                    "Check `flicknote daemon status` and start it with `flicknote daemon start`."
                        .to_string(),
                ),
                DaemonError::IncompleteResponse { .. }
                | DaemonError::MalformedResponse { .. }
                | DaemonError::PostConnectTransport { .. }
                    if !is_mutating =>
                {
                    ServiceError::DaemonUnavailable(
                        "The FlickNote daemon is not ready. Check `flicknote daemon status` and start it with `flicknote daemon start`."
                            .to_string(),
                    )
                }
                DaemonError::IncompleteResponse { message }
                | DaemonError::MalformedResponse { message }
                | DaemonError::PostConnectTransport { message }
                | DaemonError::InvalidResponse { message }
                    if is_mutating =>
                {
                    Self::outcome_unknown(message)
                }
                DaemonError::InvalidResponse { .. } => Self::protocol_mismatch(None),
                other => ServiceError::Daemon(other.to_string()),
            })
    }

    pub async fn health(&self) -> Result<ServerInfo, ServiceError> {
        match self
            .request(DaemonRequest::Health {
                protocol: PROTOCOL_VERSION,
            })
            .await?
        {
            DaemonResponse::ServerInfo(info) if info.protocol == PROTOCOL_VERSION => Ok(info),
            DaemonResponse::ServerInfo(info) => Err(Self::protocol_mismatch(Some(&info))),
            DaemonResponse::AppError(error) if error.code == PROTOCOL_MISMATCH_CODE => {
                Err(Self::protocol_mismatch_from_details(error.details.as_ref()))
            }
            DaemonResponse::AppError(error) => Err(Self::remote_error(error)),
            _ => Err(Self::protocol_mismatch(None)),
        }
    }

    pub async fn app(&self, request: AppRequest) -> Result<AppResponse, ServiceError> {
        let may_write = request.may_write();
        match self
            .request(DaemonRequest::App {
                protocol: PROTOCOL_VERSION,
                request: Box::new(request),
            })
            .await?
        {
            DaemonResponse::App(response) => Ok(*response),
            DaemonResponse::AppError(error) => Err(Self::remote_error(error)),
            _ if may_write => Err(Self::outcome_unknown(
                "The daemon returned an unexpected envelope after a mutating request; the operation outcome is unknown."
                    .to_string(),
            )),
            _ => Err(Self::protocol_mismatch(None)),
        }
    }

    pub async fn call<T: AppResult>(&self, request: AppRequest) -> Result<T, ServiceError> {
        let may_write = request.may_write();
        let response = self.app(request).await?;
        T::from_response(response).ok_or_else(|| {
            if may_write {
                Self::outcome_unknown(
                    "The daemon returned an unexpected response after a mutating request; the operation outcome is unknown.".to_string(),
                )
            } else {
                Self::protocol_mismatch(None)
            }
        })
    }

    fn remote_error(error: WireError) -> ServiceError {
        ServiceError::Remote {
            code: error.code,
            message: error.message,
            retryable: error.retryable,
            details: error.details,
        }
    }

    fn protocol_mismatch(info: Option<&ServerInfo>) -> ServiceError {
        let details = info.map(|info| {
            serde_json::json!({
                "daemon_executable": info.executable,
                "daemon_version": info.version,
                "daemon_protocol": info.protocol,
            })
        });
        Self::protocol_mismatch_from_details(details.as_ref())
    }

    fn protocol_mismatch_from_details(details: Option<&serde_json::Value>) -> ServiceError {
        let daemon_executable = details
            .and_then(|value| value.get("daemon_executable"))
            .and_then(serde_json::Value::as_str);
        let daemon_version = details
            .and_then(|value| value.get("daemon_version"))
            .and_then(serde_json::Value::as_str);
        let daemon_protocol = details
            .and_then(|value| value.get("daemon_protocol"))
            .and_then(serde_json::Value::as_u64);
        let daemon_diagnostics = match (daemon_executable, daemon_version, daemon_protocol) {
            (Some(executable), Some(version), Some(protocol)) => format!(
                "daemon executable {executable}, daemon version {version} protocol {protocol}"
            ),
            _ => "daemon executable/version/protocol unavailable".to_string(),
        };
        let message = format!(
            "The running FlickNote daemon is incompatible: CLI version {} protocol {}; {daemon_diagnostics}. Restart it with `flicknote daemon restart`.",
            env!("CARGO_PKG_VERSION"),
            PROTOCOL_VERSION,
        );
        ServiceError::Remote {
            code: PROTOCOL_MISMATCH_CODE.to_string(),
            message,
            retryable: false,
            details: Some(serde_json::json!({
                "cli_version": env!("CARGO_PKG_VERSION"),
                "cli_protocol": PROTOCOL_VERSION,
                "daemon_executable": daemon_executable,
                "daemon_version": daemon_version,
                "daemon_protocol": daemon_protocol,
            })),
        }
    }

    fn outcome_unknown(message: String) -> ServiceError {
        ServiceError::Remote {
            code: "daemon_request_outcome_unknown".to_string(),
            message,
            retryable: false,
            details: None,
        }
    }
}
