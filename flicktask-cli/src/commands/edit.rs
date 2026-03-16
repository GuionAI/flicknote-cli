use anyhow::{Context, Result};
use clap::Args;
use taskchampion::{Operations, PowerSyncStorage, Replica};

use crate::{
    commands::add::{parse_date, parse_kv},
    ids::{resolve_id, short_id},
};

#[derive(Args)]
pub struct EditArgs {
    /// Task ID (8-char hex or full UUID)
    pub id: String,

    /// New description
    #[arg(long)]
    pub description: Option<String>,

    /// Due date (YYYY-MM-DD)
    #[arg(long)]
    pub due: Option<String>,

    /// Priority (H, M, or L)
    #[arg(long)]
    pub priority: Option<String>,

    /// New parent task ID (8-char hex or full UUID)
    #[arg(long)]
    pub parent: Option<String>,

    /// Wait date (YYYY-MM-DD)
    #[arg(long)]
    pub wait: Option<String>,

    /// Project name
    #[arg(long)]
    pub project: Option<String>,

    /// Set a UDA value (key=value, repeatable)
    #[arg(long = "set", value_name = "KEY=VALUE")]
    pub set: Vec<String>,
}

pub async fn run(replica: &mut Replica<PowerSyncStorage>, args: EditArgs) -> Result<()> {
    let uuid = resolve_id(replica, &args.id).await?;

    // Resolve parent before borrowing all_tasks mutably
    let parent_uuid = if let Some(ref parent_id) = args.parent {
        Some(resolve_id(replica, parent_id).await?)
    } else {
        None
    };

    let mut all_tasks = replica.all_tasks().await.context("Failed to load tasks")?;

    let task = all_tasks
        .get_mut(&uuid)
        .with_context(|| format!("Task {} not found", args.id))?;

    let mut ops = Operations::new();

    if let Some(desc) = args.description {
        task.set_description(desc, &mut ops)?;
    }

    if let Some(due_str) = args.due {
        let due = parse_date(&due_str)?;
        task.set_due(Some(due), &mut ops)?;
    }

    if let Some(priority) = args.priority {
        task.set_priority(priority, &mut ops)?;
    }

    if let Some(wait_str) = args.wait {
        let wait = parse_date(&wait_str)?;
        task.set_wait(Some(wait), &mut ops)?;
    }

    if let Some(project) = args.project {
        task.set_value("project", Some(project), &mut ops)?;
    }

    if let Some(p_uuid) = parent_uuid {
        task.set_value("parent", Some(p_uuid.to_string()), &mut ops)?;
    }

    for kv in args.set {
        let (key, value) = parse_kv(&kv)?;
        task.set_value(key, Some(value.to_string()), &mut ops)?;
    }

    replica
        .commit_operations(ops)
        .await
        .context("Failed to commit")?;

    println!("Updated: {}", short_id(&uuid));
    Ok(())
}
