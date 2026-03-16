use clap::{Args, Subcommand};
use flicknote_core::config::Config;
use flicknote_core::error::CliError;
use std::fs;

#[derive(Args)]
pub(crate) struct SyncArgs {
    #[command(subcommand)]
    command: SyncCommand,
}

#[derive(Subcommand)]
enum SyncCommand {
    /// Start sync daemon in background
    Start,
    /// Stop sync daemon
    Stop,
    /// Check sync daemon status
    Status,
    /// Install sync daemon as launchd service
    Install,
    /// Uninstall sync daemon launchd service
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
        println!("Sync daemon already running (pid {pid})");
        return Ok(());
    }

    let log = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&config.paths.log_file)?;
    let log2 = log.try_clone()?;

    let child = std::process::Command::new(super::daemon::daemon_binary()?)
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
    fs::write(super::daemon::pid_file(config), pid.to_string())?;
    println!("Sync daemon started (pid {pid})");
    Ok(())
}

fn stop(config: &Config) -> Result<(), CliError> {
    if super::daemon::read_pid(config).is_none() {
        println!("Sync daemon not running");
        return Ok(());
    }
    super::daemon::stop(config)?;
    println!("Sync daemon stopped");
    Ok(())
}

fn status(config: &Config) -> Result<(), CliError> {
    match super::daemon::read_pid(config) {
        Some(pid) => println!("Sync daemon: running (pid {pid})"),
        None => println!("Sync daemon: not running"),
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
