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
    /// Start local sync service in background
    Start,
    /// Stop local sync service
    Stop,
    /// Check local sync service status
    Status,
    /// Install local sync service
    Install,
    /// Uninstall local sync service
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
        println!("Local sync service already running (pid {pid})");
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
    println!("Local sync service started (pid {pid})");
    Ok(())
}

fn stop(config: &Config) -> Result<(), CliError> {
    if super::daemon::read_pid(config).is_none() {
        println!("Local sync service not running");
        return Ok(());
    }
    super::daemon::stop(config)?;
    println!("Local sync service stopped");
    Ok(())
}

fn status(config: &Config) -> Result<(), CliError> {
    match super::daemon::read_pid(config) {
        Some(pid) => println!("Local sync service: running (pid {pid})"),
        None => println!("Local sync service: not running"),
    }
    Ok(())
}

fn install(config: &Config) -> Result<(), CliError> {
    super::daemon::install(config)?;
    println!("Installed and started: io.guion.flicknote.sync");
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
}
