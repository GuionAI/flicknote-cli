//! Launch configuration for the synthetic-only experiment; no production config load.
use clap::Parser;
use flicknote_sync::spike::SpikeHost;
use std::{path::PathBuf, time::Duration};

#[derive(Debug, Parser)]
#[command(about = "Experimental FlickNote host — synthetic fixtures only")]
pub struct Options {
    /// Explicit absolute independent root, never the normal FlickNote directory.
    #[arg(long)]
    pub root: PathBuf,
    /// Independent loopback MCP port; 0 chooses a free port.
    #[arg(long)]
    pub mcp_port: u16,
    /// Seed only a new empty database; restart never reseeds existing notes.
    #[arg(long, default_value_t = 30)]
    pub seed: u32,
    /// Artificial creation latency for responsiveness/failure exercises.
    #[arg(long, default_value_t = 0)]
    pub delay_ms: u64,
    /// Controlled 100-row update batches at 20 ms intervals (maximum 100).
    #[arg(long, default_value_t = 0)]
    pub burst_batches: u32,
}

impl Options {
    pub fn parse() -> Self {
        <Self as Parser>::parse()
    }
    pub async fn start(&self) -> anyhow::Result<SpikeHost> {
        if self.delay_ms > 10_000 || self.burst_batches > 100 {
            anyhow::bail!("Fixture latency/burst exceeds bounded limits");
        }
        let host = SpikeHost::start(
            &self.root,
            self.mcp_port,
            self.seed,
            Duration::from_millis(self.delay_ms),
        )
        .await
        .map_err(anyhow::Error::msg)?;
        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(host.log_path())?;
        env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
            .target(env_logger::Target::Pipe(Box::new(file)))
            .try_init()?;
        log_endpoints(&host);
        Ok(host)
    }
}
#[allow(clippy::print_stderr)] // Explicit operator endpoints, containing no credentials.
fn log_endpoints(host: &SpikeHost) {
    // Both entrypoints are operator tools; never logs credentials or note bodies.
    eprintln!(
        "SYNTHETIC SPIKE ONLY\nIPC={}\nMCP=http://127.0.0.1:{}/mcp",
        host.socket.display(),
        host.mcp_port
    );
}
