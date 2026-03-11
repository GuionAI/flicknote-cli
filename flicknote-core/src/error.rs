use thiserror::Error;

#[derive(Debug, Error)]
pub enum CliError {
    #[error("Not authenticated — run `flicknote login`")]
    NotAuthenticated,

    #[error("Note not found: {id}")]
    NoteNotFound { id: String },

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

    #[error("Postgres error: {0}")]
    Postgres(#[from] postgres::Error),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("Hook rejected: {message}")]
    HookRejected { message: String },

    #[error("{0}")]
    Other(String),
}
