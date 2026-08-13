use thiserror::Error;

use crate::error::CliError;

#[derive(Debug, Error)]
pub enum ServiceError {
    #[error("{0}")]
    InvalidArgument(String),
    #[error("Note not found: {0}")]
    NoteNotFound(String),
    #[error("Project not found: {0}")]
    ProjectNotFound(String),
    #[error("Section not found: {0}")]
    SectionNotFound(String),
    #[error("{message}")]
    BeforeNotFound { message: String },
    #[error("{message}")]
    BeforeAmbiguous { matches: usize, message: String },
    #[error("Note has no text content")]
    NoTextContent,
    #[error("Note has no source data")]
    NoSource,
    #[error("Nothing to modify")]
    NothingToModify,
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
    #[error("Missing configuration: {0}")]
    ConfigMissing(String),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Internal error: {0}")]
    Internal(String),
    #[error(transparent)]
    Backend(CliError),
}

impl ServiceError {
    pub fn code(&self) -> &str {
        match self {
            Self::InvalidArgument(_) => "invalid_argument",
            Self::NoteNotFound(_) => "note_not_found",
            Self::ProjectNotFound(_) => "project_not_found",
            Self::SectionNotFound(_) => "section_not_found",
            Self::BeforeNotFound { .. } => "before_not_found",
            Self::BeforeAmbiguous { .. } => "before_ambiguous",
            Self::NoTextContent => "no_text_content",
            Self::NoSource => "no_source",
            Self::NothingToModify => "nothing_to_modify",
            Self::DaemonUnavailable(_) => "daemon_unavailable",
            Self::Daemon(_) => "daemon_error",
            Self::Remote { code, .. } => code,
            Self::ConfigMissing(_) => "config_missing",
            Self::Io(_) => "io_error",
            Self::Internal(_) | Self::Backend(_) => "internal_error",
        }
    }

    pub const fn retryable(&self) -> bool {
        match self {
            Self::DaemonUnavailable(_) => true,
            Self::Remote { retryable, .. } => *retryable,
            _ => false,
        }
    }
}

impl From<CliError> for ServiceError {
    fn from(error: CliError) -> Self {
        match error {
            CliError::NoteNotFound { id } => Self::NoteNotFound(id),
            CliError::ProjectNotFound { name } => Self::ProjectNotFound(name),
            CliError::Io(error) => Self::Io(error),
            other => Self::Backend(other),
        }
    }
}

impl From<ServiceError> for CliError {
    fn from(error: ServiceError) -> Self {
        match error {
            ServiceError::Backend(error) => error,
            ServiceError::Io(error) => Self::Io(error),
            other => Self::Other(other.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::ServiceError;

    #[test]
    fn service_errors_expose_stable_codes_and_retryability() {
        let unavailable = ServiceError::DaemonUnavailable("socket missing".to_string());
        assert_eq!(unavailable.code(), "daemon_unavailable");
        assert!(unavailable.retryable());

        let invalid = ServiceError::InvalidArgument("bad range".to_string());
        assert_eq!(invalid.code(), "invalid_argument");
        assert!(!invalid.retryable());
    }
}
