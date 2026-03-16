use anyhow::{Context, Result};
use clap::Args;
use taskchampion::{Operations, PowerSyncStorage, Replica};

use crate::ids::{resolve_id, short_id};

#[derive(Args)]
pub struct StopArgs {
    /// Task ID (8-char hex or full UUID)
    pub id: String,
}

pub async fn run(replica: &mut Replica<PowerSyncStorage>, args: StopArgs) -> Result<()> {
    let uuid = resolve_id(replica, &args.id).await?;
    let mut all_tasks = replica.all_tasks().await.context("Failed to load tasks")?;

    let task = all_tasks
        .get_mut(&uuid)
        .with_context(|| format!("Task {} not found", args.id))?;

    let mut ops = Operations::new();
    task.stop(&mut ops)?;

    replica
        .commit_operations(ops)
        .await
        .context("Failed to commit")?;

    println!("Stopped: {}", short_id(&uuid));
    Ok(())
}
