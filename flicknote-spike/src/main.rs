use flicknote_spike::Options;
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let options = Options::parse();
    let host = options.start().await?;
    host.burst(options.burst_batches)
        .await
        .map_err(anyhow::Error::msg)?;
    host.run_until(async {
        let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()).map_err(|e| e.to_string())?;
        tokio::select! { result = tokio::signal::ctrl_c() => result.map_err(|e| e.to_string()), _ = terminate.recv() => Ok(()) }
    }).await.map_err(anyhow::Error::msg)
}
