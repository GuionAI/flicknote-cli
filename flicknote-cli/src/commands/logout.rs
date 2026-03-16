use flicknote_core::config::Config;
use flicknote_core::error::CliError;
use std::fs;

pub(crate) fn run(config: &Config) -> Result<(), CliError> {
    if !config.paths.session_file.exists() {
        println!("Already logged out");
        return Ok(());
    }

    // 1. Stop the sync daemon (silently succeeds if not running)
    super::daemon::stop(config)?;

    // 2. Uninstall the launchd service
    super::daemon::uninstall()?;

    // 3. Delete local DB files — collect errors so session is always cleared
    let db_base = config.paths.db_file.with_extension("");
    let mut db_errors: Vec<String> = Vec::new();
    for ext in ["db", "db-shm", "db-wal"] {
        let path = db_base.with_extension(ext);
        if path.exists()
            && let Err(e) = fs::remove_file(&path)
        {
            db_errors.push(format!("{}: {e}", path.display()));
        }
    }

    // 4. Delete session file regardless of DB deletion failures
    fs::remove_file(&config.paths.session_file)?;

    if !db_errors.is_empty() {
        return Err(CliError::Other(format!(
            "Logged out but some local data could not be deleted: {}",
            db_errors.join(", ")
        )));
    }

    println!("Logged out (session, daemon, and local data cleared)");
    Ok(())
}
