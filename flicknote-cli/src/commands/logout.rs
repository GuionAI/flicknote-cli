use flicknote_core::config::Config;
use flicknote_core::error::CliError;
use std::fs;

pub(crate) async fn run(config: &Config) -> Result<(), CliError> {
    if !config.paths.session_file.exists() {
        println!("Already logged out");
        return Ok(());
    }

    let running = super::sync::running_server_info(config).await?;
    let manages_local_daemon = manages_local_daemon_for(running.as_ref().map(|info| info.backend));
    if manages_local_daemon {
        super::daemon::stop(config)?;
        super::daemon::uninstall()?;
    }

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

    if manages_local_daemon {
        println!("Logged out (session, daemon, and local data cleared)");
    } else {
        println!("Logged out (local session and data cleared; managed daemon left running)");
    }
    Ok(())
}

const fn manages_local_daemon_for(
    running_backend: Option<flicknote_sync::ipc::BackendMode>,
) -> bool {
    !matches!(
        running_backend,
        Some(flicknote_sync::ipc::BackendMode::Managed)
    )
}

#[cfg(test)]
mod tests {
    use flicknote_sync::ipc::BackendMode;

    #[test]
    fn logout_never_manages_a_live_managed_daemon() {
        assert!(!super::manages_local_daemon_for(Some(BackendMode::Managed)));
        assert!(super::manages_local_daemon_for(Some(BackendMode::Local)));
        assert!(super::manages_local_daemon_for(None));
    }
}
