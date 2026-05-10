use powersync::env::PowerSyncEnvironment;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePool, SqlitePoolOptions};

use crate::config::Config;
use crate::schema::app_schema;

pub struct Database {
    pub pool: SqlitePool,
}

impl Database {
    /// Open the database for local-only use (no sync connection).
    pub async fn open_local(config: &Config) -> Result<Self, crate::error::CliError> {
        PowerSyncEnvironment::powersync_auto_extension().map_err(|e| {
            crate::error::CliError::Other(format!("Failed to load PowerSync extension: {e}"))
        })?;

        let options = SqliteConnectOptions::new()
            .filename(&config.paths.db_file)
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal)
            .pragma("journal_size_limit", "6291456")
            .pragma("busy_timeout", "30000")
            .pragma("cache_size", "51200");

        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await?;

        sqlx::query("SELECT powersync_update_hooks('install')")
            .execute(&pool)
            .await?;
        sqlx::query("SELECT powersync_init()")
            .execute(&pool)
            .await?;

        let schema = serde_json::to_string(&app_schema())?;
        sqlx::query("SELECT powersync_replace_schema(?)")
            .bind(schema)
            .execute(&pool)
            .await?;

        Ok(Self { pool })
    }
}
