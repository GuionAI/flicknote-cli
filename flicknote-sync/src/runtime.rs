use std::path::{Path, PathBuf};
use std::sync::Arc;

use flicknote_auth::client::GoTrueClient;
use flicknote_core::{
    backend::{LocalPowerSyncBackend, NoteDb},
    config::Config,
    schema::app_schema,
    services::ports::{NoteCreator, ShareGateway},
};
use powersync::{ConnectionPool, PowerSyncDatabase, SyncOptions, env::PowerSyncEnvironment};
use tokio::net::UnixListener;

use crate::app::Application;
use crate::ipc;
use crate::remote::{RemoteNoteCreator, RemoteShareGateway};
use crate::storage_maintenance::{WalCheckpointMode, checkpoint_wal_standalone};
use crate::upload::FlickNoteConnector;

fn pid_path(config: &Config) -> PathBuf {
    PathBuf::from(&config.paths.data_dir).join("sync.pid")
}

struct PidGuard(PathBuf);

impl Drop for PidGuard {
    fn drop(&mut self) {
        if let Err(error) = std::fs::remove_file(&self.0) {
            log::warn!("Failed to remove PID file: {error}");
        }
    }
}

struct SocketGuard(PathBuf);

impl Drop for SocketGuard {
    fn drop(&mut self) {
        if let Err(error) = std::fs::remove_file(&self.0) {
            log::warn!("Failed to remove socket file: {error}");
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

/// Check for an existing sync daemon and write our PID file.
///
/// Note: there is a small TOCTOU window between the liveness check and writing
/// the new PID file. Two daemons launched simultaneously could both pass.
#[allow(unsafe_code)]
fn check_and_write_pid(path: &Path) -> Result<PidGuard, Box<dyn std::error::Error>> {
    if let Ok(contents) = std::fs::read_to_string(path)
        && let Ok(pid) = contents.trim().parse::<i32>()
    {
        let result = unsafe { libc::kill(pid, 0) };
        if result == 0
            || (result == -1 && std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM))
        {
            return Err(format!(
                "Sync daemon already running (pid={pid}). Kill it first or delete {}",
                path.display()
            )
            .into());
        }
        log::info!("Removing stale PID file (pid={pid} no longer running)");
    }

    std::fs::write(path, std::process::id().to_string())
        .map_err(|error| format!("Failed to write PID file {}: {error}", path.display()))?;
    Ok(PidGuard(path.to_path_buf()))
}

struct ActorHandles {
    checkpoint: tokio::task::JoinHandle<()>,
    socket: tokio::task::JoinHandle<()>,
}

pub async fn run() -> Result<(), Box<dyn std::error::Error>> {
    let config = Arc::new(Config::load()?);
    let _pid_guard = check_and_write_pid(&pid_path(&config))?;
    let (socket_listener, _socket_guard) = bind_socket(&config)?;
    config.validate()?;

    let db = open_powersync_database(&config)?;
    let auth = Arc::new(GoTrueClient::new(
        &config.supabase_url,
        &config.supabase_anon_key,
        &config.paths.session_file,
    ));
    let connector = build_connector(&db, &auth, &config);

    startup_checkpoint(config.paths.db_file.clone()).await;
    let backend = open_local_backend(&db, &config)?;

    log::info!("Sync daemon connecting (pid {})", std::process::id());
    db.connect(SyncOptions::new(connector)).await;
    log::info!("Sync daemon connected (pid {})", std::process::id());

    let mut actors = ActorHandles {
        checkpoint: spawn_checkpoint_worker(config.paths.db_file.clone()),
        socket: spawn_socket_server(socket_listener, backend, &db, &auth, &config),
    };
    let result = wait_for_shutdown(&mut actors).await;
    shutdown_daemon(&mut actors, &db, config.paths.db_file.clone()).await;
    result.map_err(Into::into)
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
    db.async_tasks().spawn_with_tokio();
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

async fn startup_checkpoint(db_path: PathBuf) {
    log::info!("Running startup WAL checkpoint");
    if let Err(error) = tokio::task::spawn_blocking(move || {
        checkpoint_wal_standalone(&db_path, "startup", WalCheckpointMode::Truncate)
    })
    .await
    {
        log::error!("Startup WAL checkpoint task panicked: {error}");
    }
}

fn open_local_backend(
    db: &PowerSyncDatabase,
    config: &Config,
) -> Result<Arc<dyn NoteDb>, Box<dyn std::error::Error>> {
    let user_id = flicknote_core::session::get_user_id(config)?;
    Ok(Arc::new(LocalPowerSyncBackend::new(db.clone(), user_id)))
}

fn spawn_checkpoint_worker(db_path: PathBuf) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(30));
        interval.tick().await;
        loop {
            interval.tick().await;
            let path = db_path.clone();
            if let Err(error) = tokio::task::spawn_blocking(move || {
                checkpoint_wal_standalone(&path, "periodic", WalCheckpointMode::Passive)
            })
            .await
            {
                log::error!("Periodic WAL checkpoint task panicked: {error}");
            }
        }
    })
}

fn spawn_socket_server(
    listener: UnixListener,
    backend: Arc<dyn NoteDb>,
    db: &PowerSyncDatabase,
    auth: &Arc<GoTrueClient>,
    config: &Arc<Config>,
) -> tokio::task::JoinHandle<()> {
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
    let app =
        Arc::new(Application::new(backend, creator, gateway).with_web_url(config.web_url.clone()));
    tokio::spawn(async move {
        if let Err(error) = ipc::serve_app(listener, app, ipc::ServerInfo::current()).await {
            log::error!("Application socket server failed: {error}");
        }
    })
}

async fn wait_for_shutdown(actors: &mut ActorHandles) -> Result<(), String> {
    tokio::select! {
        _ = tokio::signal::ctrl_c() => Ok(()),
        result = &mut actors.checkpoint => {
            if let Err(error) = &result {
                log::error!("Checkpoint task panicked: {error}");
            } else {
                log::error!("Checkpoint task exited unexpectedly");
            }
            Err(format!("Checkpoint task exited: {result:?}"))
        }
        result = &mut actors.socket => {
            if let Err(error) = &result {
                log::error!("Socket task panicked: {error}");
            } else {
                log::error!("Socket task exited unexpectedly");
            }
            Err(format!("Socket task exited: {result:?}"))
        }
    }
}

async fn shutdown_daemon(actors: &mut ActorHandles, db: &PowerSyncDatabase, db_path: PathBuf) {
    actors.checkpoint.abort();
    actors.socket.abort();
    db.disconnect().await;
    if let Err(error) = tokio::task::spawn_blocking(move || {
        checkpoint_wal_standalone(&db_path, "shutdown", WalCheckpointMode::Truncate)
    })
    .await
    {
        log::error!("Shutdown WAL checkpoint task panicked: {error}");
    }
    log::info!("Sync daemon stopped");
}
