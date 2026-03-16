#![allow(clippy::print_stdout, clippy::print_stderr, unreachable_pub)]

use anyhow::Result;

#[cfg(feature = "powersync")]
mod commands;
#[cfg(feature = "powersync")]
mod config;
#[cfg(feature = "powersync")]
mod display;
#[cfg(feature = "powersync")]
mod ids;
#[cfg(feature = "powersync")]
mod task_tree;

#[tokio::main(flavor = "current_thread")]
async fn main() {
    if let Err(e) = run().await {
        eprintln!("Error: {e:#}");
        std::process::exit(1);
    }
}

async fn run() -> Result<()> {
    #[cfg(not(feature = "powersync"))]
    {
        anyhow::bail!("PgStorage not yet implemented — this binary requires the powersync feature");
    }

    #[cfg(feature = "powersync")]
    {
        use clap::Parser;

        let cli = commands::Cli::parse();
        let flicktask_config = config::FlicktaskConfig::load();

        let core_config = flicknote_core::config::Config::load()
            .map_err(|e| anyhow::anyhow!("Failed to load core config: {e}"))?;
        let db_path = core_config.paths.db_file.clone();

        let user_id_str = flicknote_core::session::get_user_id(&core_config)
            .map_err(|_| anyhow::anyhow!("Not authenticated. Run `flicknote login` first."))?;
        let user_id = taskchampion::Uuid::parse_str(&user_id_str)
            .map_err(|e| anyhow::anyhow!("Invalid user_id in session: {e}"))?;

        let storage = taskchampion::PowerSyncStorage::new(&db_path, user_id).await?;
        let mut replica = taskchampion::Replica::new(storage);

        commands::dispatch(&mut replica, &flicktask_config, cli).await
    }
}
