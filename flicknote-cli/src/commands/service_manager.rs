use flicknote_core::config::Config;
use std::ffi::OsString;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use serde::Serialize;
use thiserror::Error;

use service_manager::{
    RestartPolicy, ServiceInstallCtx, ServiceLabel, ServiceLevel, ServiceManager as NativeManager,
    ServiceManagerKind, ServiceStartCtx, ServiceStatus, ServiceStatusCtx, ServiceStopCtx,
    ServiceUninstallCtx,
};

pub(crate) const SERVICE_LABEL: &str = "io.guion.flicknote.daemon";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ServiceState {
    NotInstalled,
    Stopped,
    Running,
}

#[derive(Debug, Error)]
pub(crate) enum ServiceManagerError {
    #[error("user daemon service {operation} failed: {detail}")]
    Operation {
        operation: &'static str,
        detail: String,
    },
}

impl ServiceManagerError {
    pub(crate) fn new(operation: &'static str, detail: impl std::fmt::Display) -> Self {
        Self::Operation {
            operation,
            detail: detail.to_string(),
        }
    }
}

#[cfg(any(target_os = "linux", test))]
#[derive(Debug)]
struct CommandStatus {
    success: bool,
    description: String,
}

#[cfg(any(target_os = "linux", test))]
trait CommandRunner {
    fn status(&self, program: &str, args: &[&str]) -> io::Result<CommandStatus>;
}

#[cfg(target_os = "linux")]
struct ProcessCommandRunner;

#[cfg(target_os = "linux")]
impl CommandRunner for ProcessCommandRunner {
    fn status(&self, program: &str, args: &[&str]) -> io::Result<CommandStatus> {
        let status = Command::new(program).args(args).status()?;
        Ok(CommandStatus {
            success: status.success(),
            description: status.to_string(),
        })
    }
}

pub(crate) trait ServiceManagerAdapter {
    fn status(&self) -> Result<ServiceState, ServiceManagerError>;
    fn install(&self, config: &Config) -> Result<(), ServiceManagerError>;
    fn reload(&self) -> Result<(), ServiceManagerError>;
    fn start(&self) -> Result<(), ServiceManagerError>;
    fn stop(&self) -> Result<(), ServiceManagerError>;
    fn uninstall(&self) -> Result<(), ServiceManagerError>;
}

pub(crate) trait ServiceManagerFactory: Send + Sync {
    fn manager(&self) -> Result<Box<dyn ServiceManagerAdapter>, ServiceManagerError>;
}

pub(crate) struct NativeServiceFactory;

impl ServiceManagerFactory for NativeServiceFactory {
    fn manager(&self) -> Result<Box<dyn ServiceManagerAdapter>, ServiceManagerError> {
        Ok(Box::new(NativeServiceManager::new()?))
    }
}

pub(crate) struct NativeServiceManager {
    manager: Box<dyn NativeManager>,
}

impl NativeServiceManager {
    pub(crate) fn new() -> Result<Self, ServiceManagerError> {
        let kind = native_kind()?;
        let mut manager = <dyn NativeManager>::target(kind);
        manager
            .set_level(ServiceLevel::User)
            .map_err(|error| ServiceManagerError::new("select user level", error))?;
        match manager
            .available()
            .map_err(|error| ServiceManagerError::new("query availability", error))?
        {
            true => Ok(Self { manager }),
            false => Err(ServiceManagerError::new(
                "query availability",
                format!("{kind:?} is not available on this host"),
            )),
        }
    }
}

impl ServiceManagerAdapter for NativeServiceManager {
    fn status(&self) -> Result<ServiceState, ServiceManagerError> {
        self.manager
            .status(ServiceStatusCtx {
                label: service_label(),
            })
            .map(|status| map_status(&status))
            .map_err(|error| ServiceManagerError::new("query status", error))
    }

    fn install(&self, config: &Config) -> Result<(), ServiceManagerError> {
        let program = selected_executable()
            .map_err(|error| ServiceManagerError::new("validate executable", error))?;
        self.manager
            .install(ServiceInstallCtx {
                label: service_label(),
                program: program.clone(),
                args: vec![OsString::from("daemon"), OsString::from("run")],
                username: None,
                working_directory: Some(config.paths.data_dir.clone()),
                environment: Some(service_environment(config)),
                contents: None,
                autostart: true,
                restart_policy: RestartPolicy::OnFailure {
                    delay_secs: Some(5),
                    max_retries: Some(5),
                    reset_after_secs: Some(60),
                },
            })
            .map_err(|error| ServiceManagerError::new("install", error))
    }

    fn reload(&self) -> Result<(), ServiceManagerError> {
        #[cfg(target_os = "linux")]
        reload_user_systemd(&ProcessCommandRunner)?;
        Ok(())
    }

    fn start(&self) -> Result<(), ServiceManagerError> {
        self.manager
            .start(ServiceStartCtx {
                label: service_label(),
            })
            .map_err(|error| ServiceManagerError::new("start", error))
    }

    fn stop(&self) -> Result<(), ServiceManagerError> {
        self.manager
            .stop(ServiceStopCtx {
                label: service_label(),
            })
            .map_err(|error| ServiceManagerError::new("stop", error))
    }

    fn uninstall(&self) -> Result<(), ServiceManagerError> {
        self.manager
            .uninstall(ServiceUninstallCtx {
                label: service_label(),
            })
            .map_err(|error| ServiceManagerError::new("uninstall", error))
    }
}

fn service_label() -> ServiceLabel {
    SERVICE_LABEL
        .parse()
        .expect("the built-in daemon service label is valid")
}

fn map_status(status: &ServiceStatus) -> ServiceState {
    match status {
        ServiceStatus::NotInstalled => ServiceState::NotInstalled,
        ServiceStatus::Running => ServiceState::Running,
        ServiceStatus::Stopped(_) => ServiceState::Stopped,
    }
}

fn native_kind() -> Result<ServiceManagerKind, ServiceManagerError> {
    if cfg!(target_os = "macos") {
        return Ok(ServiceManagerKind::Launchd);
    }
    if cfg!(target_os = "linux") {
        return Ok(ServiceManagerKind::Systemd);
    }
    Err(ServiceManagerError::new(
        "select platform",
        "user launchd and systemd services are the supported platforms",
    ))
}

#[cfg(any(target_os = "linux", test))]
fn reload_user_systemd(runner: &dyn CommandRunner) -> Result<(), ServiceManagerError> {
    let status = runner
        .status("systemctl", &["--user", "daemon-reload"])
        .map_err(|error| ServiceManagerError::new("reload", error))?;
    if status.success {
        return Ok(());
    }
    Err(ServiceManagerError::new(
        "reload",
        format!(
            "systemctl --user daemon-reload exited with {}",
            status.description
        ),
    ))
}

fn service_environment(config: &Config) -> Vec<(String, String)> {
    let config_root = config
        .paths
        .config_dir
        .parent()
        .unwrap_or(&config.paths.config_dir);
    let data_root = config
        .paths
        .data_dir
        .parent()
        .unwrap_or(&config.paths.data_dir);
    let mut environment = vec![
        ("FLICKNOTE_DAEMON_MANAGED".to_string(), "1".to_string()),
        (
            "RUST_LOG".to_string(),
            "flicknote_sync=info,powersync=debug".to_string(),
        ),
        (
            "XDG_CONFIG_HOME".to_string(),
            config_root.display().to_string(),
        ),
        ("XDG_DATA_HOME".to_string(), data_root.display().to_string()),
    ];
    for name in [
        "FLICKNOTE_ENV",
        "FLICKNOTE_SUPABASE_URL",
        "FLICKNOTE_SUPABASE_KEY",
        "FLICKNOTE_POWERSYNC_URL",
        "FLICKNOTE_API_URL",
        "FLICKNOTE_WEB_URL",
    ] {
        if let Ok(value) = std::env::var(name) {
            environment.push((name.to_string(), value));
        }
    }
    environment
}

pub(crate) fn selected_executable() -> Result<PathBuf, String> {
    let argument = std::env::args_os()
        .next()
        .ok_or_else(|| "the FlickNote executable path is unavailable".to_string())?;
    let path = resolve_program_path(&argument)?;
    validate_executable(&path)?;
    Ok(path)
}

fn resolve_program_path(argument: &std::ffi::OsStr) -> Result<PathBuf, String> {
    let path = PathBuf::from(argument);
    if path.components().count() > 1 {
        return Ok(if path.is_absolute() {
            path
        } else {
            std::env::current_dir()
                .map_err(|error| error.to_string())?
                .join(path)
        });
    }
    let path_variable = std::env::var_os("PATH").unwrap_or_default();
    for directory in std::env::split_paths(&path_variable) {
        let candidate = directory.join(&path);
        if candidate.is_file() {
            return Ok(candidate);
        }
    }
    std::env::current_exe().map_err(|error| error.to_string())
}

fn validate_executable(path: &Path) -> Result<(), String> {
    let metadata =
        fs::metadata(path).map_err(|error| format!("{} is not usable: {error}", path.display()))?;
    if !metadata.is_file() {
        return Err(format!("{} is not a regular file", path.display()));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o111 == 0 {
            return Err(format!("{} is not executable", path.display()));
        }
    }
    let output = Command::new(path)
        .arg("--version")
        .stdin(Stdio::null())
        .output()
        .map_err(|error| format!("{} could not be executed: {error}", path.display()))?;
    if !output.status.success() {
        return Err(format!("{} did not identify as FlickNote", path.display()));
    }
    let identity = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    if !identity.to_ascii_lowercase().contains("flicknote") {
        return Err(format!("{} did not identify as FlickNote", path.display()));
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct LogGuidance {
    pub(crate) destination: String,
    pub(crate) command: String,
}

pub(crate) fn log_guidance(config: &Config) -> LogGuidance {
    if cfg!(target_os = "macos") {
        LogGuidance {
            destination: config.paths.log_file.display().to_string(),
            command: "flicknote daemon logs --follow".to_string(),
        }
    } else {
        LogGuidance {
            destination: "systemd user journal".to_string(),
            command: "flicknote daemon logs --follow".to_string(),
        }
    }
}

pub(crate) async fn show_logs(config: &Config, lines: usize, follow: bool) -> Result<(), String> {
    if lines == 0 || lines > 10_000 {
        return Err("--lines must be between 1 and 10000".to_string());
    }
    if cfg!(target_os = "macos") {
        return show_file_logs(&config.paths.log_file, lines, follow).await;
    }
    if cfg!(target_os = "linux") {
        return show_journal_logs(lines, follow).await;
    }
    Err("daemon logs are supported on macOS and Linux".to_string())
}

async fn show_file_logs(path: &Path, lines: usize, follow: bool) -> Result<(), String> {
    let initial = read_tail(path, lines)?;
    print!("{initial}");
    io::stdout().flush().map_err(|error| error.to_string())?;
    if !follow {
        return Ok(());
    }
    let mut offset = fs::metadata(path).map_err(|error| error.to_string())?.len();
    loop {
        tokio::time::sleep(Duration::from_millis(250)).await;
        let content = fs::read(path).map_err(|error| error.to_string())?;
        if content.len() < offset as usize {
            offset = 0;
        }
        if content.len() > offset as usize {
            let chunk = String::from_utf8_lossy(&content[offset as usize..]);
            print!("{chunk}");
            io::stdout().flush().map_err(|error| error.to_string())?;
            offset = content.len() as u64;
        }
    }
}

fn read_tail(path: &Path, lines: usize) -> Result<String, String> {
    let content = fs::read_to_string(path)
        .map_err(|error| format!("could not read daemon logs at {}: {error}", path.display()))?;
    let all_lines: Vec<&str> = content.lines().collect();
    let start = all_lines.len().saturating_sub(lines);
    let mut output = all_lines[start..].join("\n");
    if !output.is_empty() {
        output.push('\n');
    }
    Ok(output)
}

async fn show_journal_logs(lines: usize, follow: bool) -> Result<(), String> {
    let mut command = tokio::process::Command::new("journalctl");
    command
        .args([
            "--user",
            "-u",
            &format!("{}.service", service_unit_name()),
            "-n",
        ])
        .arg(lines.to_string())
        .arg("--no-pager");
    if follow {
        command.arg("--follow");
        let status = command
            .status()
            .await
            .map_err(|error| format!("could not read systemd user journal: {error}"))?;
        if status.success() {
            return Ok(());
        }
        return Err(format!("journalctl exited with {status}"));
    }
    let output = command
        .output()
        .await
        .map_err(|error| format!("could not read systemd user journal: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "journalctl exited with {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    print!("{}", String::from_utf8_lossy(&output.stdout));
    Ok(())
}

pub(crate) fn service_unit_name() -> String {
    service_label().to_script_name()
}

#[cfg(test)]
mod tests {
    use super::*;
    use flicknote_core::config::ConfigPaths;

    #[test]
    fn service_environment_preserves_configured_xdg_roots() {
        let root = tempfile::tempdir().unwrap();
        let config = Config {
            supabase_url: "https://auth.example".to_string(),
            supabase_anon_key: "key".to_string(),
            powersync_url: "https://sync.example".to_string(),
            api_url: "https://api.example".to_string(),
            web_url: None,
            paths: ConfigPaths {
                config_dir: root.path().join("config/flicknote"),
                data_dir: root.path().join("data/flicknote"),
                config_file: root.path().join("config/flicknote/config.json"),
                session_file: root.path().join("config/flicknote/session.json"),
                db_file: root.path().join("data/flicknote/flicknote.db"),
                log_file: root.path().join("data/flicknote/flicknote.log"),
            },
        };
        let environment = service_environment(&config);
        assert!(environment.contains(&(
            "XDG_CONFIG_HOME".to_string(),
            root.path().join("config").display().to_string()
        )));
        assert!(environment.contains(&(
            "XDG_DATA_HOME".to_string(),
            root.path().join("data").display().to_string()
        )));
    }

    #[derive(Default)]
    struct FakeCommandRunner {
        calls: std::sync::Mutex<Vec<(String, Vec<String>)>>,
        result: std::sync::Mutex<Option<io::Result<CommandStatus>>>,
    }

    impl CommandRunner for FakeCommandRunner {
        fn status(&self, program: &str, args: &[&str]) -> io::Result<CommandStatus> {
            self.calls.lock().unwrap().push((
                program.to_string(),
                args.iter().map(ToString::to_string).collect(),
            ));
            self.result.lock().unwrap().take().unwrap_or_else(|| {
                Ok(CommandStatus {
                    success: true,
                    description: "exit status: 0".to_string(),
                })
            })
        }
    }

    #[test]
    fn systemd_reload_uses_the_user_manager() {
        let runner = FakeCommandRunner::default();

        reload_user_systemd(&runner).unwrap();

        assert_eq!(
            runner.calls.lock().unwrap().as_slice(),
            [(
                "systemctl".to_string(),
                vec!["--user".to_string(), "daemon-reload".to_string()]
            )]
        );
    }

    #[test]
    fn systemd_reload_propagates_command_failure() {
        let runner = FakeCommandRunner {
            result: std::sync::Mutex::new(Some(Ok(CommandStatus {
                success: false,
                description: "exit status: 1".to_string(),
            }))),
            ..FakeCommandRunner::default()
        };

        let error = reload_user_systemd(&runner).unwrap_err();

        assert_eq!(
            error.to_string(),
            "user daemon service reload failed: systemctl --user daemon-reload exited with exit status: 1"
        );
    }

    #[test]
    fn user_service_uses_the_daemon_label_and_unit_name() {
        assert_eq!(SERVICE_LABEL, "io.guion.flicknote.daemon");
        assert_eq!(service_unit_name(), "guion-flicknote.daemon");
    }

    #[test]
    fn executable_validation_accepts_flicknote_and_rejects_other_programs() {
        let executable = std::env::var("CARGO_BIN_EXE_flicknote")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|_| {
                std::env::current_exe()
                    .unwrap()
                    .parent()
                    .unwrap()
                    .parent()
                    .unwrap()
                    .join("flicknote")
            });
        validate_executable(&executable).unwrap();

        let directory = tempfile::tempdir().unwrap();
        let other = directory.path().join("other");
        std::fs::write(&other, "#!/bin/sh\necho another-tool 1.0.0\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&other, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        assert!(validate_executable(&other).is_err());
    }
}
