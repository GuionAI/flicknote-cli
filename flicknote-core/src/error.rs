use thiserror::Error;

#[derive(Debug, Error)]
pub enum CliError {
    #[error("Not authenticated — run `flicknote login`")]
    NotAuthenticated,

    #[error("Note not found: {id}")]
    NoteNotFound { id: String },

    #[error(
        "project \"{name}\" not found — create it first with: flicknote project add \"{name}\""
    )]
    ProjectNotFound { name: String },

    #[error("project \"{name}\" already exists")]
    ProjectAlreadyExists { name: String },

    #[error("Auth {operation} failed: {description}")]
    Auth {
        operation: String,
        description: String,
    },

    #[cfg(feature = "powersync")]
    #[error("PowerSync error: {0}")]
    PowerSync(#[from] powersync::error::PowerSyncError),

    #[cfg(feature = "powersync")]
    #[error("Database error: {0}")]
    Sqlite(#[from] rusqlite::Error),

    #[cfg(feature = "storage-pgwire")]
    #[error("Database error: {0}")]
    Database(String),

    #[cfg(feature = "storage-pgwire")]
    #[error("Database error: {}", format_pg_err(.0))]
    Pg(#[from] postgres::Error),

    #[error("HTTP error: {0}")]
    Http(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("{0}")]
    Other(String),
}

#[cfg(feature = "storage-pgwire")]
pub(crate) fn format_pg_err(e: &postgres::Error) -> String {
    e.as_db_error()
        .map(std::string::ToString::to_string)
        .unwrap_or_else(|| e.to_string())
}
