use anyhow::{Context, Result};
use clap::Args;
use taskchampion::{PowerSyncStorage, Replica};

use crate::ids::resolve_id;

#[derive(Args)]
pub struct ExportArgs {
    /// Export a single task by ID (8-char hex or full UUID)
    pub id: Option<String>,

    /// Export completed tasks instead of pending
    #[arg(long)]
    pub completed: bool,

    /// Export deleted tasks instead of pending
    #[arg(long)]
    pub deleted: bool,
}

pub async fn run(replica: &mut Replica<PowerSyncStorage>, args: ExportArgs) -> Result<()> {
    let all_tasks = replica.all_tasks().await.context("Failed to load tasks")?;

    if let Some(id_str) = args.id {
        let uuid = resolve_id(replica, &id_str).await?;
        let task = all_tasks
            .get(&uuid)
            .with_context(|| format!("Task {id_str} not found"))?;
        let json = crate::tw_json::task_to_tw_json(&uuid.to_string(), task);
        println!("{}", serde_json::to_string_pretty(&json)?);
        return Ok(());
    }

    let mut results: Vec<serde_json::Value> = all_tasks
        .iter()
        .filter(|(_, task)| {
            let status = task.get_value("status").unwrap_or("pending");
            status_matches(status, args.completed, args.deleted)
        })
        .map(|(uuid, task)| crate::tw_json::task_to_tw_json(&uuid.to_string(), task))
        .collect();

    // Stable sort by entry timestamp for deterministic output
    results.sort_by(|a, b| {
        a["entry"]
            .as_str()
            .unwrap_or("")
            .cmp(b["entry"].as_str().unwrap_or(""))
    });

    println!("{}", serde_json::to_string_pretty(&results)?);
    Ok(())
}

/// Match tasks by requested status mode.
///
/// If both `--completed` and `--deleted` are passed, completed takes precedence (first match wins).
pub(crate) fn status_matches(status: &str, want_completed: bool, want_deleted: bool) -> bool {
    match (want_completed, want_deleted) {
        (true, _) => status == "completed",
        (_, true) => status == "deleted",
        _ => status == "pending",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_status_filter_pending() {
        assert!(status_matches("pending", false, false));
        assert!(!status_matches("completed", false, false));
        assert!(!status_matches("deleted", false, false));
    }

    #[test]
    fn test_status_filter_completed() {
        assert!(!status_matches("pending", true, false));
        assert!(status_matches("completed", true, false));
        assert!(!status_matches("deleted", true, false));
    }

    #[test]
    fn test_status_filter_deleted() {
        assert!(!status_matches("pending", false, true));
        assert!(!status_matches("completed", false, true));
        assert!(status_matches("deleted", false, true));
    }
}
