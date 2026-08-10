use std::fmt;
use std::path::Path;

/// WAL checkpoint mode passed to [`checkpoint_wal_standalone`].
#[derive(Clone, Copy)]
pub(crate) enum WalCheckpointMode {
    /// Checkpoints frames up to the oldest active reader's mark. Never acquires
    /// PENDING or EXCLUSIVE locks — returns immediately. Safe at any time alongside
    /// active pool connections. Returns `busy=1` when readers constrain the
    /// checkpoint to an earlier WAL position (normal during runtime).
    Passive,
    /// Acquires a PENDING lock while waiting for readers to finish, then resets
    /// the WAL to zero length. Use only when no pool connections exist (startup,
    /// shutdown) to avoid the PENDING lock blocking pool writers.
    Truncate,
}

impl fmt::Display for WalCheckpointMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Passive => write!(f, "PASSIVE"),
            Self::Truncate => write!(f, "TRUNCATE"),
        }
    }
}

/// Run a WAL checkpoint using a standalone rusqlite connection.
///
/// Opens its own connection to the DB file, bypassing PowerSync's writer mutex
/// entirely — competes only at the SQLite file-lock level, not the Rust mutex level.
///
/// `mode` controls the checkpoint type — see [`WalCheckpointMode`] for semantics.
///
/// `busy_timeout` is set to 5 000 ms for TRUNCATE so it retries at the SQLite level
/// while pool readers finish their short transactions. It is irrelevant for PASSIVE
/// (which never waits) but harmless to keep set.
///
/// Reads the `(busy, log, checkpointed)` return tuple from PRAGMA so failures
/// are never silently swallowed. For PASSIVE, `busy=1` when active readers
/// constrain the checkpoint to an earlier WAL position (normal and expected during
/// runtime). For TRUNCATE, `busy=1` means the reset could not complete.
///
/// This function is **synchronous** (blocking rusqlite I/O). Async callers must
/// wrap it with `tokio::task::spawn_blocking`.
///
/// `label` identifies the call site in log output (e.g. `"startup"`, `"post-upload"`,
/// `"periodic"`, `"shutdown"`) so production logs are unambiguous.
pub(crate) fn checkpoint_wal_standalone(db_path: &Path, label: &str, mode: WalCheckpointMode) {
    let conn = match rusqlite::Connection::open(db_path) {
        Ok(c) => c,
        Err(e) => {
            log::warn!("WAL checkpoint [{label}]: could not open db: {e}");
            return;
        }
    };
    if let Err(e) = conn.pragma_update(None, "busy_timeout", 5_000i64) {
        log::warn!("WAL checkpoint [{label}]: could not set busy_timeout: {e}");
        return;
    }
    let pragma = format!("PRAGMA wal_checkpoint({})", mode);
    match conn.query_row(&pragma, [], |row| {
        Ok((
            row.get::<_, i32>(0)?,
            row.get::<_, i32>(1)?,
            row.get::<_, i32>(2)?,
        ))
    }) {
        Ok((busy, log, checkpointed)) => {
            if busy == 0 {
                log::info!(
                    "WAL checkpoint [{label}] ({mode}): {log} pages, {checkpointed} checkpointed"
                );
            } else {
                log::warn!(
                    "WAL checkpoint [{label}]: incomplete (busy={busy}, {log} log pages, {checkpointed} checkpointed)"
                );
            }
        }
        Err(e) => log::warn!("WAL checkpoint [{label}]: failed: {e}"),
    }
    // Connection dropped here — no persistent state
}
