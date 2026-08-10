use super::*;

pub async fn read_request(stream: &mut UnixStream) -> Result<DaemonRequest, DaemonError> {
    let mut buf = Vec::new();
    stream
        .read_to_end(&mut buf)
        .await
        .map_err(|e| DaemonError::Other {
            message: format!("Failed to read daemon request: {e}"),
        })?;
    serde_json::from_slice(&buf).map_err(|e| DaemonError::Other {
        message: format!("Failed to parse daemon request: {e}"),
    })
}

pub async fn write_response(
    stream: &mut UnixStream,
    response: &DaemonResponse,
) -> Result<(), DaemonError> {
    write_json(stream, response).await
}

pub async fn serve_app_once(
    listener: UnixListener,
    app: std::sync::Arc<Application>,
    info: ServerInfo,
) -> Result<(), DaemonError> {
    let (mut stream, _) = listener
        .accept()
        .await
        .map_err(|error| DaemonError::Other {
            message: format!("Failed to accept daemon request: {error}"),
        })?;
    serve_app_stream(&mut stream, &app, &info).await
}

pub async fn serve_app(
    listener: UnixListener,
    app: std::sync::Arc<Application>,
    info: ServerInfo,
) -> Result<(), DaemonError> {
    loop {
        let (mut stream, _) = listener
            .accept()
            .await
            .map_err(|error| DaemonError::Other {
                message: format!("Failed to accept daemon request: {error}"),
            })?;
        let app = std::sync::Arc::clone(&app);
        let info = info.clone();
        tokio::spawn(async move {
            if let Err(error) = serve_app_stream(&mut stream, &app, &info).await {
                log::warn!("application IPC request failed: {error}");
            }
        });
    }
}

pub(crate) async fn serve_app_stream(
    stream: &mut UnixStream,
    app: &Application,
    info: &ServerInfo,
) -> Result<(), DaemonError> {
    let response = match read_request(stream).await? {
        DaemonRequest::Health { protocol } if protocol == PROTOCOL_VERSION => {
            DaemonResponse::ServerInfo(info.clone())
        }
        DaemonRequest::App { protocol, request } if protocol == PROTOCOL_VERSION => {
            match app.handle(*request).await {
                Ok(response) => DaemonResponse::App(Box::new(response)),
                Err(error) => DaemonResponse::AppError(error),
            }
        }
        DaemonRequest::Health { protocol } | DaemonRequest::App { protocol, .. } => {
            DaemonResponse::AppError(WireError {
                code: "daemon_protocol_mismatch".to_string(),
                message: format!(
                    "daemon protocol {PROTOCOL_VERSION} does not support client protocol {protocol}"
                ),
                retryable: false,
                details: None,
            })
        }
    };
    write_response(stream, &response).await
}

pub(crate) async fn write_json<T: Serialize + Sync>(
    stream: &mut UnixStream,
    value: &T,
) -> Result<(), DaemonError> {
    let bytes = serde_json::to_vec(value).map_err(|e| DaemonError::Other {
        message: format!("Failed to serialize daemon message: {e}"),
    })?;
    stream
        .write_all(&bytes)
        .await
        .map_err(|e| DaemonError::Other {
            message: format!("Failed to write daemon message: {e}"),
        })?;
    stream.shutdown().await.map_err(|e| DaemonError::Other {
        message: format!("Failed to close daemon message: {e}"),
    })
}
