//! Process-wide SQLite extensions registered before opening a PowerSync pool.

use rusqlite::ffi;

unsafe extern "C" {
    fn sqlite3_bettertrigram_init(
        db: *mut ffi::sqlite3,
        error: *mut *mut std::ffi::c_char,
        api: *const ffi::sqlite3_api_routines,
    ) -> std::ffi::c_int;
}

/// Register the statically linked tokenizer for every subsequent SQLite connection.
/// SQLite deduplicates repeated registration of the same entry point.
pub fn register_better_trigram() -> Result<(), rusqlite::Error> {
    // SAFETY: The linked C entry point has SQLite's extension-init ABI. SQLite
    // retains only its function pointer, whose code lives for the process lifetime.
    let code = unsafe { ffi::sqlite3_auto_extension(Some(sqlite3_bettertrigram_init)) };
    if code == ffi::SQLITE_OK {
        Ok(())
    } else {
        Err(rusqlite::Error::SqliteFailure(
            ffi::Error::new(code),
            Some("register better-trigram auto-extension".into()),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registration_reaches_new_connections_and_survives_reopen() {
        register_better_trigram().unwrap();
        register_better_trigram().unwrap();
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("search.db");
        {
            let db = rusqlite::Connection::open(&path).unwrap();
            db.execute_batch("CREATE VIRTUAL TABLE search USING fts5(text, tokenize='better_trigram'); INSERT INTO search(text) VALUES ('中文 A bird');").unwrap();
        }
        for _ in 0..3 {
            let db = rusqlite::Connection::open(&path).unwrap();
            let count: i64 = db
                .query_row(
                    "SELECT count(*) FROM search WHERE search MATCH ?",
                    ["中"],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(count, 1);
        }
    }
}
