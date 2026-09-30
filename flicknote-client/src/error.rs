//! Client-facing failures without backend implementations.
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ClientError {
    #[error("FlickNote daemon is unavailable: {0}")]
    DaemonUnavailable(String),
    #[error("FlickNote daemon request failed: {0}")]
    Daemon(String),
    #[error("{message}")]
    Remote {
        code: String,
        message: String,
        retryable: bool,
        details: Option<serde_json::Value>,
    },
}

impl ClientError {
    pub fn code(&self) -> &str {
        match self {
            Self::DaemonUnavailable(_) => "daemon_unavailable",
            Self::Daemon(_) => "daemon_error",
            Self::Remote { code, .. } => code,
        }
    }

    pub const fn retryable(&self) -> bool {
        match self {
            Self::DaemonUnavailable(_) => true,
            Self::Daemon(_) => false,
            Self::Remote { retryable, .. } => *retryable,
        }
    }
}
