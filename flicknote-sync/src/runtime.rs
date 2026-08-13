use std::future::Future;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use flicknote_auth::client::GoTrueClient;
use flicknote_core::{
    backend::{LocalPowerSyncBackend, NoteDb},
    config::Config,
    schema::app_schema,
    services::ports::{NoteCreator, ShareGateway},
};
use powersync::{ConnectionPool, PowerSyncDatabase, SyncOptions, env::PowerSyncEnvironment};
use tokio::net::UnixListener;
use tokio::sync::watch;
use tokio::task::{JoinHandle, JoinSet};

#[cfg(unix)]
use tokio::signal::unix::SignalKind;

use crate::app::Application;
use crate::ipc;
use crate::ownership::{DataDirectoryLock, OwnershipError};
use crate::remote::{RemoteNoteCreator, RemoteShareGateway};
use crate::storage_maintenance::{WalCheckpointMode, checkpoint_wal_standalone_with_timeout};
use crate::upload::FlickNoteConnector;

const IPC_DRAIN_TIMEOUT: Duration = Duration::from_secs(2);
const POWERSYNC_DISCONNECT_TIMEOUT: Duration = Duration::from_secs(4);
const WAL_CHECKPOINT_TIMEOUT: Duration = Duration::from_secs(2);

struct SocketGuard(PathBuf);

impl Drop for SocketGuard {
    fn drop(&mut self) {
        if let Err(error) = std::fs::remove_file(&self.0)
            && error.kind() != std::io::ErrorKind::NotFound
        {
            log::warn!(
                "Failed to remove daemon socket {}: {error}",
                self.0.display()
            );
        }
    }
}

fn bind_socket(config: &Config) -> Result<(UnixListener, SocketGuard), Box<dyn std::error::Error>> {
    let path = ipc::socket_path(config);
    if path.exists() {
        std::fs::remove_file(&path)?;
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let listener = UnixListener::bind(&path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok((listener, SocketGuard(path)))
}

struct ActorHandles {
    checkpoint: JoinHandle<()>,
    socket: JoinHandle<Result<(), ipc::DaemonError>>,
    powersync: JoinSet<()>,
}

struct StartupSignals {
    receiver: watch::Receiver<bool>,
    task: JoinHandle<()>,
}

impl StartupSignals {
    fn register() -> Result<Self, std::io::Error> {
        let (sender, receiver) = watch::channel(false);
        #[cfg(unix)]
        {
            let mut interrupt = tokio::signal::unix::signal(SignalKind::interrupt())?;
            let mut terminate = tokio::signal::unix::signal(SignalKind::terminate())?;
            let task = tokio::spawn(async move {
                tokio::select! {
                    _ = interrupt.recv() => {}
                    _ = terminate.recv() => {}
                }
                if sender.send(true).is_err() {
                    log::debug!("daemon startup signal receiver was dropped");
                }
            });
            Ok(Self { receiver, task })
        }
        #[cfg(not(unix))]
        {
            let task = tokio::spawn(std::future::pending::<()>());
            Ok(Self { receiver, task })
        }
    }

    fn requested(&self) -> bool {
        *self.receiver.borrow()
    }

    async fn wait(&self) {
        let mut receiver = self.receiver.clone();
        if *receiver.borrow() {
            return;
        }
        if receiver.changed().await.is_err() {
            log::debug!("daemon startup signal watcher was dropped");
        }
    }
}

impl Drop for StartupSignals {
    fn drop(&mut self) {
        self.task.abort();
    }
}

#[derive(Debug, thiserror::Error)]
pub enum DaemonRunError {
    #[error("{0}")]
    PermanentStartup(String),
    #[error("{0}")]
    OwnershipConflict(String),
    #[error("{0}")]
    Startup(String),
    #[error("{0}")]
    Runtime(String),
}

impl DaemonRunError {
    pub fn is_permanent_startup(&self) -> bool {
        matches!(self, Self::PermanentStartup(_))
    }
}

/// Run the daemon synchronously in the caller's process.
///
/// The caller owns the process lifetime. This function never forks, detaches,
/// creates a session, or redirects terminal output.
pub async fn run(config: Config) -> Result<(), DaemonRunError> {
    let startup_signals =
        StartupSignals::register().map_err(|error| DaemonRunError::Startup(error.to_string()))?;
    config
        .validate()
        .map_err(|error| DaemonRunError::PermanentStartup(error.to_string()))?;
    // Authentication is checked before the ownership lock, socket, and database
    // so an unauthenticated invocation cannot create persistent daemon state.
    flicknote_core::session::get_user_id(&config)
        .map_err(|error| DaemonRunError::PermanentStartup(error.to_string()))?;
    if startup_signals.requested() {
        return Ok(());
    }

    let config = Arc::new(config);
    let _ownership =
        DataDirectoryLock::acquire(&config.paths.data_dir).map_err(|error| match error {
            OwnershipError::AlreadyOwned { .. } => {
                DaemonRunError::OwnershipConflict(error.to_string())
            }
            OwnershipError::Io(_) => DaemonRunError::Startup(error.to_string()),
        })?;
    if startup_signals.requested() {
        return Ok(());
    }
    let db = open_powersync_database(&config)
        .map_err(|error| DaemonRunError::Startup(error.to_string()))?;
    let mut powersync_tasks = spawn_powersync_actors(&db);

    if startup_checkpoint(config.paths.db_file.clone(), &startup_signals).await {
        shutdown_startup(&mut powersync_tasks, &db, config.paths.db_file.clone()).await;
        return Ok(());
    }
    // Force PowerSync's local initialization before advertising IPC readiness.
    let reader = tokio::select! {
        reader = db.reader() => reader
            .map_err(|error| DaemonRunError::PermanentStartup(error.to_string()))?,
        _signal = startup_signals.wait() => {
            log::info!("Shutdown signal received during daemon startup");
            shutdown_startup(&mut powersync_tasks, &db, config.paths.db_file.clone()).await;
            return Ok(());
        }
    };
    drop(reader);

    let auth = Arc::new(GoTrueClient::new(
        &config.supabase_url,
        &config.supabase_anon_key,
        &config.paths.session_file,
    ));
    let backend = open_local_backend(&db, &config)
        .map_err(|error| DaemonRunError::PermanentStartup(error.to_string()))?;
    let app = build_application(backend, &db, &auth, &config);
    let (socket_listener, _socket_guard) =
        bind_socket(&config).map_err(|error| DaemonRunError::Startup(error.to_string()))?;

    log::info!("FlickNote daemon initialized (pid {})", std::process::id());
    tokio::select! {
        _ = db.connect(SyncOptions::new(build_connector(&db, &auth, &config))) => {}
        _signal = startup_signals.wait() => {
            log::info!("Shutdown signal received during daemon startup");
            shutdown_startup(&mut powersync_tasks, &db, config.paths.db_file.clone()).await;
            return Ok(());
        }
    }
    log::info!("FlickNote daemon accepting local requests");

    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let socket = spawn_socket_server(socket_listener, app, &db, shutdown_rx);
    let mut actors = ActorHandles {
        checkpoint: spawn_checkpoint_worker(config.paths.db_file.clone()),
        socket,
        powersync: powersync_tasks,
    };

    let result = wait_for_shutdown(&mut actors, &startup_signals).await;
    shutdown_daemon(&mut actors, &db, config.paths.db_file.clone(), &shutdown_tx).await;
    result.map_err(DaemonRunError::Runtime)
}

fn spawn_powersync_actors(db: &PowerSyncDatabase) -> JoinSet<()> {
    let mut actors = JoinSet::new();
    let abort_handles = db.async_tasks().spawn_with(|future| actors.spawn(future));
    drop(abort_handles);
    actors
}

fn open_powersync_database(
    config: &Config,
) -> Result<PowerSyncDatabase, Box<dyn std::error::Error>> {
    PowerSyncEnvironment::powersync_auto_extension()?;
    let pool = ConnectionPool::open(&config.paths.db_file)?;
    let environment = PowerSyncEnvironment::custom(
        reqwest::Client::new(),
        pool,
        PowerSyncEnvironment::tokio_timer(),
    );
    let db = PowerSyncDatabase::new(environment, app_schema());
    Ok(db)
}

fn build_connector(
    db: &PowerSyncDatabase,
    auth: &Arc<GoTrueClient>,
    config: &Config,
) -> FlickNoteConnector {
    FlickNoteConnector {
        db: db.clone(),
        auth: Arc::clone(auth),
        http_client: reqwest::Client::new(),
        powersync_url: config.powersync_url.clone(),
        supabase_url: config.supabase_url.clone(),
        supabase_anon_key: config.supabase_anon_key.clone(),
    }
}

async fn startup_checkpoint(db_path: PathBuf, signals: &StartupSignals) -> bool {
    log::info!("Running startup WAL checkpoint");
    let task = match spawn_wal_checkpoint(db_path, "startup", WalCheckpointMode::Truncate, 1_000) {
        Ok(task) => task,
        Err(error) => {
            log::warn!("Startup WAL checkpoint could not start: {error}");
            return false;
        }
    };
    tokio::pin!(task);
    tokio::select! {
        result = &mut task => {
            if result.is_err() {
                log::warn!("Startup WAL checkpoint worker ended before reporting completion");
            }
            false
        }
        _ = signals.wait() => {
            log::info!("Shutdown signal received during startup WAL checkpoint");
            true
        }
        _ = tokio::time::sleep(WAL_CHECKPOINT_TIMEOUT) => {
            log::warn!("Startup WAL checkpoint exceeded its budget");
            false
        }
    }
}

fn open_local_backend(
    db: &PowerSyncDatabase,
    config: &Config,
) -> Result<Arc<dyn NoteDb>, Box<dyn std::error::Error>> {
    let user_id = flicknote_core::session::get_user_id(config)?;
    Ok(Arc::new(LocalPowerSyncBackend::new(db.clone(), user_id)))
}

fn build_application(
    backend: Arc<dyn NoteDb>,
    db: &PowerSyncDatabase,
    auth: &Arc<GoTrueClient>,
    config: &Arc<Config>,
) -> Arc<Application> {
    let http = reqwest::Client::new();
    let creator: Arc<dyn NoteCreator> = Arc::new(RemoteNoteCreator::new(
        db.clone(),
        Arc::clone(auth),
        http.clone(),
        Arc::clone(config),
    ));
    let gateway: Arc<dyn ShareGateway> = Arc::new(RemoteShareGateway::new(
        http,
        Arc::clone(auth),
        Arc::clone(config),
    ));
    Arc::new(Application::new(backend, creator, gateway).with_web_url(config.web_url.clone()))
}

fn spawn_checkpoint_worker(db_path: PathBuf) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(30));
        interval.tick().await;
        loop {
            interval.tick().await;
            match spawn_wal_checkpoint(
                db_path.clone(),
                "periodic",
                WalCheckpointMode::Passive,
                5_000,
            ) {
                Ok(done) => {
                    if let Err(error) = done.await {
                        log::error!("Periodic WAL checkpoint task failed: {error}");
                    }
                }
                Err(error) => log::error!("Periodic WAL checkpoint could not start: {error}"),
            }
        }
    })
}

fn spawn_wal_checkpoint(
    db_path: PathBuf,
    label: &'static str,
    mode: WalCheckpointMode,
    busy_timeout_ms: u64,
) -> Result<tokio::sync::oneshot::Receiver<()>, String> {
    let (done_tx, done_rx) = tokio::sync::oneshot::channel();
    std::thread::Builder::new()
        .name(format!("flicknote-wal-{label}"))
        .spawn(move || {
            checkpoint_wal_standalone_with_timeout(&db_path, label, mode, busy_timeout_ms);
            if done_tx.send(()).is_err() {
                log::debug!("WAL checkpoint completion receiver was dropped");
            }
        })
        .map_err(|error| format!("could not start WAL checkpoint thread: {error}"))?;
    Ok(done_rx)
}

fn spawn_socket_server(
    listener: UnixListener,
    app: Arc<Application>,
    db: &PowerSyncDatabase,
    shutdown: watch::Receiver<bool>,
) -> JoinHandle<Result<(), ipc::DaemonError>> {
    let db = db.clone();
    let info_provider: ipc::ServerInfoProvider = Arc::new(move || {
        let status = db.status();
        let sync = if status.is_connected() {
            ipc::SyncConnectionState::Connected
        } else if status.is_connecting() {
            ipc::SyncConnectionState::Connecting
        } else {
            ipc::SyncConnectionState::Offline
        };
        let sync_errors = ipc::PowerSyncErrors {
            download: status.download_error().map(ToString::to_string),
            upload: status.upload_error().map(ToString::to_string),
        };
        ipc::ServerInfo::current().with_sync_status(sync, sync_errors)
    });
    tokio::spawn(async move {
        ipc::serve_app_until_with_provider(listener, app, info_provider, shutdown).await
    })
}

async fn wait_for_shutdown(
    actors: &mut ActorHandles,
    signals: &StartupSignals,
) -> Result<(), String> {
    wait_for_runtime_event(actors, async {
        signals.wait().await;
        Ok(())
    })
    .await
}

async fn wait_for_runtime_event<F>(actors: &mut ActorHandles, shutdown: F) -> Result<(), String>
where
    F: Future<Output = Result<(), String>>,
{
    tokio::pin!(shutdown);
    tokio::select! {
        result = &mut shutdown => result,
        result = &mut actors.checkpoint => {
            if let Err(error) = &result {
                log::error!("Checkpoint task panicked: {error}");
            } else {
                log::error!("Checkpoint task exited unexpectedly");
            }
            Err(format!("Checkpoint task exited: {result:?}"))
        }
        result = &mut actors.socket => {
            match result {
                Ok(Ok(())) => Err("IPC server exited unexpectedly".to_string()),
                Ok(Err(error)) => Err(format!("IPC server failed: {error}")),
                Err(error) => Err(format!("IPC server task panicked: {error}")),
            }
        }
        result = actors.powersync.join_next(), if !actors.powersync.is_empty() => {
            match result {
                Some(Ok(())) => Err("PowerSync actor exited unexpectedly".to_string()),
                Some(Err(error)) => Err(format!("PowerSync actor task panicked: {error}")),
                None => Err("PowerSync actor set became empty unexpectedly".to_string()),
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ShutdownStageOutcome {
    Completed,
    TimedOut,
    Failed(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ShutdownStageResult {
    stage: &'static str,
    outcome: ShutdownStageOutcome,
}

#[async_trait::async_trait]
trait ShutdownOperations: Sync {
    async fn disconnect(&self) -> Result<(), String>;
    async fn checkpoint(&self) -> Result<(), String>;
}

struct RuntimeShutdownOperations<'a> {
    db: &'a PowerSyncDatabase,
    db_path: PathBuf,
}

#[async_trait::async_trait]
impl ShutdownOperations for RuntimeShutdownOperations<'_> {
    async fn disconnect(&self) -> Result<(), String> {
        self.db.disconnect().await;
        Ok(())
    }

    async fn checkpoint(&self) -> Result<(), String> {
        let task = spawn_wal_checkpoint(
            self.db_path.clone(),
            "shutdown",
            WalCheckpointMode::Truncate,
            1_000,
        )?;
        task.await
            .map_err(|_| "WAL checkpoint worker ended before reporting completion".to_string())
    }
}

async fn run_shutdown_stage<F>(
    stage: &'static str,
    timeout: Duration,
    operation: F,
) -> ShutdownStageResult
where
    F: Future<Output = Result<(), String>>,
{
    let started = Instant::now();
    log::info!("Shutdown stage: {stage}");
    let outcome = match tokio::time::timeout(timeout, operation).await {
        Ok(Ok(())) => {
            log::info!(
                "Shutdown stage {stage} completed in {:?}",
                started.elapsed()
            );
            ShutdownStageOutcome::Completed
        }
        Ok(Err(error)) => {
            log::warn!(
                "Shutdown stage {stage} failed after {:?}: {error}",
                started.elapsed()
            );
            ShutdownStageOutcome::Failed(error)
        }
        Err(_) => {
            log::warn!("Shutdown stage {stage} exceeded {timeout:?}; continuing cleanup");
            ShutdownStageOutcome::TimedOut
        }
    };
    ShutdownStageResult { stage, outcome }
}

async fn run_storage_shutdown(
    operations: &dyn ShutdownOperations,
    disconnect_timeout: Duration,
    checkpoint_timeout: Duration,
) -> Vec<ShutdownStageResult> {
    vec![
        run_shutdown_stage(
            "disconnect PowerSync",
            disconnect_timeout,
            operations.disconnect(),
        )
        .await,
        run_shutdown_stage("truncate WAL", checkpoint_timeout, operations.checkpoint()).await,
    ]
}

async fn shutdown_startup(actors: &mut JoinSet<()>, db: &PowerSyncDatabase, db_path: PathBuf) {
    let operations = RuntimeShutdownOperations { db, db_path };
    let _stage_results = run_storage_shutdown(
        &operations,
        POWERSYNC_DISCONNECT_TIMEOUT,
        WAL_CHECKPOINT_TIMEOUT,
    )
    .await;
    actors.abort_all();
    log::info!("Daemon startup shutdown coordinator finished");
}

async fn shutdown_daemon(
    actors: &mut ActorHandles,
    db: &PowerSyncDatabase,
    db_path: PathBuf,
    shutdown: &watch::Sender<bool>,
) {
    if shutdown.send(true).is_err() {
        log::debug!("IPC shutdown coordinator had no active receiver");
    }
    let ipc_result = run_shutdown_stage("stop accepting and drain IPC", IPC_DRAIN_TIMEOUT, async {
        if actors.socket.is_finished() {
            return Ok(());
        }
        (&mut actors.socket)
            .await
            .map_err(|error| format!("IPC server task panicked: {error}"))?
            .map_err(|error| format!("IPC server failed during shutdown: {error}"))
    })
    .await;
    if ipc_result.outcome == ShutdownStageOutcome::TimedOut {
        actors.socket.abort();
    }

    let operations = RuntimeShutdownOperations { db, db_path };
    let _stage_results = run_storage_shutdown(
        &operations,
        POWERSYNC_DISCONNECT_TIMEOUT,
        WAL_CHECKPOINT_TIMEOUT,
    )
    .await;
    actors.powersync.abort_all();
    actors.checkpoint.abort();
    log::info!("Daemon shutdown coordinator finished");
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    struct FakeShutdownOperations {
        events: Mutex<Vec<&'static str>>,
        stall_disconnect: bool,
        stall_checkpoint: bool,
    }

    #[async_trait::async_trait]
    impl ShutdownOperations for FakeShutdownOperations {
        async fn disconnect(&self) -> Result<(), String> {
            self.events.lock().unwrap().push("disconnect");
            if self.stall_disconnect {
                std::future::pending().await
            } else {
                Ok(())
            }
        }

        async fn checkpoint(&self) -> Result<(), String> {
            self.events.lock().unwrap().push("checkpoint");
            if self.stall_checkpoint {
                std::future::pending().await
            } else {
                Ok(())
            }
        }
    }

    #[tokio::test(start_paused = true)]
    async fn stalled_disconnect_times_out_and_checkpoint_still_runs() {
        let operations = FakeShutdownOperations {
            events: Mutex::new(Vec::new()),
            stall_disconnect: true,
            stall_checkpoint: false,
        };
        let started = Instant::now();

        let stages = run_storage_shutdown(
            &operations,
            Duration::from_millis(10),
            Duration::from_millis(10),
        )
        .await;

        assert!(started.elapsed() < Duration::from_secs(1));
        assert_eq!(
            stages,
            vec![
                ShutdownStageResult {
                    stage: "disconnect PowerSync",
                    outcome: ShutdownStageOutcome::TimedOut,
                },
                ShutdownStageResult {
                    stage: "truncate WAL",
                    outcome: ShutdownStageOutcome::Completed,
                },
            ]
        );
        assert_eq!(
            operations.events.lock().unwrap().as_slice(),
            ["disconnect", "checkpoint"]
        );
    }

    #[tokio::test(start_paused = true)]
    async fn stalled_checkpoint_times_out_after_disconnect_completes() {
        let operations = FakeShutdownOperations {
            events: Mutex::new(Vec::new()),
            stall_disconnect: false,
            stall_checkpoint: true,
        };

        let stages = run_storage_shutdown(
            &operations,
            Duration::from_millis(10),
            Duration::from_millis(10),
        )
        .await;

        assert_eq!(stages[0].outcome, ShutdownStageOutcome::Completed);
        assert_eq!(stages[1].outcome, ShutdownStageOutcome::TimedOut);
        assert_eq!(
            operations.events.lock().unwrap().as_slice(),
            ["disconnect", "checkpoint"]
        );
    }

    #[tokio::test]
    async fn unexpected_powersync_actor_completion_is_a_runtime_failure() {
        let mut powersync = JoinSet::new();
        powersync.spawn(async {});
        let mut actors = ActorHandles {
            checkpoint: tokio::spawn(std::future::pending()),
            socket: tokio::spawn(std::future::pending()),
            powersync,
        };

        let error = wait_for_runtime_event(&mut actors, std::future::pending())
            .await
            .unwrap_err();

        assert_eq!(error, "PowerSync actor exited unexpectedly");
    }
}
