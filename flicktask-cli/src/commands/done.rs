use anyhow::{Context, Result};
use clap::Args;
use taskchampion::{Operations, PowerSyncStorage, Replica, Status, Uuid};

use crate::{
    ids::{resolve_id, short_id},
    task_tree::TaskTree,
};

#[derive(Args)]
pub struct DoneArgs {
    /// Task ID (8-char hex or full UUID)
    pub id: String,

    /// Also mark all subtasks as done
    #[arg(long)]
    pub recursive: bool,
}

pub async fn run(replica: &mut Replica<PowerSyncStorage>, args: DoneArgs) -> Result<()> {
    let uuid = resolve_id(replica, &args.id).await?;
    let mut all_tasks = replica.all_tasks().await.context("Failed to load tasks")?;
    let tree = TaskTree::from_tasks(&all_tasks);

    let to_mark: Vec<Uuid> = if args.recursive {
        let mut descendants = tree.descendants(uuid);
        descendants.reverse(); // leaves first
        descendants.push(uuid);
        descendants
    } else {
        let pending = tree.pending_child_ids(uuid, &all_tasks);
        if !pending.is_empty() {
            anyhow::bail!(
                "Task has pending subtasks: {}. Use --recursive to cascade, or complete them first.",
                pending.join(", ")
            );
        }
        vec![uuid]
    };

    let mut ops = Operations::new();
    for task_uuid in &to_mark {
        if let Some(task) = all_tasks
            .get_mut(task_uuid)
            .filter(|t| matches!(t.get_status(), Status::Pending))
        {
            task.done(&mut ops)?;
        }
    }

    replica
        .commit_operations(ops)
        .await
        .context("Failed to commit")?;

    println!("Done: {}", short_id(&uuid));
    Ok(())
}
