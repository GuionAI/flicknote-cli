use super::*;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::watch;
use tokio::task::JoinSet;

pub type ServerInfoProvider = Arc<dyn Fn() -> ServerInfo + Send + Sync>;

pub async fn read_request(stream: &mut UnixStream) -> Result<DaemonRequest, DaemonError> {
    let mut buf = Vec::new();
    stream
        .read_to_end(&mut buf)
        .await
        .map_err(|error| DaemonError::Other {
            message: format!("Failed to read daemon request: {error}"),
        })?;
    serde_json::from_slice(&buf).map_err(|error| DaemonError::Other {
        message: format!("Failed to parse daemon request: {error}"),
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
    app: Arc<Application>,
    info: ServerInfo,
) -> Result<(), DaemonError> {
    let (mut stream, _) = listener
        .accept()
        .await
        .map_err(|error| DaemonError::Other {
            message: format!("Failed to accept daemon request: {error}"),
        })?;
    let provider = static_info_provider(info);
    serve_app_stream(&mut stream, &app, &provider).await
}

pub async fn serve_app(
    listener: UnixListener,
    app: Arc<Application>,
    info: ServerInfo,
) -> Result<(), DaemonError> {
    let (_shutdown_tx, shutdown_rx) = watch::channel(false);
    serve_app_until_with_provider(listener, app, static_info_provider(info), shutdown_rx).await
}

pub async fn serve_app_until_with_provider(
    listener: UnixListener,
    app: Arc<Application>,
    provider: ServerInfoProvider,
    mut shutdown: watch::Receiver<bool>,
) -> Result<(), DaemonError> {
    let mut requests = JoinSet::new();
    loop {
        tokio::select! {
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() {
                    break;
                }
            }
            accepted = listener.accept() => {
                let (mut stream, _) = accepted.map_err(|error| DaemonError::Other {
                    message: format!("Failed to accept daemon request: {error}"),
                })?;
                let app = Arc::clone(&app);
                let provider = Arc::clone(&provider);
                requests.spawn(async move {
                    if let Err(error) = serve_app_stream(&mut stream, &app, &provider).await {
                        log::warn!("application IPC request failed: {error}");
                    }
                });
            }
            Some(result) = requests.join_next() => {
                if let Err(error) = result {
                    log::warn!("application IPC task failed: {error}");
                }
            }
        }
    }

    let drain = async {
        while let Some(result) = requests.join_next().await {
            if let Err(error) = result {
                log::warn!("application IPC task failed while draining: {error}");
            }
        }
    };
    if tokio::time::timeout(Duration::from_secs(2), drain)
        .await
        .is_err()
    {
        log::warn!("IPC in-flight request drain exceeded 2s");
        requests.abort_all();
        while requests.join_next().await.is_some() {}
    }
    Ok(())
}

async fn serve_app_stream(
    stream: &mut UnixStream,
    app: &Application,
    provider: &ServerInfoProvider,
) -> Result<(), DaemonError> {
    let response = match read_request(stream).await? {
        DaemonRequest::Health { protocol } if protocol == PROTOCOL_VERSION => {
            DaemonResponse::ServerInfo(provider())
        }
        DaemonRequest::App { protocol, request } if protocol == PROTOCOL_VERSION => {
            match app.handle(*request).await {
                Ok(response) => DaemonResponse::App(Box::new(response)),
                Err(error) => DaemonResponse::AppError(error),
            }
        }
        DaemonRequest::Health { protocol } | DaemonRequest::App { protocol, .. } => {
            let info = ServerInfo::current();
            DaemonResponse::AppError(WireError {
                code: PROTOCOL_MISMATCH_CODE.to_string(),
                message: format!(
                    "FlickNote daemon version {} uses protocol {PROTOCOL_VERSION}; client sent protocol {protocol}",
                    env!("CARGO_PKG_VERSION")
                ),
                retryable: false,
                details: Some(serde_json::json!({
                    "daemon_executable": info.executable,
                    "daemon_version": info.version,
                    "daemon_protocol": info.protocol,
                    "client_protocol": protocol,
                })),
            })
        }
    };
    write_response(stream, &response).await
}

fn static_info_provider(info: ServerInfo) -> ServerInfoProvider {
    Arc::new(move || info.clone())
}

pub(crate) async fn write_json<T: Serialize + Sync>(
    stream: &mut UnixStream,
    value: &T,
) -> Result<(), DaemonError> {
    let bytes = serde_json::to_vec(value).map_err(|error| DaemonError::Other {
        message: format!("Failed to serialize daemon message: {error}"),
    })?;
    stream
        .write_all(&bytes)
        .await
        .map_err(|error| DaemonError::Other {
            message: format!("Failed to write daemon message: {error}"),
        })?;
    stream.shutdown().await.map_err(|error| DaemonError::Other {
        message: format!("Failed to close daemon message: {error}"),
    })
}
