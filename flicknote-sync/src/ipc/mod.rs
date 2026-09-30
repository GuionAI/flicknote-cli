//! Daemon-side Unix socket server; shared protocol and client live in flicknote-client.
use crate::app::Application;
use flicknote_client::{
    DaemonError, DaemonRequest, DaemonResponse, PROTOCOL_MISMATCH_CODE, PROTOCOL_VERSION,
    PowerSyncErrors, ServerInfo, WireError,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};

mod server;
pub use server::{
    ServerInfoProvider, read_request, serve_app, serve_app_once, serve_app_until_with_provider,
    write_response,
};

pub fn socket_path(config: &flicknote_core::config::Config) -> std::path::PathBuf {
    config.paths.data_dir.join("daemon.sock")
}

pub fn server_info() -> ServerInfo {
    ServerInfo {
        protocol: PROTOCOL_VERSION,
        version: env!("CARGO_PKG_VERSION").to_string(),
        executable: current_executable(),
        sync: None,
        sync_errors: PowerSyncErrors::default(),
        search: None,
    }
}

fn current_executable() -> String {
    std::env::current_exe()
        .ok()
        .or_else(|| std::env::args_os().next().map(std::path::PathBuf::from))
        .map_or_else(
            || "unavailable".to_string(),
            |path| path.display().to_string(),
        )
}
