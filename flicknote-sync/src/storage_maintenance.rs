use std::fmt;
use std::path::Path;

/// WAL checkpoint mode passed to [`checkpoint_wal_standalone_with_timeout`].
#[derive(Clone, Copy)]
pub(crate) enum WalCheckpointMode {
    /// Checkpoints frames up to the oldest active reader's mark without waiting.
    Passive,
    /// Resets the WAL to zero length after readers finish.
    Truncate,
}

impl fmt::Display for WalCheckpointMode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Passive => formatter.write_str("PASSIVE"),
            Self::Truncate => formatter.write_str("TRUNCATE"),
        }
    }
}

/// Run a WAL checkpoint with an explicit SQLite busy timeout.
///
/// The synchronous operation is called from a detached worker thread by async
/// callers. The shutdown coordinator uses a short timeout so a busy database
/// cannot hold process exit open.
pub(crate) fn checkpoint_wal_standalone_with_timeout(
    db_path: &Path,
    label: &str,
    mode: WalCheckpointMode,
    busy_timeout_ms: u64,
) {
    let conn = match rusqlite::Connection::open(db_path) {
        Ok(connection) => connection,
        Err(error) => {
            log::warn!("WAL checkpoint [{label}]: could not open db: {error}");
            return;
        }
    };
    if let Err(error) = conn.pragma_update(None, "busy_timeout", busy_timeout_ms as i64) {
        log::warn!("WAL checkpoint [{label}]: could not set busy_timeout: {error}");
        return;
    }
    let pragma = format!("PRAGMA wal_checkpoint({mode})");
    match conn.query_row(&pragma, [], |row| {
        Ok((
            row.get::<_, i32>(0)?,
            row.get::<_, i32>(1)?,
            row.get::<_, i32>(2)?,
        ))
    }) {
        Ok((0, log_pages, checkpointed)) => {
            log::info!(
                "WAL checkpoint [{label}] ({mode}): {log_pages} pages, {checkpointed} checkpointed"
            );
        }
        Ok((busy, log_pages, checkpointed)) => {
            log::warn!(
                "WAL checkpoint [{label}]: incomplete (busy={busy}, {log_pages} log pages, {checkpointed} checkpointed)"
            );
        }
        Err(error) => log::warn!("WAL checkpoint [{label}]: failed: {error}"),
    }
}
