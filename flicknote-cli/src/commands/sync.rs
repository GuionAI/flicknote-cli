use clap::{Args, Subcommand};
use flicknote_core::config::Config;
use flicknote_core::error::CliError;
use std::fs;
use std::path::Path;

#[derive(Args)]
pub(crate) struct SyncArgs {
    #[command(subcommand)]
    command: SyncCommand,
}

#[derive(Subcommand)]
enum SyncCommand {
    /// Start the FlickNote daemon in background
    Start,
    /// Stop the FlickNote daemon
    Stop,
    /// Check daemon status
    Status,
    /// Install the local PowerSync daemon
    Install,
    /// Uninstall the local PowerSync daemon
    Uninstall,
}

pub(crate) fn run(config: &Config, args: &SyncArgs) -> Result<(), CliError> {
    match &args.command {
        SyncCommand::Start => start(config),
        SyncCommand::Stop => stop(config),
        SyncCommand::Status => status(config),
        SyncCommand::Install => install(config),
        SyncCommand::Uninstall => uninstall(),
    }
}

fn start(config: &Config) -> Result<(), CliError> {
    if let Some(pid) = super::daemon::read_pid(config) {
        println!("FlickNote daemon already running (pid {pid})");
        return Ok(());
    }

    let daemon_binary = super::daemon::daemon_binary()?;
    start_with_binary(config, &daemon_binary)
}

fn start_with_binary(config: &Config, daemon_binary: &Path) -> Result<(), CliError> {
    let log = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&config.paths.log_file)?;
    let log2 = log.try_clone()?;

    let child = std::process::Command::new(daemon_binary)
        .env(
            "RUST_LOG",
            std::env::var("RUST_LOG")
                .unwrap_or_else(|_| "flicknote_sync=info,powersync=debug".into()),
        )
        .stdin(std::process::Stdio::null())
        .stdout(log)
        .stderr(log2)
        .spawn()?;

    let pid = child.id();
    println!("FlickNote daemon started (pid {pid})");
    Ok(())
}

fn stop(config: &Config) -> Result<(), CliError> {
    if super::daemon::read_pid(config).is_none() {
        println!("FlickNote daemon not running");
        return Ok(());
    }
    super::daemon::stop(config)?;
    println!("FlickNote daemon stopped");
    Ok(())
}

fn status(config: &Config) -> Result<(), CliError> {
    match super::daemon::read_pid(config) {
        Some(pid) => println!("FlickNote daemon: running (pid {pid})"),
        None => println!("FlickNote daemon: not running"),
    }
    Ok(())
}

fn install(config: &Config) -> Result<(), CliError> {
    validate_install_mode(std::env::var("DATABASE_URL").ok().as_deref())?;
    super::daemon::install(config)?;
    println!("Installed and started: io.guion.flicknote.sync");
    Ok(())
}

fn validate_install_mode(database_url: Option<&str>) -> Result<(), CliError> {
    if database_url.is_some() {
        return Err(CliError::Other(
            "`flicknote sync install` only installs the local PowerSync daemon; start a managed daemon explicitly with `flicknote sync start`.".to_string(),
        ));
    }
    Ok(())
}

fn uninstall() -> Result<(), CliError> {
    super::daemon::uninstall()?;
    println!("Uninstalled: io.guion.flicknote.sync");
    Ok(())
}

#[cfg(test)]
mod tests {
    use flicknote_core::config::{Config, ConfigPaths};
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    use super::*;

    fn test_config(dir: &std::path::Path) -> Config {
        Config {
            supabase_url: String::new(),
            supabase_anon_key: String::new(),
            powersync_url: String::new(),
            api_url: String::new(),
            web_url: None,
            paths: ConfigPaths {
                config_dir: dir.to_path_buf(),
                data_dir: dir.to_path_buf(),
                config_file: dir.join("config.json"),
                session_file: dir.join("session.json"),
                db_file: dir.join("flicknote.db"),
                log_file: dir.join("flicknote.log"),
            },
        }
    }

    #[cfg(unix)]
    #[test]
    fn parent_process_does_not_write_daemon_pid_file() {
        let dir = tempfile::tempdir().expect("temp dir");
        let config = test_config(dir.path());
        let daemon = dir.path().join("fake-daemon");
        fs::write(&daemon, "#!/bin/sh\nexit 0\n").expect("write fake daemon");
        #[cfg(unix)]
        fs::set_permissions(&daemon, fs::Permissions::from_mode(0o700)).expect("chmod fake daemon");

        start_with_binary(&config, &daemon).expect("start fake daemon");

        assert!(!super::super::daemon::pid_file(&config).exists());
    }

    #[test]
    fn launchd_install_is_local_only() {
        validate_install_mode(None).unwrap();
        let error = validate_install_mode(Some("postgres://managed")).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("only installs the local PowerSync daemon")
        );
    }
}
