use crate::*;

pub(crate) fn pid_path(config: &Config) -> PathBuf {
    PathBuf::from(&config.paths.data_dir).join("sync.pid")
}

pub(crate) struct PidGuard(PathBuf);

impl Drop for PidGuard {
    fn drop(&mut self) {
        if let Err(e) = std::fs::remove_file(&self.0) {
            log::warn!("Failed to remove PID file: {}", e);
        }
    }
}

pub(crate) struct SocketGuard(PathBuf);

impl Drop for SocketGuard {
    fn drop(&mut self) {
        if let Err(e) = std::fs::remove_file(&self.0) {
            log::warn!("Failed to remove socket file: {}", e);
        }
    }
}

pub(crate) fn bind_socket(
    config: &Config,
) -> Result<(UnixListener, SocketGuard), Box<dyn std::error::Error>> {
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
/// Note: there is a small TOCTOU window between the `kill(pid, 0)` liveness
/// check and writing the new PID file. Two daemons launched simultaneously
/// could both pass. For a CLI daemon this is acceptable; use `flock` or
/// `O_CREAT|O_EXCL` if stronger guarantees are ever needed.
#[allow(unsafe_code)]
pub(crate) fn check_and_write_pid(path: &Path) -> Result<PidGuard, Box<dyn std::error::Error>> {
    if let Ok(contents) = std::fs::read_to_string(path)
        && let Ok(pid) = contents.trim().parse::<i32>()
    {
        let result = unsafe { libc::kill(pid, 0) };
        if result == 0
            || (result == -1 && std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM))
        {
            return Err(format!(
                "Sync daemon already running (pid={}). Kill it first or delete {}",
                pid,
                path.display()
            )
            .into());
        }
        log::info!("Removing stale PID file (pid={} no longer running)", pid);
    }

    std::fs::write(path, std::process::id().to_string())
        .map_err(|e| format!("Failed to write PID file {}: {}", path.display(), e))?;
    Ok(PidGuard(path.to_path_buf()))
}

/// Tear down all async actors, disconnect the database, and run a final TRUNCATE
/// checkpoint.
///
/// Called from every shutdown path (ctrl-c, task panic, normal exit). The pool
/// is fully gone after `db.disconnect().await`, so TRUNCATE succeeds without
/// contention. Uses `spawn_blocking` to keep the blocking rusqlite I/O off the
/// async executor thread per [`checkpoint_wal_standalone`]'s contract.
pub(crate) async fn shutdown_daemon(
    upload_handle: &mut tokio::task::JoinHandle<()>,
    checkpoint_handle: &mut tokio::task::JoinHandle<()>,
    socket_handle: &mut tokio::task::JoinHandle<()>,
    db: &PowerSyncDatabase,
    db_path: PathBuf,
) {
    upload_handle.abort();
    checkpoint_handle.abort();
    socket_handle.abort();
    db.disconnect().await;
    if let Err(e) = tokio::task::spawn_blocking(move || {
        checkpoint_wal_standalone(&db_path, "shutdown", WalCheckpointMode::Truncate)
    })
    .await
    {
        log::error!("Shutdown WAL checkpoint task panicked: {e}");
    }
    log::info!("Sync daemon stopped");
}

pub async fn run() -> Result<(), Box<dyn std::error::Error>> {
    let config = Arc::new(Config::load()?);

    let pid_file = pid_path(&config);
    let _pid_guard = check_and_write_pid(&pid_file)?;
    let (socket_listener, _socket_guard) = bind_socket(&config)?;

    config.validate()?;

    PowerSyncEnvironment::powersync_auto_extension()?;

    let pool = ConnectionPool::open(&config.paths.db_file)?;
    let env = PowerSyncEnvironment::custom(
        reqwest::Client::new(),
        pool,
        PowerSyncEnvironment::tokio_timer(),
    );

    let db = PowerSyncDatabase::new(env, app_schema());
    db.async_tasks().spawn_with_tokio();

    let auth = Arc::new(GoTrueClient::new(
        &config.supabase_url,
        &config.supabase_anon_key,
        &config.paths.session_file,
    ));

    let upload_guard = Arc::new(tokio::sync::Mutex::new(()));
    let http_client = reqwest::Client::new();
    let upload_client = http_client.clone();

    let connector = FlickNoteConnector {
        db: db.clone(),
        auth: Arc::clone(&auth),
        upload_guard: Arc::clone(&upload_guard),
        http_client,
        powersync_url: config.powersync_url.clone(),
        supabase_url: config.supabase_url.clone(),
        supabase_anon_key: config.supabase_anon_key.clone(),
    };

    // Reclaim leftover WAL from previous sessions BEFORE connecting sync actors.
    // TRUNCATE is safe here because no pool connections exist yet — db.connect()
    // hasn't started the download actor. A bloated WAL inherited from a crashed
    // session is reset to zero so incremental PASSIVE checkpoints start from a
    // clean baseline.
    // spawn_blocking keeps blocking rusqlite I/O off the async executor thread.
    log::info!("Running startup WAL checkpoint");
    let startup_db_path = config.paths.db_file.clone();
    if let Err(e) = tokio::task::spawn_blocking(move || {
        checkpoint_wal_standalone(&startup_db_path, "startup", WalCheckpointMode::Truncate)
    })
    .await
    {
        log::error!("Startup WAL checkpoint task panicked: {e}");
    }

    // Finish schema replacement through the application pool before PowerSync
    // starts its download/upload actors. Replacing tracking views after connect
    // races the actor-held SQLite connections and can fail with SQLITE_BUSY on
    // an existing database.
    let user_id = flicknote_core::session::get_user_id(&config)?;
    let backend: Arc<dyn NoteDb> = Arc::new(SqliteBackend {
        db: Database::open_local(&config).await?,
        user_id,
    });

    log::info!("Sync daemon connecting (pid {})", std::process::id());
    db.connect(SyncOptions::new(connector)).await;
    log::info!("Sync daemon connected (pid {})", std::process::id());

    // Application writes happen in this process. Each may-write request sends a
    // best-effort trigger; the startup drain recovers committed writes whose signal
    // was lost because of a crash or a full channel.
    let (trigger_tx, mut trigger_rx) = mpsc::channel::<()>(16);

    let upload_db = db.clone();
    let upload_supabase_url = config.supabase_url.clone();
    let upload_anon_key = config.supabase_anon_key.clone();
    let upload_guard_clone = Arc::clone(&upload_guard);
    let upload_auth_clone = Arc::clone(&auth);
    let upload_db_path = config.paths.db_file.clone();

    let mut upload_handle = tokio::spawn(async move {
        // Initial upload on startup recovers committed CRUD left by a crash,
        // a lost in-process signal, or a pre-upgrade CLI writer.
        retry_upload_until_success(
            &upload_db,
            &upload_client,
            &upload_auth_clone,
            &upload_guard_clone,
            &upload_supabase_url,
            &upload_anon_key,
            "Startup upload",
            &upload_db_path,
        )
        .await;

        loop {
            // Block until the application host reports a may-write request.
            if trigger_rx.recv().await.is_none() {
                break;
            }

            // Trailing debounce: collapse burst writes (e.g. bulk import) into a
            // single upload attempt. Fire only after 200ms of silence.
            loop {
                tokio::select! {
                    _ = tokio::time::sleep(std::time::Duration::from_millis(200)) => break,
                    v = trigger_rx.recv() => {
                        if v.is_none() { return; } // channel closed
                        // more events arrived — reset the silence window
                    }
                }
            }

            retry_upload_until_success(
                &upload_db,
                &upload_client,
                &upload_auth_clone,
                &upload_guard_clone,
                &upload_supabase_url,
                &upload_anon_key,
                "Upload",
                &upload_db_path,
            )
            .await;
        }
    });

    // Periodic PASSIVE checkpoint every 30s — independent of upload success or
    // download actor state. Makes incremental progress draining the WAL without
    // acquiring PENDING/EXCLUSIVE locks, so it never contends with pool writers.
    let checkpoint_db_path = config.paths.db_file.clone();
    let mut checkpoint_handle = tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(30));
        interval.tick().await; // skip the immediate first tick
        loop {
            interval.tick().await;
            let path = checkpoint_db_path.clone();
            if let Err(e) = tokio::task::spawn_blocking(move || {
                checkpoint_wal_standalone(&path, "periodic", WalCheckpointMode::Passive)
            })
            .await
            {
                log::error!("Periodic WAL checkpoint task panicked: {e}");
            }
        }
    });

    let socket_config = Arc::clone(&config);
    let socket_http = reqwest::Client::new();
    let socket_share_lock = Arc::new(ShareRequestLock::default());
    let creator: Arc<dyn NoteCreator> = Arc::new(RemoteNoteCreator {
        db: db.clone(),
        auth: Arc::clone(&auth),
        http: socket_http.clone(),
        config: Arc::clone(&config),
    });
    let gateway: Arc<dyn ShareGateway> = Arc::new(RemoteShareGateway {
        http: socket_http.clone(),
        auth: Arc::clone(&auth),
        config: Arc::clone(&config),
        lock: socket_share_lock,
    });
    let app = Arc::new(
        Application::new(backend, creator, gateway)
            .with_web_url(config.web_url.clone())
            .with_write_signal(trigger_tx),
    );
    let mut socket_handle = tokio::spawn(async move {
        if let Err(error) = ipc::serve_app(socket_listener, app, ipc::ServerInfo::current()).await {
            log::error!("Application socket server failed: {error}");
        }
    });

    tokio::select! {
        _ = tokio::signal::ctrl_c() => {}
        res = &mut upload_handle => {
            if let Err(e) = res {
                log::error!("Upload task panicked: {e}");
                shutdown_daemon(&mut upload_handle, &mut checkpoint_handle, &mut socket_handle, &db, socket_config.paths.db_file.clone()).await;
                return Err(e.into());
            }
        }
        res = &mut checkpoint_handle => {
            match res {
                Ok(_) => log::error!("Checkpoint task exited unexpectedly"),
                Err(ref e) => log::error!("Checkpoint task panicked: {e}"),
            }
            let err_msg = format!("Checkpoint task exited: {res:?}");
            shutdown_daemon(&mut upload_handle, &mut checkpoint_handle, &mut socket_handle, &db, socket_config.paths.db_file.clone()).await;
            return Err(err_msg.into());
        }
        res = &mut socket_handle => {
            match res {
                Ok(_) => log::error!("Socket task exited unexpectedly"),
                Err(ref e) => log::error!("Socket task panicked: {e}"),
            }
            let err_msg = format!("Socket task exited: {res:?}");
            shutdown_daemon(&mut upload_handle, &mut checkpoint_handle, &mut socket_handle, &db, socket_config.paths.db_file.clone()).await;
            return Err(err_msg.into());
        }
    }
    shutdown_daemon(
        &mut upload_handle,
        &mut checkpoint_handle,
        &mut socket_handle,
        &db,
        socket_config.paths.db_file.clone(),
    )
    .await;

    Ok(())
}
