use flicknote_core::config::Config;
use flicknote_core::error::CliError;
use std::ffi::OsStr;
use std::fs;
use std::path::PathBuf;
#[cfg(target_os = "macos")]
use std::process::Command;

const DAEMON_BINARY_NAME: &str = "flicknote-sync";

pub(crate) fn pid_file(config: &Config) -> PathBuf {
    config.paths.data_dir.join("sync.pid")
}

pub(crate) fn read_pid(config: &Config) -> Option<u32> {
    let path = pid_file(config);
    let content = fs::read_to_string(&path).ok()?;
    let pid: u32 = content.trim().parse().ok()?;
    #[allow(unsafe_code)]
    if unsafe { libc::kill(pid as i32, 0) } == 0
        && process_matches_executable(pid, std::path::Path::new(DAEMON_BINARY_NAME))
    {
        return Some(pid);
    }
    #[allow(clippy::let_underscore_must_use, clippy::let_underscore_untyped)]
    let _ = fs::remove_file(&path);
    None
}

#[cfg(target_os = "linux")]
fn process_executable(pid: u32) -> Option<PathBuf> {
    fs::read_link(format!("/proc/{pid}/exe")).ok()
}

#[cfg(target_os = "macos")]
fn process_executable(pid: u32) -> Option<PathBuf> {
    use std::os::unix::ffi::OsStrExt;

    let mut buffer = vec![0_u8; libc::PROC_PIDPATHINFO_MAXSIZE as usize];
    #[allow(unsafe_code)]
    let length = unsafe {
        libc::proc_pidpath(
            pid as libc::c_int,
            buffer.as_mut_ptr().cast(),
            buffer.len() as u32,
        )
    };
    if length <= 0 {
        return None;
    }
    buffer.truncate(length as usize);
    Some(PathBuf::from(OsStr::from_bytes(&buffer)))
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn process_executable(_pid: u32) -> Option<PathBuf> {
    None
}

fn process_matches_executable(pid: u32, expected: &std::path::Path) -> bool {
    process_executable(pid).and_then(|path| path.file_name().map(OsStr::to_owned))
        == expected.file_name().map(OsStr::to_owned)
}

pub(crate) fn daemon_binary() -> Result<PathBuf, CliError> {
    let exe = std::env::current_exe()
        .map_err(|e| CliError::Other(format!("Could not determine executable path: {e}")))?;
    let dir = exe
        .parent()
        .ok_or_else(|| CliError::Other("Could not determine executable directory".into()))?;
    let binary = dir.join(DAEMON_BINARY_NAME);
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
    #[cfg(target_os = "macos")]
    {
        #[allow(unsafe_code)]
        let uid = unsafe { libc::getuid() };
        bootout_service(uid, service_label())?;
    }

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
    bootout_service(uid, label)?;

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
    bootout_service(uid, label)?;

    for args in launchd_install_commands(uid, label, &plist_path) {
        let command_name = args
            .first()
            .cloned()
            .unwrap_or_else(|| "launchctl".to_string());
        let output = Command::new("launchctl")
            .args(&args)
            .output()
            .map_err(|e| {
                CliError::Other(format!("launchctl {command_name} failed to execute: {e}"))
            })?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(CliError::Other(format!(
                "launchctl {command_name} failed: {stderr}"
            )));
        }
    }

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

#[cfg(any(target_os = "macos", test))]
fn launchd_install_commands(
    uid: u32,
    label: &str,
    plist_path: &std::path::Path,
) -> Vec<Vec<String>> {
    vec![
        vec![
            "bootstrap".to_string(),
            format!("gui/{uid}"),
            plist_path.to_string_lossy().into_owned(),
        ],
        vec![
            "kickstart".to_string(),
            "-k".to_string(),
            format!("gui/{uid}/{label}"),
        ],
    ]
}

#[cfg(any(target_os = "macos", test))]
fn launchd_stop_command(uid: u32, label: &str) -> Vec<String> {
    vec!["bootout".to_string(), format!("gui/{uid}/{label}")]
}

/// Run `launchctl bootout`; an already-unloaded service is an idempotent success.
#[cfg(target_os = "macos")]
fn bootout_service(uid: u32, label: &str) -> Result<(), CliError> {
    let args = launchd_stop_command(uid, label);
    let result = Command::new("launchctl")
        .args(&args)
        .output()
        .map_err(|error| CliError::Other(format!("launchctl bootout failed: {error}")))?;
    if !result.status.success() {
        let out = result;
        let stderr = String::from_utf8_lossy(&out.stderr);
        let is_expected = stderr.contains("No such process")
            || stderr.contains("not loaded")
            || stderr.contains("Could not find");
        if !is_expected {
            return Err(CliError::Other(format!(
                "launchctl bootout failed: {}",
                stderr.trim()
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use flicknote_core::config::ConfigPaths;

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
    fn launchd_install_runs_bootstrap_then_kickstart() {
        let plist = PathBuf::from("/Users/neil/Library/LaunchAgents/io.guion.flicknote.sync.plist");
        let commands = launchd_install_commands(501, "io.guion.flicknote.sync", &plist);

        assert_eq!(
            commands,
            vec![
                vec![
                    "bootstrap".to_string(),
                    "gui/501".to_string(),
                    plist.to_string_lossy().into_owned(),
                ],
                vec![
                    "kickstart".to_string(),
                    "-k".to_string(),
                    "gui/501/io.guion.flicknote.sync".to_string(),
                ],
            ]
        );
    }

    #[test]
    fn launchd_stop_boots_out_the_keepalive_service() {
        assert_eq!(
            launchd_stop_command(501, "io.guion.flicknote.sync"),
            vec![
                "bootout".to_string(),
                "gui/501/io.guion.flicknote.sync".to_string(),
            ]
        );
    }

    #[test]
    fn process_identity_must_match_expected_executable_before_signalling() {
        let current = std::env::current_exe().unwrap();
        assert!(process_matches_executable(std::process::id(), &current));

        let unrelated = tempfile::NamedTempFile::new().unwrap();
        assert!(!process_matches_executable(
            std::process::id(),
            unrelated.path(),
        ));
    }

    #[test]
    fn stale_pid_for_an_unrelated_live_process_is_removed_without_being_accepted() {
        let directory = tempfile::tempdir().unwrap();
        let config = test_config(directory.path());
        fs::write(pid_file(&config), std::process::id().to_string()).unwrap();

        assert_eq!(read_pid(&config), None);
        assert!(!pid_file(&config).exists());
    }
}
