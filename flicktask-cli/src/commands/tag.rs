use anyhow::{Context, Result};
use clap::Args;
use taskchampion::{Operations, PowerSyncStorage, Replica, Tag};

use crate::ids::{resolve_id, short_id};

#[derive(Args)]
pub struct TagArgs {
    /// Task ID (8-char hex or full UUID)
    pub id: String,

    /// Tag to add
    pub tag: String,
}

pub async fn run(replica: &mut Replica<PowerSyncStorage>, args: TagArgs) -> Result<()> {
    let uuid = resolve_id(replica, &args.id).await?;
    let mut all_tasks = replica.all_tasks().await.context("Failed to load tasks")?;

    let task = all_tasks
        .get_mut(&uuid)
        .with_context(|| format!("Task {} not found", args.id))?;

    let tag: Tag = args
        .tag
        .parse()
        .with_context(|| format!("Invalid tag: {:?}", args.tag))?;

    let mut ops = Operations::new();
    super::with_on_modify(&uuid, task, &mut ops, |task, ops| {
        task.add_tag(&tag, ops)?;
        Ok(())
    })?;

    replica
        .commit_operations(ops)
        .await
        .context("Failed to commit")?;

    println!("Tagged {} with {:?}", short_id(&uuid), args.tag);
    Ok(())
}
