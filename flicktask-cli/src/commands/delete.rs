use anyhow::{Context, Result};
use clap::Args;
use taskchampion::{Operations, PowerSyncStorage, Replica, Status, Uuid};

use crate::{
    ids::{resolve_id, short_id},
    task_tree::TaskTree,
};

#[derive(Args)]
pub struct DeleteArgs {
    /// Task ID (8-char hex or full UUID)
    pub id: String,

    /// Confirm deletion (required)
    #[arg(long)]
    pub yes: bool,

    /// Also delete all subtasks
    #[arg(long)]
    pub recursive: bool,
}

pub async fn run(replica: &mut Replica<PowerSyncStorage>, args: DeleteArgs) -> Result<()> {
    if !args.yes {
        anyhow::bail!("Deletion requires --yes flag to confirm");
    }

    let uuid = resolve_id(replica, &args.id).await?;
    let mut all_tasks = replica.all_tasks().await.context("Failed to load tasks")?;
    let tree = TaskTree::from_tasks(&all_tasks);

    let to_delete: Vec<Uuid> = if args.recursive {
        let mut descendants = tree.descendants(uuid);
        descendants.reverse(); // leaves first
        descendants.push(uuid);
        descendants
    } else {
        let pending = tree.pending_child_ids(uuid, &all_tasks);
        if !pending.is_empty() {
            anyhow::bail!(
                "Task has pending subtasks: {}. Use --recursive to cascade, or delete them first.",
                pending.join(", ")
            );
        }
        vec![uuid]
    };

    let mut ops = Operations::new();
    for task_uuid in &to_delete {
        if let Some(task) = all_tasks.get_mut(task_uuid) {
            task.set_status(Status::Deleted, &mut ops)?;
        }
    }

    replica
        .commit_operations(ops)
        .await
        .context("Failed to commit")?;

    println!("Deleted: {}", short_id(&uuid));
    Ok(())
}
