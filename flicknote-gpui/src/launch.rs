//! Explicit launch modes; real mode never initializes synthetic storage.
use clap::Parser;
use flicknote_sync::{LocalHost, spike::SpikeHost};
use std::{path::PathBuf, time::Duration};

#[derive(Parser)]
#[command(about = "Experimental FlickNote Today — explicit independent profile")]
pub(crate) struct Options {
    /// Independent real-account profile; sign in with email in the GUI or use login --auth-only.
    #[arg(long, required_unless_present = "root", conflicts_with = "root")]
    profile: Option<PathBuf>,
    /// Independent synthetic fixture root (no account or cloud connection).
    #[arg(long, required_unless_present = "profile")]
    root: Option<PathBuf>,
    /// Local loopback MCP port; 0 allocates an available port. Default daemon port is rejected.
    #[arg(long)]
    mcp_port: u16,
    #[arg(long, default_value_t = 30, conflicts_with = "profile")]
    seed: u32,
    #[arg(long, default_value_t = 0, conflicts_with = "profile")]
    delay_ms: u64,
    #[arg(long, default_value_t = 0, conflicts_with = "profile")]
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
        if self.mcp_port == 37789 {
            return Err("Choose a trial MCP port other than 37789".into());
        }
        if let Some(root) = &self.profile {
            let config = flicknote_core::profile::load(root)?;
            let ownership =
                flicknote_sync::ownership::DataDirectoryLock::acquire(&config.paths.data_dir)
                    .map_err(|e| e.to_string())?;
            let session = flicknote_auth::session::load_session(&config.paths.session_file).ok();
            if !session.is_some_and(|s| {
                !s.user.id.is_empty() && !s.access_token.is_empty() && !s.refresh_token.is_empty()
            }) {
                crate::login::authenticate(&config, cancel.clone(), show_login).await?;
            }
            let host = LocalHost::start_owned(config, Some(self.mcp_port), cancel, ownership)
                .await
                .map_err(|e| e.to_string())?;
            endpoints("REAL ACCOUNT TRIAL", &host.socket, host.mcp_port);
            return Ok(Host::Real(host));
        }
        self.start_synthetic().await.map(Host::Synthetic)
    }

    async fn start_synthetic(&self) -> Result<SpikeHost, String> {
        if self.delay_ms > 10_000 || self.burst_batches > 100 {
            return Err("Fixture latency/burst exceeds bounded limits".into());
        }
        let host = SpikeHost::start(
            self.root.as_deref().ok_or("Explicit root required")?,
            self.mcp_port,
            self.seed,
            Duration::from_millis(self.delay_ms),
        )
        .await?;
        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(host.log_path())
            .map_err(|e| e.to_string())?;
        env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
            .target(env_logger::Target::Pipe(Box::new(file)))
            .try_init()
            .map_err(|e| e.to_string())?;
        endpoints("SYNTHETIC SPIKE ONLY", &host.socket, host.mcp_port);
        Ok(host)
    }
}
#[allow(clippy::print_stderr)]
fn endpoints(label: &str, socket: &std::path::Path, port: u16) {
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
        super::ui::Services {
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
            capture_changed: tokio::sync::watch::channel(()).0,
        }
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
                profile: None,
                root: Some(root.clone()),
                mcp_port: 0,
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
    fn real_and_synthetic_modes_require_explicit_separate_roots_and_ports() {
        assert!(
            Options::try_parse_from(["trial", "--profile", "/tmp/owned", "--mcp-port", "0"])
                .is_ok()
        );
        assert!(
            Options::try_parse_from([
                "trial",
                "--root",
                "/tmp/owned",
                "--mcp-port",
                "0",
                "--seed",
                "5"
            ])
            .is_ok()
        );
        for arguments in [
            vec!["trial", "--mcp-port", "0"],
            vec!["trial", "--profile", "/tmp/owned"],
            vec![
                "trial",
                "--profile",
                "/tmp/owned",
                "--root",
                "/tmp/other",
                "--mcp-port",
                "0",
            ],
            vec![
                "trial",
                "--profile",
                "/tmp/owned",
                "--mcp-port",
                "0",
                "--seed",
                "5",
            ],
        ] {
            assert!(Options::try_parse_from(arguments).is_err());
        }
    }
}
