use clap::{Args, Subcommand};
use flicknote_core::config::Config;
use flicknote_core::error::CliError;
use std::fs;
use std::path::Path;

const DAEMON_START_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);
const HEALTH_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(100);

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
    /// Install the local PowerSync daemon as a launchd service (macOS only)
    Install,
    /// Uninstall the local PowerSync launchd service (macOS only)
    Uninstall,
}

pub(crate) async fn run(config: &Config, args: &SyncArgs) -> Result<(), CliError> {
    match &args.command {
        SyncCommand::Start => start(config).await,
        SyncCommand::Stop => stop(config).await,
        SyncCommand::Status => status(config).await,
        SyncCommand::Install => install(config).await,
        SyncCommand::Uninstall => uninstall(),
    }
}

async fn start(config: &Config) -> Result<(), CliError> {
    if let Some(pid) = super::daemon::read_pid(config) {
        wait_for_daemon_ready(config, DAEMON_START_TIMEOUT, HEALTH_POLL_INTERVAL).await?;
        println!("FlickNote daemon already running (pid {pid})");
        return Ok(());
    }

    let daemon_binary = super::daemon::daemon_binary()?;
    start_with_binary(config, &daemon_binary).await
}

async fn start_with_binary(config: &Config, daemon_binary: &Path) -> Result<(), CliError> {
    start_with_binary_and_timeout(config, daemon_binary, DAEMON_START_TIMEOUT).await
}

async fn start_with_binary_and_timeout(
    config: &Config,
    daemon_binary: &Path,
    timeout: std::time::Duration,
) -> Result<(), CliError> {
    let log = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&config.paths.log_file)?;
    let log2 = log.try_clone()?;

    let mut command = std::process::Command::new(daemon_binary);
    command
        .env(
            "RUST_LOG",
            std::env::var("RUST_LOG")
                .unwrap_or_else(|_| "flicknote_sync=info,powersync=debug".into()),
        )
        .stdin(std::process::Stdio::null())
        .stdout(log)
        .stderr(log2);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;

        // Manual background start must survive the invoking terminal/session.
        // launchd already owns this responsibility for installed macOS services.
        #[allow(unsafe_code)]
        unsafe {
            command.pre_exec(|| {
                if libc::setsid() == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
    }
    let mut child = command.spawn()?;

    let pid = child.id();
    if let Err(error) = wait_for_daemon_ready(config, timeout, HEALTH_POLL_INTERVAL).await {
        if let Err(kill_error) = child.kill()
            && kill_error.kind() != std::io::ErrorKind::InvalidInput
        {
            log::warn!("Failed to stop unready daemon process {pid}: {kill_error}");
        }
        if let Err(wait_error) = child.wait() {
            log::warn!("Failed to reap unready daemon process {pid}: {wait_error}");
        }
        return Err(error);
    }
    println!("FlickNote daemon started (pid {pid})");
    Ok(())
}

pub(super) async fn wait_for_daemon_ready(
    config: &Config,
    timeout: std::time::Duration,
    interval: std::time::Duration,
) -> Result<(), CliError> {
    let wait = async {
        loop {
            match flicknote_sync::ipc::DaemonClient::new(config)
                .health()
                .await
            {
                Ok(_) => return Ok(()),
                Err(error) if !error.retryable() => return Err(CliError::from(error)),
                Err(_) => tokio::time::sleep(interval).await,
            }
        }
    };
    tokio::time::timeout(timeout, wait).await.map_err(|_| {
        CliError::Other(format!(
            "Sync daemon did not become ready within {timeout:?}; check {}",
            config.paths.log_file.display()
        ))
    })?
}

async fn stop(config: &Config) -> Result<(), CliError> {
    let was_running = super::daemon::read_pid(config).is_some()
        || flicknote_sync::ipc::socket_path(config).exists();
    super::daemon::stop(config)?;
    if !was_running {
        println!("FlickNote daemon not running");
        return Ok(());
    }
    wait_for_daemon_stopped(config, DAEMON_START_TIMEOUT, HEALTH_POLL_INTERVAL).await?;
    let socket = flicknote_sync::ipc::socket_path(config);
    if socket.exists() {
        fs::remove_file(socket)?;
    }
    println!("FlickNote daemon stopped");
    Ok(())
}

async fn wait_for_daemon_stopped(
    config: &Config,
    timeout: std::time::Duration,
    interval: std::time::Duration,
) -> Result<(), CliError> {
    let wait = async {
        loop {
            let health = flicknote_sync::ipc::DaemonClient::new(config)
                .health()
                .await;
            if matches!(health, Err(ref error) if error.code() == "daemon_unavailable") {
                return;
            }
            tokio::time::sleep(interval).await;
        }
    };
    tokio::time::timeout(timeout, wait).await.map_err(|_| {
        CliError::Other(format!(
            "Sync daemon did not stop within {timeout:?}; check {}",
            config.paths.log_file.display()
        ))
    })
}

async fn status(config: &Config) -> Result<(), CliError> {
    match super::daemon::read_pid(config) {
        Some(pid) => {
            let info = flicknote_sync::ipc::DaemonClient::new(config)
                .health()
                .await?;
            println!("{}", format_running_status(pid, &info));
        }
        None => println!("FlickNote daemon: not running"),
    }
    Ok(())
}

fn format_running_status(pid: u32, info: &flicknote_sync::ipc::ServerInfo) -> String {
    format!(
        "FlickNote daemon: running (pid {pid}, version {}, protocol {})",
        info.version, info.protocol
    )
}

async fn install(config: &Config) -> Result<(), CliError> {
    install_with_timeout(config, DAEMON_START_TIMEOUT).await
}

async fn install_with_timeout(
    config: &Config,
    timeout: std::time::Duration,
) -> Result<(), CliError> {
    install_local_daemon(config, timeout).await?;
    println!("Installed and started: io.guion.flicknote.sync");
    Ok(())
}

pub(super) async fn install_local_daemon(
    config: &Config,
    timeout: std::time::Duration,
) -> Result<(), CliError> {
    #[cfg(not(target_os = "macos"))]
    {
        let (_config, _timeout) = (config, timeout);
        validate_launchd_platform()
    }

    #[cfg(target_os = "macos")]
    {
        validate_launchd_platform()?;
        // Prove that the shared endpoint is no longer owned by an old launchd or
        // standalone daemon before starting the new local LaunchAgent.
        super::daemon::stop(config)?;
        wait_for_daemon_stopped(config, timeout, HEALTH_POLL_INTERVAL).await?;
        super::daemon::install(config)?;
        wait_for_daemon_ready(config, timeout, HEALTH_POLL_INTERVAL).await
    }
}

fn validate_launchd_platform() -> Result<(), CliError> {
    if !cfg!(target_os = "macos") {
        return Err(CliError::Other(
            "launchd installation is only supported on macOS; use `flicknote sync start` on this platform".to_string(),
        ));
    }
    Ok(())
}

fn uninstall() -> Result<(), CliError> {
    validate_launchd_platform()?;
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
    #[tokio::test]
    async fn start_does_not_report_success_before_daemon_health_is_ready() {
        let dir = tempfile::tempdir().expect("temp dir");
        let config = test_config(dir.path());
        let daemon = dir.path().join("fake-daemon");
        fs::write(&daemon, "#!/bin/sh\nexit 0\n").expect("write fake daemon");
        #[cfg(unix)]
        fs::set_permissions(&daemon, fs::Permissions::from_mode(0o700)).expect("chmod fake daemon");

        let error =
            start_with_binary_and_timeout(&config, &daemon, std::time::Duration::from_millis(50))
                .await
                .expect_err("a process that exits without serving health is not ready");

        assert!(!super::super::daemon::pid_file(&config).exists());
        assert!(error.to_string().contains("did not become ready"));
    }

    #[cfg(not(target_os = "macos"))]
    #[tokio::test]
    async fn install_rejects_unsupported_platform_without_waiting() {
        let dir = tempfile::tempdir().expect("temp dir");
        let config = test_config(dir.path());

        let error = install_with_timeout(&config, std::time::Duration::from_millis(20))
            .await
            .expect_err("non-macOS install must be rejected immediately");

        assert!(error.to_string().contains("only supported on macOS"));
    }

    #[tokio::test]
    async fn stop_waits_until_daemon_health_is_unavailable() {
        use tokio::io::{AsyncBufReadExt, AsyncWriteExt};

        let dir = tempfile::tempdir().expect("temp dir");
        let config = test_config(dir.path());
        let listener = tokio::net::UnixListener::bind(flicknote_sync::ipc::socket_path(&config))
            .expect("bind socket");
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let (reader, mut writer) = stream.into_split();
            let mut reader = tokio::io::BufReader::new(reader);
            let mut request = String::new();
            reader.read_line(&mut request).await.unwrap();
            let response = serde_json::to_vec(&flicknote_sync::ipc::DaemonResponse::ServerInfo(
                flicknote_sync::ipc::ServerInfo::current(),
            ))
            .unwrap();
            writer.write_all(&response).await.unwrap();
        });

        wait_for_daemon_stopped(
            &config,
            std::time::Duration::from_millis(500),
            std::time::Duration::from_millis(10),
        )
        .await
        .unwrap();
        server.await.unwrap();
    }

    #[test]
    fn status_line_reports_runtime_version_and_protocol() {
        let line = format_running_status(42, &flicknote_sync::ipc::ServerInfo::current());

        assert!(line.contains("pid 42"));
        assert!(line.contains(env!("CARGO_PKG_VERSION")));
        assert!(line.contains("protocol 2"));
    }
}
