use async_trait::async_trait;
use flicknote_core::config::Config;
use flicknote_core::error::CliError;
use flicknote_core::services::error::ServiceError;
use flicknote_sync::ipc::{DaemonClient, PROTOCOL_MISMATCH_CODE, ServerInfo};
use std::time::Duration;

use super::service_manager::{
    NativeServiceFactory, ServiceManagerAdapter, ServiceManagerError, ServiceManagerFactory,
    ServiceState,
};

pub(crate) const SERVICE_OPERATION_TIMEOUT: Duration = Duration::from_secs(10);
const HEALTH_POLL_INTERVAL: Duration = Duration::from_millis(100);

#[async_trait]
pub(crate) trait DaemonHealthProbe: Send + Sync {
    async fn health(&self, config: &Config) -> Result<ServerInfo, ServiceError>;
}

pub(crate) struct IpcHealthProbe;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ServiceCleanup {
    NotInstalled,
    Removed,
}

#[async_trait]
pub(crate) trait DaemonLifecycle: Send + Sync {
    async fn install_and_wait(&self, config: &Config) -> Result<(), CliError>;
    async fn uninstall(&self, config: &Config) -> Result<ServiceCleanup, CliError>;
}

pub(crate) struct NativeDaemonLifecycle;

#[async_trait]
impl DaemonLifecycle for NativeDaemonLifecycle {
    async fn install_and_wait(&self, config: &Config) -> Result<(), CliError> {
        LifecycleController::native(&IpcHealthProbe)
            .install_and_wait(config)
            .await
    }

    async fn uninstall(&self, config: &Config) -> Result<ServiceCleanup, CliError> {
        LifecycleController::native(&IpcHealthProbe)
            .uninstall(config)
            .await
    }
}

#[async_trait]
impl DaemonHealthProbe for IpcHealthProbe {
    async fn health(&self, config: &Config) -> Result<ServerInfo, ServiceError> {
        DaemonClient::new(config).health().await
    }
}

pub(crate) struct LifecycleController<'a> {
    factory: &'a dyn ServiceManagerFactory,
    health: &'a dyn DaemonHealthProbe,
}

impl<'a> LifecycleController<'a> {
    pub(crate) fn new(
        factory: &'a dyn ServiceManagerFactory,
        health: &'a dyn DaemonHealthProbe,
    ) -> Self {
        Self { factory, health }
    }

    pub(crate) fn native(health: &'a dyn DaemonHealthProbe) -> Self {
        static FACTORY: NativeServiceFactory = NativeServiceFactory;
        Self::new(&FACTORY, health)
    }

    fn manager(&self, action: &'static str) -> Result<Box<dyn ServiceManagerAdapter>, CliError> {
        self.factory
            .manager()
            .map_err(|error| lifecycle_error(action, &error))
    }

    pub(crate) async fn install_and_wait(&self, config: &Config) -> Result<(), CliError> {
        self.stop_running(config, self.service_state("query")?)
            .await?;
        self.service_call("install", |manager| manager.install(config))?;
        self.service_call("reload", |manager| manager.reload())?;
        self.start_and_wait(config).await
    }

    pub(crate) async fn uninstall(&self, config: &Config) -> Result<ServiceCleanup, CliError> {
        self.uninstall_with_timeout(config, SERVICE_OPERATION_TIMEOUT)
            .await
    }

    async fn uninstall_with_timeout(
        &self,
        config: &Config,
        timeout: Duration,
    ) -> Result<ServiceCleanup, CliError> {
        let state = self.service_state("query")?;
        self.stop_running_with_timeout(config, state, timeout)
            .await?;
        if state == ServiceState::NotInstalled {
            return Ok(ServiceCleanup::NotInstalled);
        }
        self.service_call("uninstall", |manager| manager.uninstall())?;
        self.service_call("reload", |manager| manager.reload())?;
        self.wait_for_stopped(config, timeout).await?;
        Ok(ServiceCleanup::Removed)
    }

    pub(crate) async fn start(&self, config: &Config) -> Result<(), CliError> {
        self.ensure_installed("query")?;
        self.start_and_wait(config).await
    }

    pub(crate) async fn stop(&self, config: &Config) -> Result<bool, CliError> {
        let state = self.service_state("query")?;
        self.stop_running(config, state).await
    }

    pub(crate) async fn restart(&self, config: &Config) -> Result<(), CliError> {
        self.stop_running(config, self.ensure_installed("query")?)
            .await?;
        self.start_and_wait(config).await
    }

    fn ensure_installed(&self, action: &'static str) -> Result<ServiceState, CliError> {
        let state = self.service_state(action)?;
        if state == ServiceState::NotInstalled {
            return Err(CliError::Other(
                "FlickNote daemon service is not installed; run `flicknote daemon install`"
                    .to_string(),
            ));
        }
        Ok(state)
    }

    async fn start_and_wait(&self, config: &Config) -> Result<(), CliError> {
        self.start_and_wait_with_timeout(config, SERVICE_OPERATION_TIMEOUT)
            .await
    }

    async fn start_and_wait_with_timeout(
        &self,
        config: &Config,
        timeout: Duration,
    ) -> Result<(), CliError> {
        self.service_call("start", |manager| manager.start())?;
        self.wait_for_ready(config, timeout).await
    }

    async fn stop_running(&self, config: &Config, state: ServiceState) -> Result<bool, CliError> {
        self.stop_running_with_timeout(config, state, SERVICE_OPERATION_TIMEOUT)
            .await
    }

    async fn stop_running_with_timeout(
        &self,
        config: &Config,
        state: ServiceState,
        timeout: Duration,
    ) -> Result<bool, CliError> {
        let was_running = state == ServiceState::Running;
        if was_running {
            self.service_call("stop", |manager| manager.stop())?;
        }
        self.wait_for_stopped(config, timeout).await?;
        Ok(was_running)
    }

    fn service_state(&self, action: &'static str) -> Result<ServiceState, CliError> {
        let manager = self.manager(action)?;
        manager
            .status()
            .map_err(|error| lifecycle_error(action, &error))
    }

    fn service_call(
        &self,
        action: &'static str,
        operation: impl FnOnce(&dyn ServiceManagerAdapter) -> Result<(), ServiceManagerError>,
    ) -> Result<(), CliError> {
        let manager = self.manager(action)?;
        operation(&*manager).map_err(|error| lifecycle_error(action, &error))
    }

    async fn wait_for_ready(&self, config: &Config, timeout: Duration) -> Result<(), CliError> {
        let wait = async {
            loop {
                match self.service_state("confirm running")? {
                    ServiceState::Running => {}
                    ServiceState::Stopped => {
                        // launchd can report a freshly submitted agent as stopped
                        // while it is still being scheduled; keep polling within
                        // the operation timeout instead of failing immediately.
                        tokio::time::sleep(HEALTH_POLL_INTERVAL).await;
                        continue;
                    }
                    ServiceState::NotInstalled => {
                        return Err(CliError::Other(
                            "FlickNote daemon service was removed before becoming ready; run `flicknote daemon install`"
                                .to_string(),
                        ));
                    }
                }

                match self.health.health(config).await {
                    Ok(_) => {
                        tokio::time::sleep(HEALTH_POLL_INTERVAL).await;
                        if self.service_state("confirm running")? == ServiceState::Running
                            && self.health.health(config).await.is_ok()
                        {
                            return Ok(());
                        }
                    }
                    Err(error) if error.code() == PROTOCOL_MISMATCH_CODE => {
                        return Err(CliError::Other(error.to_string()));
                    }
                    Err(error) if !error.retryable() => return Err(CliError::from(error)),
                    Err(_) => tokio::time::sleep(HEALTH_POLL_INTERVAL).await,
                }
            }
        };
        tokio::time::timeout(timeout, wait).await.map_err(|_| {
            CliError::Other(format!(
                "FlickNote daemon did not become ready within {timeout:?}; run `flicknote daemon status --verbose`"
            ))
        })?
    }

    async fn wait_for_stopped(&self, config: &Config, timeout: Duration) -> Result<(), CliError> {
        let wait = async {
            loop {
                match self.health.health(config).await {
                    Err(error) if error.code() == "daemon_unavailable" => return Ok(()),
                    _ => tokio::time::sleep(HEALTH_POLL_INTERVAL).await,
                }
            }
        };
        tokio::time::timeout(timeout, wait).await.map_err(|_| {
            CliError::Other(format!(
                "FlickNote daemon did not stop within {timeout:?}; run `flicknote daemon status --verbose`"
            ))
        })?
    }
}

pub(crate) fn lifecycle_error(action: &str, error: &ServiceManagerError) -> CliError {
    log::error!("FlickNote daemon service {action} operation failed: {error}");
    let guidance = match action {
        "start" | "install" => {
            "run `flicknote daemon status --verbose` and retry `flicknote daemon install`"
        }
        "stop" | "uninstall" => "run `flicknote daemon status --verbose` before retrying cleanup",
        _ => "run `flicknote daemon status --verbose`",
    };
    CliError::Other(format!(
        "Could not {action} the FlickNote daemon service; {guidance}"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use flicknote_core::services::error::ServiceError;
    use std::collections::VecDeque;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex, Once};

    static TEST_LOGGER: LifecycleTestLogger = LifecycleTestLogger;
    static TEST_LOGGER_INIT: Once = Once::new();
    static TEST_LOG_RECORDS: Mutex<Vec<(log::Level, String)>> = Mutex::new(Vec::new());

    struct LifecycleTestLogger;

    impl log::Log for LifecycleTestLogger {
        fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
            metadata.level() == log::Level::Error
                && metadata.target().ends_with("commands::daemon_lifecycle")
        }

        fn log(&self, record: &log::Record<'_>) {
            if self.enabled(record.metadata()) {
                TEST_LOG_RECORDS
                    .lock()
                    .unwrap()
                    .push((record.level(), record.args().to_string()));
            }
        }

        fn flush(&self) {}
    }

    fn enable_test_logger() {
        TEST_LOGGER_INIT.call_once(|| {
            log::set_logger(&TEST_LOGGER)
                .expect("test logger should be the first logger installed");
            log::set_max_level(log::LevelFilter::Error);
        });
    }

    struct FakeManager {
        state: Mutex<ServiceState>,
        start_state: Mutex<ServiceState>,
        calls: Mutex<Vec<&'static str>>,
        fail: Mutex<Option<&'static str>>,
        status_sequence: Mutex<VecDeque<ServiceState>>,
    }

    impl FakeManager {
        fn new(state: ServiceState) -> Self {
            Self {
                state: Mutex::new(state),
                start_state: Mutex::new(ServiceState::Running),
                calls: Mutex::new(Vec::new()),
                fail: Mutex::new(None),
                status_sequence: Mutex::new(VecDeque::new()),
            }
        }

        fn calls(&self) -> Vec<&'static str> {
            self.calls.lock().unwrap().clone()
        }

        fn operation(&self, name: &'static str) -> Result<(), ServiceManagerError> {
            self.calls.lock().unwrap().push(name);
            if self.fail.lock().unwrap().as_ref() == Some(&name) {
                return Err(ServiceManagerError::new(name, "forced failure"));
            }
            Ok(())
        }
    }

    impl ServiceManagerAdapter for FakeManager {
        fn status(&self) -> Result<ServiceState, ServiceManagerError> {
            self.operation("status")?;
            Ok(self
                .status_sequence
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or(*self.state.lock().unwrap()))
        }

        fn install(&self, _config: &Config) -> Result<(), ServiceManagerError> {
            self.operation("install")?;
            *self.state.lock().unwrap() = ServiceState::Stopped;
            Ok(())
        }

        fn reload(&self) -> Result<(), ServiceManagerError> {
            self.operation("reload")
        }

        fn start(&self) -> Result<(), ServiceManagerError> {
            self.operation("start")?;
            *self.state.lock().unwrap() = *self.start_state.lock().unwrap();
            Ok(())
        }

        fn stop(&self) -> Result<(), ServiceManagerError> {
            self.operation("stop")?;
            *self.state.lock().unwrap() = ServiceState::Stopped;
            Ok(())
        }

        fn uninstall(&self) -> Result<(), ServiceManagerError> {
            self.operation("uninstall")?;
            *self.state.lock().unwrap() = ServiceState::NotInstalled;
            Ok(())
        }
    }

    struct FakeHealth {
        results: Mutex<VecDeque<Result<ServerInfo, ServiceError>>>,
        polls: AtomicUsize,
    }

    impl FakeHealth {
        fn new(results: impl IntoIterator<Item = Result<ServerInfo, ServiceError>>) -> Self {
            Self {
                results: Mutex::new(results.into_iter().collect()),
                polls: AtomicUsize::new(0),
            }
        }

        fn polls(&self) -> usize {
            self.polls.load(Ordering::SeqCst)
        }
    }

    #[async_trait]
    impl DaemonHealthProbe for FakeHealth {
        async fn health(&self, _config: &Config) -> Result<ServerInfo, ServiceError> {
            self.polls.fetch_add(1, Ordering::SeqCst);
            self.results
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or_else(|| Ok(ServerInfo::current()))
        }
    }

    fn test_config() -> (tempfile::TempDir, Config) {
        let directory = tempfile::tempdir().unwrap();
        let config = Config {
            supabase_url: String::new(),
            supabase_anon_key: String::new(),
            powersync_url: String::new(),
            api_url: String::new(),
            gateway_url: String::new(),
            web_url: None,
            paths: flicknote_core::config::ConfigPaths {
                config_dir: directory.path().to_path_buf(),
                data_dir: directory.path().to_path_buf(),
                config_file: directory.path().join("config.json"),
                session_file: directory.path().join("session.json"),
                db_file: directory.path().join("flicknote.db"),
                log_file: directory.path().join("flicknote.log"),
            },
        };
        (directory, config)
    }

    fn unavailable() -> ServiceError {
        ServiceError::DaemonUnavailable("stopped".to_string())
    }

    struct FakeFactory(Arc<FakeManager>);

    impl ServiceManagerFactory for FakeFactory {
        fn manager(&self) -> Result<Box<dyn ServiceManagerAdapter>, ServiceManagerError> {
            Ok(Box::new(FakeAdapter(Arc::clone(&self.0))))
        }
    }

    struct FakeAdapter(Arc<FakeManager>);

    impl ServiceManagerAdapter for FakeAdapter {
        fn status(&self) -> Result<ServiceState, ServiceManagerError> {
            self.0.status()
        }
        fn install(&self, config: &Config) -> Result<(), ServiceManagerError> {
            self.0.install(config)
        }
        fn reload(&self) -> Result<(), ServiceManagerError> {
            self.0.reload()
        }
        fn start(&self) -> Result<(), ServiceManagerError> {
            self.0.start()
        }
        fn stop(&self) -> Result<(), ServiceManagerError> {
            self.0.stop()
        }
        fn uninstall(&self) -> Result<(), ServiceManagerError> {
            self.0.uninstall()
        }
    }

    #[tokio::test]
    async fn install_reconciles_reload_start_and_waits_for_readiness() {
        let (_directory, config) = test_config();
        let manager = Arc::new(FakeManager::new(ServiceState::Stopped));
        let factory = FakeFactory(Arc::clone(&manager));
        let health = FakeHealth::new([
            Err(unavailable()),
            Ok(ServerInfo::current()),
            Ok(ServerInfo::current()),
        ]);
        LifecycleController::new(&factory, &health)
            .install_and_wait(&config)
            .await
            .unwrap();
        assert_eq!(
            manager.calls(),
            vec!["status", "install", "reload", "start", "status", "status"]
        );
    }

    #[tokio::test]
    async fn uninstall_stops_waits_then_removes_and_reloads() {
        let (_directory, config) = test_config();
        let manager = Arc::new(FakeManager::new(ServiceState::Running));
        let factory = FakeFactory(Arc::clone(&manager));
        let health = FakeHealth::new([Err(unavailable()), Err(unavailable())]);
        assert_eq!(
            LifecycleController::new(&factory, &health)
                .uninstall(&config)
                .await
                .unwrap(),
            ServiceCleanup::Removed
        );
        assert_eq!(
            manager.calls(),
            vec!["status", "stop", "uninstall", "reload"]
        );
    }

    #[tokio::test]
    async fn start_rejects_an_unrelated_healthy_daemon_when_the_service_does_not_run() {
        let (_directory, config) = test_config();
        let manager = Arc::new(FakeManager::new(ServiceState::Stopped));
        *manager.start_state.lock().unwrap() = ServiceState::Stopped;
        let factory = FakeFactory(Arc::clone(&manager));
        let health = FakeHealth::new([Ok(ServerInfo::current())]);

        let error = LifecycleController::new(&factory, &health)
            .start_and_wait_with_timeout(&config, Duration::from_millis(250))
            .await
            .unwrap_err();

        assert!(error.to_string().contains("did not become ready"));
        assert_eq!(health.polls(), 0);
        assert!(manager.calls().starts_with(&["start", "status"]));
    }

    #[tokio::test]
    async fn install_tolerates_transient_stopped_and_retryable_ipc_failure_before_readiness() {
        let (_directory, config) = test_config();
        let manager = Arc::new(FakeManager::new(ServiceState::Stopped));
        *manager.status_sequence.lock().unwrap() = VecDeque::from([
            ServiceState::Stopped,
            ServiceState::Stopped,
            ServiceState::Stopped,
        ]);
        let factory = FakeFactory(Arc::clone(&manager));
        let health = FakeHealth::new([
            Err(unavailable()),
            Err(unavailable()),
            Ok(ServerInfo::current()),
            Ok(ServerInfo::current()),
        ]);

        LifecycleController::new(&factory, &health)
            .install_and_wait(&config)
            .await
            .unwrap();

        assert!(
            manager
                .calls()
                .starts_with(&["status", "install", "reload", "start"])
        );
        assert!(
            manager
                .calls()
                .iter()
                .filter(|call| **call == "status")
                .count()
                >= 4,
            "readiness polling must keep observing transient states within the timeout"
        );
    }

    #[tokio::test]
    async fn wait_for_ready_treats_not_installed_as_terminal_after_start() {
        let (_directory, config) = test_config();
        let manager = Arc::new(FakeManager::new(ServiceState::Running));
        *manager.status_sequence.lock().unwrap() =
            VecDeque::from([ServiceState::Stopped, ServiceState::NotInstalled]);
        let factory = FakeFactory(Arc::clone(&manager));
        let health = FakeHealth::new([]);

        let error = LifecycleController::new(&factory, &health)
            .start_and_wait_with_timeout(&config, Duration::from_millis(500))
            .await
            .unwrap_err();

        assert!(
            error
                .to_string()
                .contains("was removed before becoming ready")
        );
    }

    #[tokio::test]
    async fn wait_for_ready_treats_protocol_mismatch_as_terminal() {
        let (_directory, config) = test_config();
        let manager = Arc::new(FakeManager::new(ServiceState::Running));
        let factory = FakeFactory(Arc::clone(&manager));
        let mismatch = ServiceError::Remote {
            code: PROTOCOL_MISMATCH_CODE.to_string(),
            message: "daemon protocol mismatch: CLI v4 vs daemon v3".to_string(),
            retryable: true,
            details: None,
        };
        let health = FakeHealth::new([Err(mismatch)]);

        let error = LifecycleController::new(&factory, &health)
            .start_and_wait_with_timeout(&config, Duration::from_millis(500))
            .await
            .unwrap_err();

        assert!(error.to_string().contains("protocol mismatch"));
        assert_eq!(manager.calls(), vec!["start", "status"]);
    }

    #[tokio::test]
    async fn wait_for_ready_returns_non_retryable_ipc_errors_immediately() {
        let (_directory, config) = test_config();
        let manager = Arc::new(FakeManager::new(ServiceState::Running));
        let factory = FakeFactory(Arc::clone(&manager));
        let health = FakeHealth::new([Err(ServiceError::Daemon(
            "IPC socket closed unexpectedly".to_string(),
        ))]);

        let error = LifecycleController::new(&factory, &health)
            .start_and_wait_with_timeout(&config, Duration::from_millis(500))
            .await
            .unwrap_err();

        assert!(error.to_string().contains("IPC socket closed unexpectedly"));
        assert_eq!(manager.calls(), vec!["start", "status"]);
    }

    #[tokio::test]
    async fn start_does_not_install_missing_service() {
        let (_directory, config) = test_config();
        let manager = Arc::new(FakeManager::new(ServiceState::NotInstalled));
        let factory = FakeFactory(Arc::clone(&manager));
        let health = FakeHealth::new([]);
        assert!(
            LifecycleController::new(&factory, &health)
                .start(&config)
                .await
                .is_err()
        );
        assert_eq!(manager.calls(), vec!["status"]);
    }

    #[tokio::test]
    async fn install_creates_a_missing_service_then_reloads_and_starts_it() {
        let (_directory, config) = test_config();
        let manager = Arc::new(FakeManager::new(ServiceState::NotInstalled));
        let factory = FakeFactory(Arc::clone(&manager));
        let health = FakeHealth::new([
            Err(unavailable()),
            Ok(ServerInfo::current()),
            Ok(ServerInfo::current()),
        ]);

        LifecycleController::new(&factory, &health)
            .install_and_wait(&config)
            .await
            .unwrap();

        assert_eq!(
            manager.calls(),
            vec!["status", "install", "reload", "start", "status", "status"]
        );
    }

    #[tokio::test]
    async fn install_replaces_a_running_service_before_reinstalling() {
        let (_directory, config) = test_config();
        let manager = Arc::new(FakeManager::new(ServiceState::Running));
        let factory = FakeFactory(Arc::clone(&manager));
        let health = FakeHealth::new([
            Err(unavailable()),
            Ok(ServerInfo::current()),
            Ok(ServerInfo::current()),
        ]);

        LifecycleController::new(&factory, &health)
            .install_and_wait(&config)
            .await
            .unwrap();

        assert_eq!(
            manager.calls(),
            vec![
                "status", "stop", "install", "reload", "start", "status", "status"
            ]
        );
    }

    #[tokio::test]
    async fn install_reload_failure_is_returned_before_start() {
        let (_directory, config) = test_config();
        let manager = Arc::new(FakeManager::new(ServiceState::Stopped));
        *manager.fail.lock().unwrap() = Some("reload");
        let factory = FakeFactory(Arc::clone(&manager));
        let health = FakeHealth::new([Err(unavailable())]);

        assert!(
            LifecycleController::new(&factory, &health)
                .install_and_wait(&config)
                .await
                .is_err()
        );
        assert_eq!(manager.calls(), vec!["status", "install", "reload"]);
    }

    #[tokio::test]
    async fn start_stop_and_restart_reconcile_without_installing() {
        let (_directory, config) = test_config();
        let manager = Arc::new(FakeManager::new(ServiceState::Stopped));
        let factory = FakeFactory(Arc::clone(&manager));
        let health = FakeHealth::new([
            Ok(ServerInfo::current()),
            Ok(ServerInfo::current()),
            Err(unavailable()),
            Err(unavailable()),
            Ok(ServerInfo::current()),
            Ok(ServerInfo::current()),
        ]);
        let controller = LifecycleController::new(&factory, &health);

        controller.start(&config).await.unwrap();
        assert!(controller.stop(&config).await.unwrap());
        controller.restart(&config).await.unwrap();

        assert_eq!(
            manager.calls(),
            vec![
                "status", "start", "status", "status", "status", "stop", "status", "start",
                "status", "status"
            ]
        );
    }

    #[tokio::test]
    async fn restart_stops_a_running_service_before_starting() {
        let (_directory, config) = test_config();
        let manager = Arc::new(FakeManager::new(ServiceState::Running));
        let factory = FakeFactory(Arc::clone(&manager));
        let health = FakeHealth::new([
            Err(unavailable()),
            Ok(ServerInfo::current()),
            Ok(ServerInfo::current()),
        ]);

        LifecycleController::new(&factory, &health)
            .restart(&config)
            .await
            .unwrap();

        assert_eq!(
            manager.calls(),
            vec!["status", "stop", "start", "status", "status"]
        );
    }

    #[tokio::test]
    async fn uninstall_skips_mutations_when_service_is_absent() {
        let (_directory, config) = test_config();
        let manager = Arc::new(FakeManager::new(ServiceState::NotInstalled));
        let factory = FakeFactory(Arc::clone(&manager));
        let health = FakeHealth::new([Err(unavailable())]);

        assert_eq!(
            LifecycleController::new(&factory, &health)
                .uninstall(&config)
                .await
                .unwrap(),
            ServiceCleanup::NotInstalled
        );
        assert_eq!(manager.calls(), vec!["status"]);
    }

    #[tokio::test]
    async fn uninstall_reload_failure_is_propagated_after_removal() {
        let (_directory, config) = test_config();
        let manager = Arc::new(FakeManager::new(ServiceState::Stopped));
        *manager.fail.lock().unwrap() = Some("reload");
        let factory = FakeFactory(Arc::clone(&manager));
        let health = FakeHealth::new([Err(unavailable())]);

        assert!(
            LifecycleController::new(&factory, &health)
                .uninstall(&config)
                .await
                .is_err()
        );
        assert_eq!(manager.calls(), vec!["status", "uninstall", "reload"]);
    }

    #[tokio::test]
    async fn stopped_service_does_not_hide_a_foreground_daemon_during_stop() {
        let (_directory, config) = test_config();
        let manager = Arc::new(FakeManager::new(ServiceState::Stopped));
        let factory = FakeFactory(Arc::clone(&manager));
        let health = FakeHealth::new([Ok(ServerInfo::current())]);

        let error = LifecycleController::new(&factory, &health)
            .stop_running_with_timeout(&config, ServiceState::Stopped, Duration::from_millis(10))
            .await
            .unwrap_err();

        assert!(error.to_string().contains("did not stop"));
        assert!(manager.calls().is_empty());
    }

    #[tokio::test]
    async fn absent_service_does_not_hide_a_foreground_daemon_during_uninstall() {
        let (_directory, config) = test_config();
        let manager = Arc::new(FakeManager::new(ServiceState::NotInstalled));
        let factory = FakeFactory(Arc::clone(&manager));
        let health = FakeHealth::new([Ok(ServerInfo::current())]);

        let error = LifecycleController::new(&factory, &health)
            .uninstall_with_timeout(&config, Duration::from_millis(10))
            .await
            .unwrap_err();

        assert!(error.to_string().contains("did not stop"));
        assert_eq!(manager.calls(), vec!["status"]);
    }

    #[test]
    fn lifecycle_errors_use_flicknote_guidance_and_log_platform_diagnostics() {
        enable_test_logger();
        let platform = ServiceManagerError::new(
            "start",
            "launchctl bootstrap failed: Input/output error (code 5)",
        );

        let message = lifecycle_error("start", &platform).to_string();

        assert_eq!(
            message,
            "Could not start the FlickNote daemon service; run `flicknote daemon status --verbose` and retry `flicknote daemon install`"
        );
        assert!(TEST_LOG_RECORDS.lock().unwrap().iter().any(|record| {
            record.0 == log::Level::Error
                && record.1.contains("launchctl bootstrap failed")
                && record.1.contains("Input/output error (code 5)")
        }));
    }

    #[tokio::test]
    async fn cleanup_failure_stops_before_uninstall_is_attempted() {
        let (_directory, config) = test_config();
        let manager = Arc::new(FakeManager::new(ServiceState::Running));
        *manager.fail.lock().unwrap() = Some("stop");
        let factory = FakeFactory(Arc::clone(&manager));
        let health = FakeHealth::new([]);
        assert!(
            LifecycleController::new(&factory, &health)
                .uninstall(&config)
                .await
                .is_err()
        );
        assert_eq!(manager.calls(), vec!["status", "stop"]);
    }
}
