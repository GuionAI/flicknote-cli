use std::io;
use std::sync::Arc;

use axum::Router;
use rmcp::transport::streamable_http_server::{
    StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
};
use tokio::net::TcpListener;
use tokio::sync::watch;

use super::FlickNoteMcp;
use crate::app::Application;

const MCP_PORT: u16 = 37789;
const MCP_PORT_ENV: &str = "FLICKNOTE_MCP_PORT";

pub async fn bind() -> io::Result<TcpListener> {
    let port = match std::env::var(MCP_PORT_ENV) {
        Ok(value) => value.parse::<u16>().map_err(|error| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("{MCP_PORT_ENV}: {error}"),
            )
        })?,
        Err(std::env::VarError::NotPresent) => MCP_PORT,
        Err(error) => return Err(io::Error::new(io::ErrorKind::InvalidInput, error)),
    };
    TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port)).await
}

pub async fn serve(
    listener: TcpListener,
    app: Arc<Application>,
    mut shutdown: watch::Receiver<bool>,
) -> io::Result<()> {
    let port = listener.local_addr()?.port();
    let allowed_origins = [
        format!("http://127.0.0.1:{port}"),
        format!("http://localhost:{port}"),
    ];
    let config = StreamableHttpServerConfig::default().with_allowed_origins(allowed_origins);
    let cancellation = config.cancellation_token.clone();
    let service: StreamableHttpService<FlickNoteMcp, LocalSessionManager> =
        StreamableHttpService::new(
            move || Ok(FlickNoteMcp::new(Arc::clone(&app))),
            Default::default(),
            config,
        );
    let router = Router::new().nest_service("/mcp", service);
    axum::serve(listener, router)
        .with_graceful_shutdown(async move {
            if !*shutdown.borrow() && shutdown.changed().await.is_err() {
                log::debug!("MCP shutdown sender was dropped");
            }
            cancellation.cancel();
        })
        .await
}
