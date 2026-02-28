use clap::{Args, Subcommand};
use flicknote_core::config::Config;
use flicknote_core::error::CliError;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

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
        SyncCommand::Uninstall => uninstall(config),
    }
}

fn pid_file(config: &Config) -> PathBuf {
    config.paths.data_dir.join("sync.pid")
}

fn read_pid(config: &Config) -> Option<u32> {
    let path = pid_file(config);
    let content = fs::read_to_string(&path).ok()?;
    let pid: u32 = content.trim().parse().ok()?;
    // Check if process is alive
    #[allow(unsafe_code)]
    if unsafe { libc::kill(pid as i32, 0) } == 0 {
        return Some(pid);
    }
    // Stale PID file — best-effort cleanup
    #[allow(clippy::let_underscore_must_use, clippy::let_underscore_untyped)]
    let _ = fs::remove_file(&path);
    None
}

fn daemon_binary() -> Result<PathBuf, CliError> {
    let exe = std::env::current_exe()
        .map_err(|e| CliError::Other(format!("Could not determine executable path: {e}")))?;
    let dir = exe
        .parent()
        .ok_or_else(|| CliError::Other("Could not determine executable directory".into()))?;
    Ok(dir.join("flicknote-sync"))
}

fn start(config: &Config) -> Result<(), CliError> {
    if let Some(pid) = read_pid(config) {
        println!("Sync daemon already running (pid {pid})");
        return Ok(());
    }

    let log = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&config.paths.log_file)?;
    let log2 = log.try_clone()?;

    let child = Command::new(daemon_binary()?)
        .env(
            "RUST_LOG",
            std::env::var("RUST_LOG")
                .unwrap_or_else(|_| "flicknote_sync=info,powersync=warn".into()),
        )
        .stdin(std::process::Stdio::null())
        .stdout(log)
        .stderr(log2)
        .spawn()?;

    let pid = child.id();
    fs::write(pid_file(config), pid.to_string())?;
    println!("Sync daemon started (pid {pid})");
    Ok(())
}

fn stop(config: &Config) -> Result<(), CliError> {
    let Some(pid) = read_pid(config) else {
        println!("Sync daemon not running");
        return Ok(());
    };

    #[allow(unsafe_code)]
    let ret = unsafe { libc::kill(pid as i32, libc::SIGTERM) };
    if ret == -1 {
        let err = std::io::Error::last_os_error();
        eprintln!("Warning: failed to send SIGTERM to pid {pid}: {err}");
    }
    // Best-effort cleanup after SIGTERM
    #[allow(clippy::let_underscore_must_use, clippy::let_underscore_untyped)]
    let _ = fs::remove_file(pid_file(config));
    println!("Sync daemon stopped");
    Ok(())
}

fn status(config: &Config) -> Result<(), CliError> {
    match read_pid(config) {
        Some(pid) => println!("Sync daemon: running (pid {pid})"),
        None => println!("Sync daemon: not running"),
    }
    Ok(())
}

fn install(config: &Config) -> Result<(), CliError> {
    #[cfg(not(target_os = "macos"))]
    {
        println!("launchd install is only supported on macOS");
        return Ok(());
    }

    #[cfg(target_os = "macos")]
    {
        let label = "io.guion.flicknote.sync";
        let home = dirs::home_dir()
            .ok_or_else(|| CliError::Other("Could not determine home directory".into()))?;
        let plist_path = home
            .join("Library/LaunchAgents")
            .join(format!("{label}.plist"));
        let daemon = daemon_binary()?;

        let plist = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>{label}</string>
    <key>ProgramArguments</key>
    <array>
        <string>{}</string>
    </array>
    <key>EnvironmentVariables</key>
    <dict>
        <key>RUST_LOG</key>
        <string>flicknote_sync=info,powersync=warn</string>
    </dict>
    <key>KeepAlive</key>
    <true/>
    <key>RunAtLoad</key>
    <true/>
    <key>StandardOutPath</key>
    <string>{}</string>
    <key>StandardErrorPath</key>
    <string>{}</string>
</dict>
</plist>"#,
            daemon.display(),
            config.paths.log_file.display(),
            config.paths.log_file.display(),
        );

        fs::create_dir_all(
            plist_path
                .parent()
                .ok_or_else(|| CliError::Other("Could not determine LaunchAgents directory".into()))?,
        )?;
        fs::write(&plist_path, &plist)?;

        #[allow(unsafe_code)]
        let uid = unsafe { libc::getuid() };

        // Bootout existing service — ok to fail if not loaded
        let output = Command::new("launchctl")
            .args(["bootout", &format!("gui/{uid}/{label}")])
            .output()
            .map_err(|e| CliError::Other(format!("launchctl bootout failed to execute: {e}")))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            // "Could not find specified service" is expected on first install
            if !stderr.contains("Could not find specified service") {
                return Err(CliError::Other(format!(
                    "launchctl bootout failed: {stderr}"
                )));
            }
        }

        // Bootstrap must succeed
        let output = Command::new("launchctl")
            .args([
                "bootstrap",
                &format!("gui/{uid}"),
                plist_path.to_string_lossy().as_ref(),
            ])
            .output()
            .map_err(|e| CliError::Other(format!("launchctl bootstrap failed to execute: {e}")))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(CliError::Other(format!(
                "launchctl bootstrap failed: {stderr}"
            )));
        }

        println!("Installed and started: {label}");
        Ok(())
    }
}

fn uninstall(_config: &Config) -> Result<(), CliError> {
    #[cfg(not(target_os = "macos"))]
    {
        println!("launchd uninstall is only supported on macOS");
        return Ok(());
    }

    #[cfg(target_os = "macos")]
    {
        let label = "io.guion.flicknote.sync";
        let home = dirs::home_dir()
            .ok_or_else(|| CliError::Other("Could not determine home directory".into()))?;
        let plist_path = home
            .join("Library/LaunchAgents")
            .join(format!("{label}.plist"));

        #[allow(unsafe_code)]
        let uid = unsafe { libc::getuid() };
        let output = Command::new("launchctl")
            .args(["bootout", &format!("gui/{uid}/{label}")])
            .output()
            .map_err(|e| CliError::Other(format!("launchctl bootout failed to execute: {e}")))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            // Uninstall is lenient with bootout failures (unlike install) because
            // the goal is to clean up — if the service is already gone or in a bad
            // state, we still want to remove the plist file and report success.
            if !stderr.contains("Could not find specified service") {
                eprintln!("Warning: launchctl bootout failed: {stderr}");
            }
        }

        if plist_path.exists() {
            fs::remove_file(&plist_path)?;
        }

        println!("Uninstalled: {label}");
        Ok(())
    }
}
