//! Independent-directory, synthetic-only feasibility host. No auth or cloud connection.
use crate::{
    app::Application,
    fts_search::FtsSearchService,
    ipc, mcp,
    ownership::{DataDirectoryLock, OwnershipError},
    runtime,
};
use flicknote_core::backend::{LocalPowerSyncBackend, NoteDb};
use powersync::PowerSyncDatabase;
use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use tokio::{net::TcpListener, sync::watch};

mod fixture;
pub use fixture::{PROJECTS, USER};
mod paths;
pub mod today;

pub struct SpikeHost {
    pub app: Arc<Application>,
    pub db: PowerSyncDatabase,
    pub socket: PathBuf,
    pub mcp_port: u16,
    actors: runtime::ActorHandles,
    shutdown: watch::Sender<bool>,
    db_path: PathBuf,
    _socket_guard: runtime::SocketGuard,
    _ownership: DataDirectoryLock,
}

impl SpikeHost {
    pub async fn start(root: &Path, port: u16, seed: u32, delay: Duration) -> Result<Self, String> {
        if port == 37789 {
            return Err("Spike MCP port must differ from production default 37789".into());
        }
        if seed > 10_000 {
            return Err("Seed is bounded at 10,000 synthetic notes".into());
        }
        let config = paths::config(root)?;
        let ownership =
            DataDirectoryLock::acquire(&config.paths.data_dir).map_err(|e| match e {
            OwnershipError::AlreadyOwned { .. } => "Spike root already owns an active host; Quit that experimental host before restarting".to_string(),
            OwnershipError::Io(e) => e.to_string(),
        })?;
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port))
            .await
            .map_err(|e| e.to_string())?;
        let mcp_port = listener.local_addr().map_err(|e| e.to_string())?.port();
        let db = runtime::open_powersync_database(&config).map_err(|e| e.to_string())?;
        let powersync = runtime::spawn_powersync_actors(&db);
        fixture::seed(&db, seed).await?;
        let search = FtsSearchService::new(db.clone());
        search.prepare().await?;
        let backend: Arc<dyn NoteDb> =
            Arc::new(LocalPowerSyncBackend::new(db.clone(), fixture::USER.into()));
        let creator = Arc::new(fixture::FixtureCreator {
            db: db.clone(),
            delay,
        });
        let app = Arc::new(Application::new_private(backend, creator).with_search(search));
        let (socket_listener, socket_guard) =
            runtime::bind_socket(&config).map_err(|e| e.to_string())?;
        let (shutdown, receiver) = watch::channel(false);
        let socket =
            runtime::spawn_socket_server(socket_listener, app.clone(), &db, receiver.clone());
        let mcp = tokio::spawn(mcp::http::serve(listener, app.clone(), receiver));
        let actors = runtime::ActorHandles {
            checkpoint: runtime::spawn_checkpoint_worker(config.paths.db_file.clone()),
            socket,
            mcp,
            powersync,
        };
        Ok(Self {
            app,
            db,
            socket: ipc::socket_path(&config),
            mcp_port,
            actors,
            shutdown,
            db_path: config.paths.db_file,
            _socket_guard: socket_guard,
            _ownership: ownership,
        })
    }

    pub fn log_path(&self) -> PathBuf {
        self.db_path.with_file_name("flicknote.log")
    }

    pub async fn run_until<F: std::future::Future<Output = Result<(), String>>>(
        mut self,
        stop: F,
    ) -> Result<(), String> {
        let result = runtime::wait_for_runtime_event(&mut self.actors, stop).await;
        self.shutdown().await;
        result
    }

    pub async fn shutdown(mut self) {
        runtime::shutdown_daemon(
            &mut self.actors,
            &self.db,
            self.db_path.clone(),
            &self.shutdown,
        )
        .await;
    }

    /// Bounded deterministic storage updates, not a cloud simulator.
    pub async fn burst(&self, batches: u32) -> Result<(), String> {
        for batch in 0..batches.min(100) {
            {
                let mut writer = self.db.writer().await.map_err(|e| e.to_string())?;
                let tx = writer.transaction().map_err(|e| e.to_string())?;
                tx.execute(
                    "UPDATE notes SET content = ? WHERE user_id = ? AND short_id BETWEEN 1 AND 100",
                    rusqlite::params![
                        format!("Synthetic update batch {batch}\n合成更新"),
                        fixture::USER
                    ],
                )
                .map_err(|e| e.to_string())?;
                tx.commit().map_err(|e| e.to_string())?;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        Ok(())
    }
}

pub type Database = PowerSyncDatabase;

impl Drop for SpikeHost {
    fn drop(&mut self) {
        self.shutdown.send_replace(true);
        self.actors.socket.abort();
        self.actors.mcp.abort();
        self.actors.checkpoint.abort();
        self.actors.powersync.abort_all();
    }
}
