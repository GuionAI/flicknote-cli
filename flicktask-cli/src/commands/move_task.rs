use anyhow::{Context, Result};
use clap::Args;
use taskchampion::{Operations, PowerSyncStorage, Replica};

use crate::{
    ids::{resolve_id, short_id},
    task_tree::TaskTree,
};

#[derive(Args)]
pub struct MoveArgs {
    /// Task ID (8-char hex or full UUID)
    pub id: String,

    /// New parent task ID
    #[arg(long, conflicts_with = "root")]
    pub parent: Option<String>,

    /// Move to root (clear parent)
    #[arg(long, conflicts_with = "parent")]
    pub root: bool,
}

pub async fn run(replica: &mut Replica<PowerSyncStorage>, args: MoveArgs) -> Result<()> {
    if args.parent.is_none() && !args.root {
        anyhow::bail!("Specify --parent <id> or --root");
    }

    let uuid = resolve_id(replica, &args.id).await?;

    // Resolve new parent before loading all_tasks
    let new_parent_uuid = if let Some(ref parent_id) = args.parent {
        Some(resolve_id(replica, parent_id).await?)
    } else {
        None
    };

    // Validate no circular reference
    if let Some(new_parent) = new_parent_uuid {
        if new_parent == uuid {
            anyhow::bail!("A task cannot be its own parent");
        }
        let all_tasks = replica.all_tasks().await.context("Failed to load tasks")?;
        let tree = TaskTree::from_tasks(&all_tasks);
        if tree.is_ancestor(new_parent, uuid) {
            anyhow::bail!(
                "Circular reference: {} is already a descendant of {}",
                &args.parent.unwrap_or_default(),
                args.id
            );
        }
    }

    let mut all_tasks = replica.all_tasks().await.context("Failed to load tasks")?;
    let task = all_tasks
        .get_mut(&uuid)
        .with_context(|| format!("Task {} not found", args.id))?;

    let mut ops = Operations::new();
    super::with_on_modify(&uuid, task, &mut ops, |task, ops| {
        if let Some(p_uuid) = new_parent_uuid {
            task.set_value("parent", Some(p_uuid.to_string()), ops)?;
        } else {
            // Clear parent (move to root)
            task.set_value("parent", None, ops)?;
        }
        Ok(())
    })?;

    replica
        .commit_operations(ops)
        .await
        .context("Failed to commit")?;

    let sid = short_id(&uuid);
    if args.root {
        println!("Moved {sid} to root");
    } else {
        let p_short = new_parent_uuid.map(|u| short_id(&u)).unwrap_or_default();
        println!("Moved {sid} under {p_short}");
    }
    Ok(())
}
