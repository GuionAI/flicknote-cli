use super::daemon_lifecycle::{DaemonHealthProbe, IpcHealthProbe, LifecycleController};
use super::service_manager::{
    LogGuidance, NativeServiceManager, ServiceManagerAdapter, ServiceManagerError, ServiceState,
    log_guidance, show_logs,
};
use clap::{Args, Subcommand};
use flicknote_core::config::Config;
use flicknote_core::error::CliError;
use flicknote_core::services::error::ServiceError;
use serde::Serialize;
#[cfg(target_os = "macos")]
use std::fs::{self, OpenOptions};
#[cfg(target_os = "macos")]
use std::io::{self, Write};
#[cfg(target_os = "macos")]
use std::os::fd::AsRawFd;

#[derive(Args)]
pub(crate) struct DaemonArgs {
    #[command(subcommand)]
    command: DaemonCommand,
}

#[derive(Subcommand)]
enum DaemonCommand {
    /// Install, reconcile, start, and verify the user daemon service
    Install,
    /// Stop and remove the user daemon service
    Uninstall,
    /// Start an installed user daemon service
    Start,
    /// Stop the user daemon service without removing its installation
    Stop,
    /// Restart an installed user daemon service
    Restart,
    /// Show daemon service, application, and PowerSync status
    Status(StatusArgs),
    /// Show recent managed-daemon logs
    Logs(LogsArgs),
    /// Run the daemon synchronously in the foreground
    Run,
}

#[derive(Args)]
struct StatusArgs {
    /// Include service, application, protocol, sync, and log details
    #[arg(long, conflicts_with = "json")]
    verbose: bool,
    /// Emit a stable object-root JSON report
    #[arg(long, conflicts_with = "verbose")]
    json: bool,
}

#[derive(Args)]
struct LogsArgs {
    /// Number of recent lines to display
    #[arg(long, default_value_t = 100)]
    lines: usize,
    /// Continue streaming new log output
    #[arg(long)]
    follow: bool,
}

pub(crate) async fn run(config: &Config, args: &DaemonArgs) -> Result<(), CliError> {
    match &args.command {
        DaemonCommand::Install => install(config).await,
        DaemonCommand::Uninstall => uninstall(config).await,
        DaemonCommand::Start => start(config).await,
        DaemonCommand::Stop => stop(config).await,
        DaemonCommand::Restart => restart(config).await,
        DaemonCommand::Status(args) => status(config, args).await,
        DaemonCommand::Logs(args) => logs(config, args).await,
        DaemonCommand::Run => run_foreground(config).await,
    }
}

async fn install(config: &Config) -> Result<(), CliError> {
    ensure_authenticated(config)?;
    install_and_wait(config).await?;
    println!("FlickNote daemon installed and ready");
    Ok(())
}

pub(crate) async fn install_and_wait(config: &Config) -> Result<(), CliError> {
    config.validate()?;
    ensure_authenticated(config)?;
    LifecycleController::native(&IpcHealthProbe)
        .install_and_wait(config)
        .await
}

async fn uninstall(config: &Config) -> Result<(), CliError> {
    uninstall_service(config).await?;
    println!("FlickNote daemon service uninstalled");
    Ok(())
}

pub(crate) async fn uninstall_service(config: &Config) -> Result<(), CliError> {
    LifecycleController::native(&IpcHealthProbe)
        .uninstall(config)
        .await
        .map(|_| ())
}

async fn start(config: &Config) -> Result<(), CliError> {
    ensure_authenticated(config)?;
    LifecycleController::native(&IpcHealthProbe)
        .start(config)
        .await?;
    println!("FlickNote daemon started and ready");
    Ok(())
}

async fn stop(config: &Config) -> Result<(), CliError> {
    let was_running = LifecycleController::native(&IpcHealthProbe)
        .stop(config)
        .await?;
    if was_running {
        println!("FlickNote daemon stopped");
    } else {
        println!("FlickNote daemon service is already stopped");
    }
    Ok(())
}

async fn restart(config: &Config) -> Result<(), CliError> {
    ensure_authenticated(config)?;
    LifecycleController::native(&IpcHealthProbe)
        .restart(config)
        .await?;
    println!("FlickNote daemon restarted and ready");
    Ok(())
}

async fn status(config: &Config, args: &StatusArgs) -> Result<(), CliError> {
    let report = build_status_report(config, service_state_for_status()).await;
    if args.json {
        println!(
            "{}",
            serde_json::to_string(&report).map_err(|error| CliError::Other(error.to_string()))?
        );
    } else if args.verbose {
        println!("{}", report.verbose_text());
    } else {
        println!("{}", report.concise_text());
    }
    if report.is_ready() {
        Ok(())
    } else {
        Err(CliError::Other("FlickNote daemon is not ready".to_string()))
    }
}

async fn logs(config: &Config, args: &LogsArgs) -> Result<(), CliError> {
    show_logs(config, args.lines, args.follow)
        .await
        .map_err(CliError::Other)
}

async fn run_foreground(config: &Config) -> Result<(), CliError> {
    let managed = std::env::var_os("FLICKNOTE_DAEMON_MANAGED").is_some();
    initialize_daemon_logging(config)?;
    match flicknote_sync::run(config.clone()).await {
        Ok(()) => Ok(()),
        Err(error) if managed && error.is_permanent_startup() => {
            log::error!("Permanent daemon startup failure: {error}");
            Ok(())
        }
        Err(error) => Err(CliError::Other(format!("FlickNote daemon failed: {error}"))),
    }
}

fn ensure_authenticated(config: &Config) -> Result<(), CliError> {
    flicknote_core::session::get_user_id(config).map(|_| ())
}

fn initialize_daemon_logging(config: &Config) -> Result<(), CliError> {
    #[cfg(target_os = "macos")]
    if std::env::var_os("FLICKNOTE_DAEMON_MANAGED").is_some() {
        redirect_managed_daemon_output(config)?;
    }
    let mut builder = env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or("flicknote_sync=info,powersync=debug"),
    );
    match builder.try_init() {
        Ok(()) | Err(_) => {}
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn redirect_managed_daemon_output(config: &Config) -> Result<(), CliError> {
    fs::create_dir_all(&config.paths.data_dir)?;
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&config.paths.log_file)?;
    io::stdout().flush()?;
    io::stderr().flush()?;
    for descriptor in [libc::STDERR_FILENO, libc::STDOUT_FILENO] {
        #[allow(unsafe_code)]
        if unsafe { libc::dup2(file.as_raw_fd(), descriptor) } == -1 {
            return Err(CliError::Io(io::Error::last_os_error()));
        }
    }
    Ok(())
}

fn service_state_for_status() -> Result<ServiceState, ServiceManagerError> {
    let manager = NativeServiceManager::new()?;
    manager.status()
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum ServiceStatusState {
    NotInstalled,
    InstalledStopped,
    Running,
    QueryFailed,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum ApplicationStatusState {
    Ready,
    Unavailable,
    ProtocolIncompatible,
    Unknown,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum SyncStatusState {
    Connected,
    Connecting,
    Offline,
    Unavailable,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
struct VersionStatus {
    cli: String,
    daemon: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
struct ProtocolStatus {
    cli: u16,
    daemon: Option<u16>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
struct StatusError {
    code: String,
    message: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
struct StatusReport {
    service_state: ServiceStatusState,
    application_state: ApplicationStatusState,
    sync_state: SyncStatusState,
    sync_errors: flicknote_sync::ipc::PowerSyncErrors,
    daemon_executable: Option<String>,
    version: VersionStatus,
    protocol: ProtocolStatus,
    error: Option<StatusError>,
    service_error: Option<StatusError>,
    #[serde(skip)]
    service_diagnostic: Option<String>,
    log_guidance: LogGuidance,
}

impl StatusReport {
    fn base(config: &Config) -> Self {
        Self {
            service_state: ServiceStatusState::QueryFailed,
            application_state: ApplicationStatusState::Unknown,
            sync_state: SyncStatusState::Unavailable,
            sync_errors: flicknote_sync::ipc::PowerSyncErrors::default(),
            daemon_executable: None,
            version: VersionStatus {
                cli: env!("CARGO_PKG_VERSION").to_string(),
                daemon: None,
            },
            protocol: ProtocolStatus {
                cli: flicknote_sync::ipc::PROTOCOL_VERSION,
                daemon: None,
            },
            error: None,
            service_error: None,
            service_diagnostic: None,
            log_guidance: log_guidance(config),
        }
    }

    fn is_ready(&self) -> bool {
        self.application_state == ApplicationStatusState::Ready
    }

    fn concise_text(&self) -> String {
        if self.is_ready() {
            return format!(
                "FlickNote daemon: ready (service {}, sync {})",
                format_service_state(self.service_state),
                format_sync_state(self.sync_state)
            );
        }
        let action = match self.application_state {
            ApplicationStatusState::ProtocolIncompatible => "restart",
            _ if self.service_state == ServiceStatusState::NotInstalled => "install",
            _ => "start",
        };
        format!(
            "FlickNote daemon: {} — run `flicknote daemon status --verbose`, then `flicknote daemon {action}`",
            format_application_state(self.application_state)
        )
    }

    fn verbose_text(&self) -> String {
        let error = self
            .error
            .as_ref()
            .map(|error| format!("{}: {}", error.code, error.message))
            .unwrap_or_else(|| "none".to_string());
        let service_error = self
            .service_error
            .as_ref()
            .map(|error| format!("{}: {}", error.code, error.message))
            .unwrap_or_else(|| "none".to_string());
        let service_diagnostic = self.service_diagnostic.as_deref().unwrap_or("none");
        let download_error = self.sync_errors.download.as_deref().unwrap_or("none");
        let upload_error = self.sync_errors.upload.as_deref().unwrap_or("none");
        format!(
            "service: {}\napplication: {}\ndaemon executable: {}\nFlickNote version: cli {}, daemon {}\nIPC protocol: cli {}, daemon {}\nPowerSync: {}\nPowerSync download error: {}\nPowerSync upload error: {}\nlast error: {}\nservice error: {}\nservice diagnostics: {}\nlogs: {}\nlog command: {}",
            format_service_state(self.service_state),
            format_application_state(self.application_state),
            self.daemon_executable.as_deref().unwrap_or("unavailable"),
            self.version.cli,
            self.version.daemon.as_deref().unwrap_or("unavailable"),
            self.protocol.cli,
            self.protocol
                .daemon
                .map_or_else(|| "unavailable".to_string(), |value| value.to_string()),
            format_sync_state(self.sync_state),
            download_error,
            upload_error,
            error,
            service_error,
            service_diagnostic,
            self.log_guidance.destination,
            self.log_guidance.command,
        )
    }
}

async fn build_status_report(
    config: &Config,
    service: Result<ServiceState, ServiceManagerError>,
) -> StatusReport {
    let probe = IpcHealthProbe;
    build_status_report_with_probe(config, service, &probe).await
}

async fn build_status_report_with_probe(
    config: &Config,
    service: Result<ServiceState, ServiceManagerError>,
    health: &dyn DaemonHealthProbe,
) -> StatusReport {
    let mut report = StatusReport::base(config);
    match service {
        Ok(ServiceState::NotInstalled) => report.service_state = ServiceStatusState::NotInstalled,
        Ok(ServiceState::Stopped) => report.service_state = ServiceStatusState::InstalledStopped,
        Ok(ServiceState::Running) => report.service_state = ServiceStatusState::Running,
        Err(error) => {
            report.service_state = ServiceStatusState::QueryFailed;
            report.service_error = Some(StatusError {
                code: "service_manager_query_failed".to_string(),
                message:
                    "Could not inspect the FlickNote daemon service; see verbose logs for diagnosis"
                        .to_string(),
            });
            report.service_diagnostic = Some(error.to_string());
        }
    }

    match health.health(config).await {
        Ok(info) => {
            report.application_state = ApplicationStatusState::Ready;
            report.daemon_executable = Some(info.executable);
            report.version.daemon = Some(info.version);
            report.protocol.daemon = Some(info.protocol);
            report.sync_errors = info.sync_errors;
            report.sync_state = match info.sync {
                Some(flicknote_sync::ipc::SyncConnectionState::Connected) => {
                    SyncStatusState::Connected
                }
                Some(flicknote_sync::ipc::SyncConnectionState::Connecting) => {
                    SyncStatusState::Connecting
                }
                Some(flicknote_sync::ipc::SyncConnectionState::Offline) | None => {
                    SyncStatusState::Offline
                }
            };
        }
        Err(error) => apply_health_error(&mut report, error),
    }
    report
}

fn apply_health_error(report: &mut StatusReport, error: ServiceError) {
    let code = error.code().to_string();
    report.application_state = if code == flicknote_sync::ipc::PROTOCOL_MISMATCH_CODE {
        ApplicationStatusState::ProtocolIncompatible
    } else {
        ApplicationStatusState::Unavailable
    };
    report.error = Some(StatusError {
        code,
        message: error.to_string(),
    });
    if let ServiceError::Remote {
        details: Some(details),
        ..
    } = error
    {
        report.daemon_executable = details
            .get("daemon_executable")
            .and_then(serde_json::Value::as_str)
            .map(ToOwned::to_owned);
        report.version.daemon = details
            .get("daemon_version")
            .and_then(serde_json::Value::as_str)
            .map(ToOwned::to_owned);
        report.protocol.daemon = details
            .get("daemon_protocol")
            .and_then(serde_json::Value::as_u64)
            .and_then(|value| u16::try_from(value).ok());
    }
}

fn format_service_state(state: ServiceStatusState) -> &'static str {
    match state {
        ServiceStatusState::NotInstalled => "not installed",
        ServiceStatusState::InstalledStopped => "installed/stopped",
        ServiceStatusState::Running => "running",
        ServiceStatusState::QueryFailed => "query failed",
    }
}

fn format_application_state(state: ApplicationStatusState) -> &'static str {
    match state {
        ApplicationStatusState::Ready => "ready",
        ApplicationStatusState::Unavailable => "application unavailable",
        ApplicationStatusState::ProtocolIncompatible => "protocol incompatible",
        ApplicationStatusState::Unknown => "unknown",
    }
}

fn format_sync_state(state: SyncStatusState) -> &'static str {
    match state {
        SyncStatusState::Connected => "connected",
        SyncStatusState::Connecting => "connecting",
        SyncStatusState::Offline => "offline",
        SyncStatusState::Unavailable => "unavailable",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flicknote_core::config::ConfigPaths;
    use flicknote_sync::ipc::ServerInfo;

    fn test_config(directory: &std::path::Path) -> Config {
        Config {
            supabase_url: String::new(),
            supabase_anon_key: String::new(),
            powersync_url: String::new(),
            api_url: String::new(),
            web_url: None,
            paths: ConfigPaths {
                config_dir: directory.to_path_buf(),
                data_dir: directory.to_path_buf(),
                config_file: directory.join("config.json"),
                session_file: directory.join("session.json"),
                db_file: directory.join("flicknote.db"),
                log_file: directory.join("flicknote.log"),
            },
        }
    }

    #[test]
    fn status_json_is_an_object_with_stable_state_fields() {
        let directory = tempfile::tempdir().unwrap();
        let report = StatusReport::base(&test_config(directory.path()));
        let value = serde_json::to_value(report).unwrap();
        assert!(value.is_object());
        for field in [
            "service_state",
            "application_state",
            "sync_state",
            "sync_errors",
            "daemon_executable",
            "version",
            "protocol",
            "error",
            "service_error",
            "log_guidance",
        ] {
            assert!(value.get(field).is_some(), "missing status field {field}");
        }
        assert_eq!(value["service_state"], "query_failed");
        assert_eq!(value["application_state"], "unknown");
        assert_eq!(value["sync_state"], "unavailable");
    }

    struct FakeHealth(Option<ServerInfo>);

    #[async_trait::async_trait]
    impl DaemonHealthProbe for FakeHealth {
        async fn health(&self, _config: &Config) -> Result<ServerInfo, ServiceError> {
            self.0
                .clone()
                .ok_or_else(|| ServiceError::DaemonUnavailable("missing".to_string()))
        }
    }

    #[tokio::test]
    async fn status_probes_application_even_when_service_is_not_installed() {
        let directory = tempfile::tempdir().unwrap();
        let config = test_config(directory.path());
        let report = build_status_report_with_probe(
            &config,
            Ok(ServiceState::NotInstalled),
            &FakeHealth(Some(ServerInfo::current())),
        )
        .await;
        assert_eq!(report.service_state, ServiceStatusState::NotInstalled);
        assert_eq!(report.application_state, ApplicationStatusState::Ready);
    }

    #[tokio::test]
    async fn status_reports_ready_for_a_healthy_foreground_daemon_while_service_is_stopped() {
        let directory = tempfile::tempdir().unwrap();
        let config = test_config(directory.path());
        let report = build_status_report_with_probe(
            &config,
            Ok(ServiceState::Stopped),
            &FakeHealth(Some(ServerInfo::current())),
        )
        .await;

        assert_eq!(report.service_state, ServiceStatusState::InstalledStopped);
        assert_eq!(report.application_state, ApplicationStatusState::Ready);
        assert!(report.is_ready());
    }

    #[tokio::test]
    async fn status_keeps_application_health_when_service_query_fails() {
        let directory = tempfile::tempdir().unwrap();
        let config = test_config(directory.path());
        let error = ServiceManagerError::new("query status", "unavailable");
        let report = build_status_report_with_probe(
            &config,
            Err(error),
            &FakeHealth(Some(ServerInfo::current())),
        )
        .await;
        assert_eq!(report.service_state, ServiceStatusState::QueryFailed);
        assert_eq!(report.application_state, ApplicationStatusState::Ready);
        assert!(report.service_error.is_some());
    }

    #[test]
    fn status_reports_connected_and_protocol_incompatible_as_explicit_states() {
        let directory = tempfile::tempdir().unwrap();
        let config = test_config(directory.path());
        let mut healthy = StatusReport::base(&config);
        healthy.service_state = ServiceStatusState::Running;
        healthy.application_state = ApplicationStatusState::Ready;
        healthy.sync_state = SyncStatusState::Connected;
        healthy.version.daemon = Some("0.9.0".to_string());
        healthy.protocol.daemon = Some(flicknote_sync::ipc::PROTOCOL_VERSION);
        let healthy_json = serde_json::to_value(healthy).unwrap();
        assert_eq!(healthy_json["application_state"], "ready");
        assert_eq!(healthy_json["sync_state"], "connected");

        let mut incompatible = StatusReport::base(&config);
        apply_health_error(
            &mut incompatible,
            ServiceError::Remote {
                code: flicknote_sync::ipc::PROTOCOL_MISMATCH_CODE.to_string(),
                message: "protocol mismatch".to_string(),
                retryable: false,
                details: Some(serde_json::json!({
                    "daemon_executable": "/opt/flicknote/bin/flicknote",
                    "daemon_version": "0.8.0",
                    "daemon_protocol": 2
                })),
            },
        );
        let incompatible_json = serde_json::to_value(incompatible).unwrap();
        assert_eq!(
            incompatible_json["application_state"],
            "protocol_incompatible"
        );
        assert_eq!(incompatible_json["protocol"]["daemon"], 2);
        assert_eq!(
            incompatible_json["daemon_executable"],
            "/opt/flicknote/bin/flicknote"
        );
    }

    #[tokio::test]
    async fn status_exposes_powersync_download_and_upload_errors() {
        let directory = tempfile::tempdir().unwrap();
        let config = test_config(directory.path());
        let info = ServerInfo::current().with_sync_status(
            flicknote_sync::ipc::SyncConnectionState::Offline,
            flicknote_sync::ipc::PowerSyncErrors {
                download: Some("download transport failed".to_string()),
                upload: Some("upload rejected".to_string()),
            },
        );

        let report = build_status_report_with_probe(
            &config,
            Ok(ServiceState::Running),
            &FakeHealth(Some(info)),
        )
        .await;

        let json = serde_json::to_value(&report).unwrap();
        assert_eq!(json["sync_errors"]["download"], "download transport failed");
        assert_eq!(json["sync_errors"]["upload"], "upload rejected");
        let verbose = report.verbose_text();
        assert!(verbose.contains("PowerSync download error: download transport failed"));
        assert!(verbose.contains("PowerSync upload error: upload rejected"));
    }

    #[test]
    fn unhealthy_concise_status_contains_recovery_commands() {
        let directory = tempfile::tempdir().unwrap();
        let mut report = StatusReport::base(&test_config(directory.path()));
        report.service_state = ServiceStatusState::InstalledStopped;
        report.application_state = ApplicationStatusState::Unavailable;
        let text = report.concise_text();
        assert!(text.contains("flicknote daemon status --verbose"));
        assert!(text.contains("flicknote daemon start"));
        assert_eq!(text.lines().count(), 1);
    }
}
