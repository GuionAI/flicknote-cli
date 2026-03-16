use anyhow::{Context, Result};
use taskchampion::{PowerSyncStorage, Replica, Status, Uuid};

use crate::{
    display::print_subtree,
    ids::{resolve_id, short_id},
    task_tree::TaskTree,
};
use clap::Args;

#[derive(Args)]
pub struct TreeArgs {
    /// Show subtree of this task only (8-char hex or full UUID)
    pub id: Option<String>,

    /// Max depth to display (default: unlimited)
    #[arg(long)]
    pub depth: Option<usize>,
}

pub async fn run(replica: &mut Replica<PowerSyncStorage>, args: TreeArgs) -> Result<()> {
    let root_uuid = if let Some(ref id) = args.id {
        Some(resolve_id(replica, id).await?)
    } else {
        None
    };

    let all_tasks = replica.all_tasks().await.context("Failed to load tasks")?;
    let tree = TaskTree::from_tasks(&all_tasks);

    let roots: Vec<Uuid> = if let Some(uuid) = root_uuid {
        vec![uuid]
    } else {
        // All pending root tasks, sorted by description
        let mut r: Vec<Uuid> = tree
            .roots()
            .into_iter()
            .filter(|uuid| {
                all_tasks
                    .get(uuid)
                    .map(|t| matches!(t.get_status(), Status::Pending))
                    .unwrap_or(false)
            })
            .collect();
        r.sort_by_key(|uuid| {
            all_tasks
                .get(uuid)
                .map(|t| t.get_description().to_string())
                .unwrap_or_default()
        });
        r
    };

    for root in &roots {
        let desc = all_tasks
            .get(root)
            .map(|t| t.get_description().to_string())
            .unwrap_or_default();
        println!("[{}] {desc}", short_id(root));
        print_subtree(
            &all_tasks,
            &tree,
            *root,
            args.depth,
            0,
            "",
            &|child_uuid, child| {
                let sid = short_id(&child_uuid);
                let status = match child.get_status() {
                    Status::Completed => "[done] ",
                    Status::Deleted => "[del] ",
                    _ => "",
                };
                Some(format!("[{sid}] {status}{}", child.get_description()))
            },
        );
    }

    Ok(())
}
