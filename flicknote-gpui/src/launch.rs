//! Explicit launch modes; real mode never initializes synthetic storage.
use clap::Parser;
use flicknote_sync::{LocalHost, spike::SpikeHost};
use std::path::PathBuf;

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
        let options = flicknote_spike::Options {
            root: self.root.clone().ok_or("Explicit root required")?,
            mcp_port: self.mcp_port,
            seed: self.seed,
            delay_ms: self.delay_ms,
            burst_batches: self.burst_batches,
        };
        options
            .start()
            .await
            .map(Host::Synthetic)
            .map_err(|e| e.to_string())
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
