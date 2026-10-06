//! Normal shared host or explicit owned synthetic storage.
use clap::Parser;
use flicknote_sync::{LocalHost, spike::SpikeHost};
use std::{path::PathBuf, time::Duration};

#[derive(Parser)]
#[command(about = "FlickNote workspace and local application host")]
pub(crate) struct Options {
    /// Independent synthetic fixture root (no account or cloud connection).
    #[arg(long)]
    root: Option<PathBuf>,
    /// Synthetic loopback MCP port; 0 allocates. Normal mode uses FLICKNOTE_MCP_PORT.
    #[arg(long, requires = "root")]
    mcp_port: Option<u16>,
    #[arg(long, default_value_t = 30, requires = "root")]
    seed: u32,
    #[arg(long, default_value_t = 0, requires = "root")]
    delay_ms: u64,
    #[arg(long, default_value_t = 0, requires = "root")]
    pub(crate) burst_batches: u32,
}

pub(crate) enum Host {
    Real(LocalHost),
    Synthetic(SpikeHost),
}
impl Options {
    pub(crate) fn parse() -> Self {
        <Self as Parser>::parse()
    }
    pub(crate) async fn start(
        &self,
        cancel: tokio::sync::watch::Receiver<bool>,
        show_login: impl FnOnce(crate::login::LoginHandle) + Send,
    ) -> Result<Host, String> {
        if self.root.is_some() {
            return self.start_synthetic().await.map(Host::Synthetic);
        }
        let config = flicknote_core::config::Config::load().map_err(|e| e.to_string())?;
        self.start_normal(config, None, cancel, show_login).await
    }

    pub(crate) async fn start_normal(
        &self,
        config: flicknote_core::config::Config,
        port: Option<u16>,
        cancel: tokio::sync::watch::Receiver<bool>,
        show_login: impl FnOnce(crate::login::LoginHandle) + Send,
    ) -> Result<Host, String> {
        let ownership = flicknote_sync::ownership::DataDirectoryLock::acquire(
            &config.paths.data_dir,
        )
        .map_err(|e| {
            format!("Cannot start FlickNote: {e}. Quit the current host before starting this app.")
        })?;
        initialize_logging(&config.paths.log_file)?;
        let session = flicknote_auth::session::load_session(&config.paths.session_file).ok();
        if !session.is_some_and(|s| {
            !s.user.id.is_empty() && !s.access_token.is_empty() && !s.refresh_token.is_empty()
        }) {
            crate::login::authenticate(&config, cancel.clone(), show_login).await?;
        }
        let host = LocalHost::start_owned(config, port, cancel, ownership)
            .await
            .map_err(|e| e.to_string())?;
        endpoints("FlickNote", &host.socket, host.mcp_port);
        Ok(Host::Real(host))
    }

    async fn start_synthetic(&self) -> Result<SpikeHost, String> {
        if self.delay_ms > 10_000 || self.burst_batches > 100 {
            return Err("Fixture latency/burst exceeds bounded limits".into());
        }
        let host = SpikeHost::start(
            self.root.as_deref().ok_or("Explicit root required")?,
            self.mcp_port
                .ok_or("Synthetic mode requires --mcp-port (use 0 for an owned listener)")?,
            self.seed,
            Duration::from_millis(self.delay_ms),
        )
        .await?;
        initialize_logging(&host.log_path())?;
        endpoints("SYNTHETIC SPIKE ONLY", &host.socket, host.mcp_port);
        Ok(host)
    }
}
fn initialize_logging(path: &std::path::Path) -> Result<(), String> {
    let file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|e| e.to_string())?;
    // A test process may already have its logger; each real app initializes once.
    let _initialized =
        env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
            .target(env_logger::Target::Pipe(Box::new(file)))
            .try_init();
    Ok(())
}
#[allow(clippy::print_stderr)]
fn endpoints(label: &str, socket: &std::path::Path, port: u16) {
    log::info!(
        "{label}: IPC={} MCP=http://127.0.0.1:{port}/mcp",
        socket.display()
    );
    eprintln!(
        "{label}\nIPC={}\nMCP=http://127.0.0.1:{port}/mcp",
        socket.display()
    );
}
impl Host {
    pub(crate) fn services(&self, runtime: tokio::runtime::Handle) -> super::ui::Services {
        let (app, db, user_id, real_account) = match self {
            Self::Real(h) => (h.app.clone(), h.db.clone(), h.user_id.clone(), true),
            Self::Synthetic(h) => (
                h.app.clone(),
                h.db.clone(),
                flicknote_sync::spike::USER.into(),
                false,
            ),
        };
        let services = super::ui::Services {
            app,
            db,
            user_id,
            real_account,
            first_sync: std::sync::Mutex::default(),
            runtime,
            operations: std::sync::Mutex::new(vec![]),
            destination: std::sync::Mutex::default(),
            capture: std::sync::Arc::default(),
            draft: std::sync::Mutex::default(),
            organization: std::sync::Mutex::default(),
            capture_changed: tokio::sync::watch::channel(()).0,
        };
        if let Self::Real(host) = self {
            let control = crate::organization::start(
                &services,
                host.socket.parent().expect("host data directory"),
            );
            *services.organization.lock().expect("organization control") = Some(control);
        }
        services
    }
    pub(crate) async fn burst(&self, count: u32) -> Result<(), String> {
        if let Self::Synthetic(host) = self {
            host.burst(count).await?;
        }
        Ok(())
    }
    pub(crate) async fn run_until(
        self,
        stop: impl std::future::Future<Output = Result<(), String>>,
        cancel_operations: impl FnOnce(),
    ) -> Result<(), String> {
        match self {
            Self::Real(host) => host
                .run_until(stop, cancel_operations)
                .await
                .map_err(|e| e.to_string()),
            Self::Synthetic(host) => {
                let mut cancel = Some(cancel_operations);
                let result = host
                    .run_until(async {
                        let result = stop.await;
                        if let Some(cancel) = cancel.take() {
                            cancel();
                        }
                        result
                    })
                    .await;
                if let Some(cancel) = cancel.take() {
                    cancel();
                }
                result
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn synthetic_launch_rejects_unbounded_options_before_creating_state() {
        let parent = tempfile::tempdir().unwrap();
        let root = parent.path().join("fixture");
        for (delay_ms, burst_batches) in [(10_001, 0), (0, 101)] {
            let options = Options {
                root: Some(root.clone()),
                mcp_port: Some(0),
                seed: 5,
                delay_ms,
                burst_batches,
            };
            let (_, cancel) = tokio::sync::watch::channel(false);
            let error = options
                .start(cancel, |_| panic!("synthetic login"))
                .await
                .err()
                .unwrap();
            assert!(error.contains("bounded limits"));
            assert!(!root.exists());
        }
    }

    #[tokio::test]
    async fn synthetic_launch_seeds_delays_bursts_and_logs_in_owned_root() {
        use flicknote_client::{AppRequest, AppResponse, dto::NoteAddInput};
        let root = tempfile::tempdir().unwrap();
        let options = Options::try_parse_from([
            "trial",
            "--root",
            root.path().to_str().unwrap(),
            "--mcp-port",
            "0",
            "--seed",
            "5",
            "--delay-ms",
            "25",
            "--burst-batches",
            "1",
        ])
        .unwrap();
        let (_, cancel) = tokio::sync::watch::channel(false);
        let host = options
            .start(cancel, |_| panic!("synthetic login"))
            .await
            .unwrap();
        let Host::Synthetic(ref synthetic) = host else {
            panic!("synthetic host")
        };
        {
            let writer = synthetic.db.writer().await.unwrap();
            let count: i64 = writer
                .query_row("SELECT count(*) FROM notes", [], |row| row.get(0))
                .unwrap();
            assert_eq!(count, 5);
        }
        assert!(
            synthetic
                .socket
                .starts_with(root.path().canonicalize().unwrap())
        );
        assert_ne!(synthetic.mcp_port, 0);
        assert_ne!(synthetic.mcp_port, 37789);
        let started = std::time::Instant::now();
        let input: NoteAddInput =
            serde_json::from_value(serde_json::json!({"content":"[fixture-fail]"})).unwrap();
        assert!(
            synthetic
                .app
                .handle(AppRequest::NoteAdd(input))
                .await
                .is_err()
        );
        assert!(started.elapsed() >= Duration::from_millis(25));
        host.burst(options.burst_batches).await.unwrap();
        let AppResponse::NoteDetail(note) = synthetic
            .app
            .handle(AppRequest::NoteGet {
                id: "1".into(),
                archived: false,
            })
            .await
            .unwrap()
        else {
            panic!("fixture detail")
        };
        assert!(note.content.starts_with("Synthetic update batch 0"));
        assert!(synthetic.log_path().is_file());
        let socket = synthetic.socket.clone();
        host.run_until(async { Ok(()) }, || {}).await.unwrap();
        assert!(!socket.exists());
    }

    #[test]
    fn normal_defaults_and_explicit_synthetic_options() {
        assert!(Options::try_parse_from(["flicknote-gpui"]).is_ok());
        assert!(
            Options::try_parse_from(["flicknote-gpui", "--root", "/tmp/owned", "--mcp-port", "0"])
                .is_ok()
        );
        for args in [
            vec!["flicknote-gpui", "--mcp-port", "0"],
            vec!["flicknote-gpui", "--seed", "5"],
        ] {
            assert!(Options::try_parse_from(args).is_err());
        }
    }
}

#[cfg(test)]
#[path = "normal_host_tests.rs"]
mod normal_host_tests;
