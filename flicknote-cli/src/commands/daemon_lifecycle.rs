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
        let state = self.service_state("query")?;
        self.stop_running(config, state).await?;
        self.service_call("install", |manager| manager.install(config))?;
        self.service_call("reload", |manager| manager.reload())?;
        self.service_call("start", |manager| manager.start())?;
        self.wait_for_ready(config, SERVICE_OPERATION_TIMEOUT).await
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
        if self.service_state("query")? == ServiceState::NotInstalled {
            return Err(CliError::Other(
                "FlickNote daemon service is not installed; run `flicknote daemon install`"
                    .to_string(),
            ));
        }
        self.service_call("start", |manager| manager.start())?;
        self.wait_for_ready(config, SERVICE_OPERATION_TIMEOUT).await
    }

    pub(crate) async fn stop(&self, config: &Config) -> Result<bool, CliError> {
        let state = self.service_state("query")?;
        self.stop_running(config, state).await
    }

    pub(crate) async fn restart(&self, config: &Config) -> Result<(), CliError> {
        let state = self.service_state("query")?;
        if state == ServiceState::NotInstalled {
            return Err(CliError::Other(
                "FlickNote daemon service is not installed; run `flicknote daemon install`"
                    .to_string(),
            ));
        }
        self.stop_running(config, state).await?;
        self.service_call("start", |manager| manager.start())?;
        self.wait_for_ready(config, SERVICE_OPERATION_TIMEOUT).await
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
                        return Err(CliError::Other(
                            "FlickNote daemon service stopped before becoming ready; run `flicknote daemon status --verbose`"
                                .to_string(),
                        ));
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
    CliError::Other(format!(
        "Could not {action} the FlickNote daemon service: {error}; run `flicknote daemon status --verbose` for diagnosis"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use flicknote_core::services::error::ServiceError;
    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex};

    struct FakeManager {
        state: Mutex<ServiceState>,
        start_state: Mutex<ServiceState>,
        calls: Mutex<Vec<&'static str>>,
        fail: Mutex<Option<&'static str>>,
    }

    impl FakeManager {
        fn new(state: ServiceState) -> Self {
            Self {
                state: Mutex::new(state),
                start_state: Mutex::new(ServiceState::Running),
                calls: Mutex::new(Vec::new()),
                fail: Mutex::new(None),
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
            Ok(*self.state.lock().unwrap())
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
    }

    impl FakeHealth {
        fn new(results: impl IntoIterator<Item = Result<ServerInfo, ServiceError>>) -> Self {
            Self {
                results: Mutex::new(results.into_iter().collect()),
            }
        }
    }

    #[async_trait]
    impl DaemonHealthProbe for FakeHealth {
        async fn health(&self, _config: &Config) -> Result<ServerInfo, ServiceError> {
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
            .start(&config)
            .await
            .unwrap_err();

        assert!(error.to_string().contains("service stopped"));
        assert_eq!(manager.calls(), vec!["status", "start", "status"]);
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
    fn lifecycle_errors_preserve_platform_diagnostics() {
        let platform = ServiceManagerError::new(
            "start",
            "launchctl bootstrap failed: Input/output error (code 5)",
        );

        let message = lifecycle_error("start", &platform).to_string();

        assert!(message.contains("launchctl bootstrap failed"));
        assert!(message.contains("Input/output error (code 5)"));
        assert!(message.contains("flicknote daemon status --verbose"));
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
