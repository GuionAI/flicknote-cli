use anyhow::{Context, Result};
use clap::Args;
use taskchampion::{Operations, PowerSyncStorage, Replica, Uuid};

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

    /// Place after this sibling task ID (8-char hex or full UUID)
    #[arg(long, conflicts_with_all = ["root", "before"])]
    pub after: Option<String>,

    /// Place before this sibling task ID (8-char hex or full UUID)
    #[arg(long, conflicts_with_all = ["root", "after"])]
    pub before: Option<String>,
}

pub async fn run(replica: &mut Replica<PowerSyncStorage>, args: MoveArgs) -> Result<()> {
    if args.parent.is_none() && !args.root && args.after.is_none() && args.before.is_none() {
        anyhow::bail!("Specify --parent <id>, --root, --after <id>, or --before <id>");
    }

    let uuid = resolve_id(replica, &args.id).await?;

    // Resolve referenced task IDs before loading all_tasks
    let new_parent_uuid = if let Some(ref parent_id) = args.parent {
        Some(resolve_id(replica, parent_id).await?)
    } else {
        None
    };
    let after_uuid = if let Some(ref after_id) = args.after {
        Some(resolve_id(replica, after_id).await?)
    } else {
        None
    };
    let before_uuid = if let Some(ref before_id) = args.before {
        Some(resolve_id(replica, before_id).await?)
    } else {
        None
    };

    // Load all_tasks once for all subsequent reads
    let mut all_tasks = replica.all_tasks().await.context("Failed to load tasks")?;
    let tree = TaskTree::from_tasks(&all_tasks);

    // Circular ref check for explicit --parent
    if let Some(new_parent) = new_parent_uuid {
        if new_parent == uuid {
            anyhow::bail!("A task cannot be its own parent");
        }
        if tree.is_ancestor(new_parent, uuid) {
            anyhow::bail!(
                "Circular reference: {} is already a descendant of {}",
                short_id(&new_parent),
                args.id
            );
        }
    }

    // Compute the target parent (explicit --parent, inferred from --after/--before, or None for --root)
    let target_parent = if let Some(p) = new_parent_uuid {
        Some(p)
    } else if args.root {
        None
    } else {
        // Infer parent from --after/--before sibling
        let ref_uuid = after_uuid.or(before_uuid).unwrap();
        let ref_task = all_tasks
            .get(&ref_uuid)
            .with_context(|| format!("Task {} not found", short_id(&ref_uuid)))?;
        match ref_task.get_value("parent") {
            None => None,
            Some(p) => Some(Uuid::parse_str(p).with_context(|| {
                format!(
                    "Task {} has invalid parent UUID: {p:?}",
                    short_id(&ref_uuid)
                )
            })?),
        }
    };

    // Circular ref check for inferred parent (--after/--before without explicit --parent)
    if new_parent_uuid.is_none()
        && let Some(tp) = target_parent
    {
        if tp == uuid {
            anyhow::bail!("A task cannot be its own parent");
        }
        if tree.is_ancestor(tp, uuid) {
            anyhow::bail!(
                "Circular reference: {} is already a descendant of {}",
                short_id(&tp),
                args.id
            );
        }
    }

    // Compute position — exclude the task being moved from sibling list
    let position: Option<String> = if after_uuid.is_some() || before_uuid.is_some() {
        let siblings = tree.sibling_positions(target_parent, &all_tasks, Some(uuid));

        if let Some(a_uuid) = after_uuid {
            // Clone position string to release the borrow on all_tasks
            let a_pos = {
                let a_task = all_tasks.get(&a_uuid).context("After task not found")?;
                a_task.get_value("position").map(str::to_string)
            };
            match a_pos {
                Some(ref ap) => {
                    let next = siblings
                        .iter()
                        .skip_while(|(u, _)| *u != a_uuid)
                        .nth(1)
                        .map(|(_, p)| p.as_str());
                    match next {
                        Some(np) => Some(crate::position::between_position(ap, np)?),
                        None => Some(crate::position::append_position(Some(ap))?),
                    }
                }
                None => {
                    eprintln!(
                        "Warning: anchor task {} has no position — appending at end",
                        short_id(&a_uuid)
                    );
                    let last = siblings.last().map(|(_, p)| p.as_str());
                    Some(crate::position::append_position(last)?)
                }
            }
        } else {
            let b_uuid = before_uuid.unwrap();
            // Clone position string to release the borrow on all_tasks
            let b_pos = {
                let b_task = all_tasks.get(&b_uuid).context("Before task not found")?;
                b_task.get_value("position").map(str::to_string)
            };
            match b_pos {
                Some(ref bp) => {
                    let prev = siblings
                        .iter()
                        .take_while(|(u, _)| *u != b_uuid)
                        .last()
                        .map(|(_, p)| p.as_str());
                    match prev {
                        Some(pp) => Some(crate::position::between_position(pp, bp)?),
                        None => Some(crate::position::prepend_position(Some(bp))?),
                    }
                }
                None => {
                    eprintln!(
                        "Warning: anchor task {} has no position — prepending at start",
                        short_id(&b_uuid)
                    );
                    let first = siblings.first().map(|(_, p)| p.as_str());
                    Some(crate::position::prepend_position(first)?)
                }
            }
        }
    } else {
        // Moving to new parent without --after/--before — append at end
        let siblings = tree.sibling_positions(target_parent, &all_tasks, Some(uuid));
        let last = siblings.last().map(|(_, p)| p.as_str());
        Some(crate::position::append_position(last)?)
    };

    let task = all_tasks
        .get_mut(&uuid)
        .with_context(|| format!("Task {} not found", args.id))?;

    let mut ops = Operations::new();
    // is_root is a bool copy — avoids partial-move of args into the closure below
    let is_root = args.root;
    super::with_on_modify(&uuid, task, &mut ops, |task, ops| {
        match target_parent {
            Some(p_uuid) => task.set_value("parent", Some(p_uuid.to_string()), ops)?,
            None if is_root => task.set_value("parent", None, ops)?,
            _ => {} // --after/--before without --parent: same parent, just reordering
        }
        if let Some(ref pos) = position {
            task.set_value("position", Some(pos.clone()), ops)?;
        }
        Ok(())
    })?;

    replica
        .commit_operations(ops)
        .await
        .context("Failed to commit")?;

    let sid = short_id(&uuid);
    if is_root {
        println!("Moved {sid} to root");
    } else if let Some(p) = target_parent {
        println!("Moved {sid} under {}", short_id(&p));
    } else {
        println!("Reordered {sid}");
    }
    Ok(())
}
