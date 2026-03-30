use async_trait::async_trait;
use powersync::{
    ConnectionPool, PowerSyncDatabase,
    env::PowerSyncEnvironment,
    error::PowerSyncError,
    http::{HttpClient, Request, Response},
};

use crate::config::Config;
use crate::schema::app_schema;

/// A no-op HTTP client for local-only PowerSync use (no sync connection).
#[derive(Debug)]
struct NoopHttpClient;

#[async_trait]
impl HttpClient for NoopHttpClient {
    async fn send(&self, _req: Request) -> Result<Response, PowerSyncError> {
        Err(std::io::Error::other("local-only mode: no HTTP client").into())
    }
}

pub struct Database {
    pub db: PowerSyncDatabase,
    pub rt: tokio::runtime::Runtime,
}

impl Database {
    /// Open the database for local-only use (no sync connection).
    pub fn open_local(config: &Config) -> Result<Self, crate::error::CliError> {
        PowerSyncEnvironment::powersync_auto_extension().map_err(|e| {
            crate::error::CliError::Other(format!("Failed to load PowerSync extension: {e}"))
        })?;

        let pool = ConnectionPool::open(&config.paths.db_file)?;

        let env =
            PowerSyncEnvironment::custom(NoopHttpClient, pool, PowerSyncEnvironment::tokio_timer());

        let schema = app_schema();
        let db = PowerSyncDatabase::new(env, schema);

        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()?;
        db.async_tasks().spawn_with_tokio_runtime(&rt);

        Ok(Self { db, rt })
    }

    /// Run a read-only closure against the database.
    pub fn read<T, F>(&self, f: F) -> Result<T, crate::error::CliError>
    where
        F: FnOnce(&rusqlite::Connection) -> Result<T, crate::error::CliError>,
    {
        let reader = self.rt.block_on(self.db.reader())?;
        f(&reader)
    }

    /// Run a read-write closure against the database.
    pub fn write<T, F>(&self, f: F) -> Result<T, crate::error::CliError>
    where
        F: FnOnce(&rusqlite::Connection) -> Result<T, crate::error::CliError>,
    {
        let writer = self.rt.block_on(self.db.writer())?;
        f(&writer)
    }
}
