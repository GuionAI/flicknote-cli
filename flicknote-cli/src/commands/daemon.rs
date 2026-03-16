use flicknote_core::config::Config;
use flicknote_core::error::CliError;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

pub(crate) fn pid_file(config: &Config) -> PathBuf {
    config.paths.data_dir.join("sync.pid")
}

pub(crate) fn read_pid(config: &Config) -> Option<u32> {
    let path = pid_file(config);
    let content = fs::read_to_string(&path).ok()?;
    let pid: u32 = content.trim().parse().ok()?;
    #[allow(unsafe_code)]
    if unsafe { libc::kill(pid as i32, 0) } == 0 {
        return Some(pid);
    }
    #[allow(clippy::let_underscore_must_use, clippy::let_underscore_untyped)]
    let _ = fs::remove_file(&path);
    None
}

pub(crate) fn daemon_binary() -> Result<PathBuf, CliError> {
    let exe = std::env::current_exe()
        .map_err(|e| CliError::Other(format!("Could not determine executable path: {e}")))?;
    let dir = exe
        .parent()
        .ok_or_else(|| CliError::Other("Could not determine executable directory".into()))?;
    let binary = dir.join("flicknote-sync");
    if !binary.exists() {
        return Err(CliError::Other(format!(
            "Sync daemon binary not found at {}: ensure flicknote-sync is installed alongside flicknote",
            binary.display()
        )));
    }
    Ok(binary)
}

/// Stop the sync daemon if running. Returns Ok(()) even if not running.
pub(crate) fn stop(config: &Config) -> Result<(), CliError> {
    let Some(pid) = read_pid(config) else {
        return Ok(());
    };

    #[allow(unsafe_code)]
    let ret = unsafe { libc::kill(pid as i32, libc::SIGTERM) };
    if ret == -1 {
        let err = std::io::Error::last_os_error();
        if err.raw_os_error() == Some(libc::ESRCH) {
            // Process already gone — clean up stale PID file
        } else {
            return Err(CliError::Other(format!(
                "Failed to stop sync daemon (pid {pid}): {err}"
            )));
        }
    }
    #[allow(clippy::let_underscore_must_use, clippy::let_underscore_untyped)]
    let _ = fs::remove_file(pid_file(config));
    Ok(())
}

/// Uninstall the launchd service. Returns Ok(()) even if not installed.
#[cfg(target_os = "macos")]
pub(crate) fn uninstall() -> Result<(), CliError> {
    let label = service_label();
    let plist_path = service_plist_path()?;

    #[allow(unsafe_code)]
    let uid = unsafe { libc::getuid() };
    bootout_service(uid, label);

    if plist_path.exists() {
        fs::remove_file(&plist_path)?;
    }

    Ok(())
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn uninstall() -> Result<(), CliError> {
    Ok(())
}

/// Install the launchd service (does bootout first if already installed).
/// The service has KeepAlive + RunAtLoad, so the daemon starts immediately.
#[cfg(target_os = "macos")]
pub(crate) fn install(config: &Config) -> Result<(), CliError> {
    let label = service_label();
    let plist_path = service_plist_path()?;
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
        <string>flicknote_sync=info,powersync=debug</string>
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
        xml_escape(&daemon.display().to_string()),
        xml_escape(&config.paths.log_file.display().to_string()),
        xml_escape(&config.paths.log_file.display().to_string()),
    );

    fs::create_dir_all(
        plist_path
            .parent()
            .ok_or_else(|| CliError::Other("Could not determine LaunchAgents directory".into()))?,
    )?;
    fs::write(&plist_path, &plist)?;

    #[allow(unsafe_code)]
    let uid = unsafe { libc::getuid() };
    bootout_service(uid, label);

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

    Ok(())
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn install(_config: &Config) -> Result<(), CliError> {
    Ok(())
}

#[cfg(target_os = "macos")]
fn service_label() -> &'static str {
    "io.guion.flicknote.sync"
}

#[cfg(target_os = "macos")]
fn service_plist_path() -> Result<PathBuf, CliError> {
    let label = service_label();
    let home = dirs::home_dir()
        .ok_or_else(|| CliError::Other("Could not determine home directory".into()))?;
    Ok(home
        .join("Library/LaunchAgents")
        .join(format!("{label}.plist")))
}

#[cfg(target_os = "macos")]
fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Run `launchctl bootout`, warning on unexpected errors (not-loaded is expected and silent).
#[cfg(target_os = "macos")]
fn bootout_service(uid: u32, label: &str) {
    let result = Command::new("launchctl")
        .args(["bootout", &format!("gui/{uid}/{label}")])
        .output();
    if let Ok(out) = result
        && !out.status.success()
    {
        let stderr = String::from_utf8_lossy(&out.stderr);
        let is_expected = stderr.contains("No such process")
            || stderr.contains("not loaded")
            || stderr.contains("Could not find");
        if !is_expected && !stderr.trim().is_empty() {
            eprintln!("Warning: launchctl bootout: {}", stderr.trim());
        }
    }
}
