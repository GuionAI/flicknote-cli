use super::daemon_lifecycle::{DaemonLifecycle, NativeDaemonLifecycle, ServiceCleanup};
use clap::Args;
use flicknote_core::config::Config;
use flicknote_core::error::CliError;
use std::fs;

#[derive(Args)]
pub(crate) struct LogoutArgs {
    /// Continue clearing credentials and local data if service cleanup cannot be confirmed
    #[arg(long)]
    force: bool,
}

pub(crate) async fn run(config: &Config, args: &LogoutArgs) -> Result<(), CliError> {
    run_with_lifecycle(config, args, &NativeDaemonLifecycle).await
}

async fn run_with_lifecycle(
    config: &Config,
    args: &LogoutArgs,
    lifecycle: &dyn DaemonLifecycle,
) -> Result<(), CliError> {
    let session_exists = config.paths.session_file.exists();
    let service_cleanup = lifecycle.uninstall(config).await;
    if !session_exists && matches!(service_cleanup, Ok(ServiceCleanup::NotInstalled)) {
        println!("Already logged out");
        return Ok(());
    }
    let service_error = service_cleanup.err();
    if let Some(error) = &service_error {
        if !args.force {
            return Err(CliError::Other(format!(
                "Could not confirm daemon cleanup: {error}. Session and local data were retained; use `flicknote logout --force` only after reviewing `flicknote daemon status --verbose`"
            )));
        }
        eprintln!("Warning: daemon cleanup was not confirmed: {error}");
    }

    let db_base = config.paths.db_file.with_extension("");
    let mut data_errors = Vec::new();
    for extension in ["db", "db-shm", "db-wal"] {
        let path = db_base.with_extension(extension);
        if path.exists()
            && let Err(error) = fs::remove_file(&path)
        {
            data_errors.push(format!("{}: {error}", path.display()));
        }
    }
    if session_exists {
        fs::remove_file(&config.paths.session_file)?;
    }

    if service_error.is_some() || !data_errors.is_empty() {
        let mut problems = data_errors;
        if service_error.is_some() {
            problems.push("daemon service cleanup remains unresolved".to_string());
        }
        return Err(CliError::Other(format!(
            "Logged out, but cleanup needs attention: {}",
            problems.join(", ")
        )));
    }

    println!("Logged out (daemon, session, and local data cleared)");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use std::sync::Mutex;

    struct FakeLifecycle {
        result: Mutex<Option<Result<ServiceCleanup, CliError>>>,
    }

    #[async_trait]
    impl DaemonLifecycle for FakeLifecycle {
        async fn install_and_wait(&self, _config: &Config) -> Result<(), CliError> {
            Ok(())
        }

        async fn uninstall(&self, _config: &Config) -> Result<ServiceCleanup, CliError> {
            self.result
                .lock()
                .unwrap()
                .take()
                .unwrap_or(Ok(ServiceCleanup::NotInstalled))
        }
    }

    fn config(directory: &std::path::Path) -> Config {
        Config {
            supabase_url: String::new(),
            supabase_anon_key: String::new(),
            powersync_url: String::new(),
            api_url: String::new(),
            web_url: None,
            paths: flicknote_core::config::ConfigPaths {
                config_dir: directory.to_path_buf(),
                data_dir: directory.to_path_buf(),
                config_file: directory.join("config.json"),
                session_file: directory.join("session.json"),
                db_file: directory.join("flicknote.db"),
                log_file: directory.join("log"),
            },
        }
    }

    struct OrderingLifecycle {
        session_file: std::path::PathBuf,
        db_file: std::path::PathBuf,
    }

    #[async_trait]
    impl DaemonLifecycle for OrderingLifecycle {
        async fn install_and_wait(&self, _config: &Config) -> Result<(), CliError> {
            Ok(())
        }

        async fn uninstall(&self, _config: &Config) -> Result<ServiceCleanup, CliError> {
            assert!(self.session_file.exists());
            assert!(self.db_file.exists());
            Ok(ServiceCleanup::Removed)
        }
    }

    #[tokio::test]
    async fn successful_logout_removes_service_before_session_and_local_data() {
        let directory = tempfile::tempdir().unwrap();
        let config = config(directory.path());
        std::fs::write(&config.paths.session_file, "session").unwrap();
        std::fs::write(&config.paths.db_file, "data").unwrap();
        let lifecycle = OrderingLifecycle {
            session_file: config.paths.session_file.clone(),
            db_file: config.paths.db_file.clone(),
        };

        run_with_lifecycle(&config, &LogoutArgs { force: false }, &lifecycle)
            .await
            .unwrap();

        assert!(!config.paths.session_file.exists());
        assert!(!config.paths.db_file.exists());
    }

    #[tokio::test]
    async fn logout_without_session_still_cleans_data_after_service_cleanup() {
        let directory = tempfile::tempdir().unwrap();
        let config = config(directory.path());
        std::fs::write(&config.paths.db_file, "stale").unwrap();
        let lifecycle = FakeLifecycle {
            result: Mutex::new(Some(Ok(ServiceCleanup::Removed))),
        };
        let args = LogoutArgs { force: false };

        run_with_lifecycle(&config, &args, &lifecycle)
            .await
            .unwrap();

        assert!(!config.paths.db_file.exists());
    }

    #[tokio::test]
    async fn normal_logout_retains_session_when_service_cleanup_fails() {
        let directory = tempfile::tempdir().unwrap();
        let config = config(directory.path());
        std::fs::write(&config.paths.session_file, "session").unwrap();
        let lifecycle = FakeLifecycle {
            result: Mutex::new(Some(Err(CliError::Other("stop failed".to_string())))),
        };
        let args = LogoutArgs { force: false };
        let result = run_with_lifecycle(&config, &args, &lifecycle).await;
        assert!(result.is_err());
        assert!(config.paths.session_file.exists());
    }

    #[tokio::test]
    async fn forced_logout_clears_session_after_service_cleanup_failure() {
        let directory = tempfile::tempdir().unwrap();
        let config = config(directory.path());
        std::fs::write(&config.paths.session_file, "session").unwrap();
        let lifecycle = FakeLifecycle {
            result: Mutex::new(Some(Err(CliError::Other("stop failed".to_string())))),
        };
        let args = LogoutArgs { force: true };
        let result = run_with_lifecycle(&config, &args, &lifecycle).await;
        assert!(result.is_err());
        assert!(!config.paths.session_file.exists());
    }
}
