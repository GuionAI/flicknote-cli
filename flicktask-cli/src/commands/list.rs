use anyhow::{Context, Result};
use clap::Args;
use taskchampion::{PowerSyncStorage, Replica, Status, Task, Uuid};

use crate::{ids::short_id, task_tree::TaskTree};

#[derive(Args)]
pub struct ListArgs {
    /// Filter by tag
    #[arg(long = "tag", short = 't')]
    pub tag: Option<String>,

    /// Filter by due: today, week, overdue
    #[arg(long)]
    pub due: Option<String>,

    /// Filter by priority (H, M, or L)
    #[arg(long)]
    pub priority: Option<String>,

    /// Show completed tasks instead
    #[arg(long)]
    pub completed: bool,
}

pub async fn run(replica: &mut Replica<PowerSyncStorage>, args: ListArgs) -> Result<()> {
    // Validate --due early so the user gets a clear error
    if let Some(ref due_filter) = args.due
        && !matches!(due_filter.as_str(), "today" | "week" | "overdue")
    {
        anyhow::bail!(
            "Unknown due filter {:?} — valid values: today, week, overdue",
            due_filter
        );
    }

    let all_tasks = replica.all_tasks().await.context("Failed to load tasks")?;
    let tree = TaskTree::from_tasks(&all_tasks);

    let target_status = if args.completed {
        Status::Completed
    } else {
        Status::Pending
    };

    // Get roots matching target status
    let mut roots: Vec<Uuid> = tree
        .roots()
        .into_iter()
        .filter(|uuid| {
            all_tasks
                .get(uuid)
                .map(|t| t.get_status() == target_status)
                .unwrap_or(false)
        })
        .collect();

    // Apply filters
    if args.tag.is_some() || args.due.is_some() || args.priority.is_some() {
        roots.retain(|uuid| {
            all_tasks
                .get(uuid)
                .map(|t| matches_filters(t, &args))
                .unwrap_or(false)
        });
    }

    // Sort by description for stable output
    roots.sort_by_key(|uuid| {
        all_tasks
            .get(uuid)
            .map(|t| t.get_description().to_string())
            .unwrap_or_default()
    });

    if roots.is_empty() {
        println!("No tasks.");
        return Ok(());
    }

    // Print table header
    println!(
        "{:<10} {:<8} {:<12} {:<10}",
        "ID", "Pri", "Due", "Description"
    );
    println!("{}", "-".repeat(60));

    for uuid in roots {
        let Some(task) = all_tasks.get(&uuid) else {
            continue;
        };

        let priority = task.get_priority();
        let due = task
            .get_due()
            .map(|d| d.format("%Y-%m-%d").to_string())
            .unwrap_or_default();

        // Count pending subtasks
        let pending_children = tree
            .descendants(uuid)
            .into_iter()
            .filter(|u| {
                all_tasks
                    .get(u)
                    .map(|t| matches!(t.get_status(), Status::Pending))
                    .unwrap_or(false)
            })
            .count();

        let desc = task.get_description();
        let desc_with_count = if pending_children > 0 {
            format!("{desc} (+{pending_children})")
        } else {
            desc.to_string()
        };

        println!(
            "{:<10} {:<8} {:<12} {}",
            short_id(&uuid),
            priority,
            due,
            desc_with_count
        );
    }

    Ok(())
}

fn matches_filters(task: &Task, args: &ListArgs) -> bool {
    if let Some(ref tag_str) = args.tag {
        let has_tag = task.get_tags().any(|t| t.to_string() == *tag_str);
        if !has_tag {
            return false;
        }
    }

    if args
        .priority
        .as_deref()
        .is_some_and(|p| task.get_priority() != p)
    {
        return false;
    }

    if let Some(ref due_filter) = args.due {
        let now = taskchampion::chrono::Utc::now();
        match due_filter.as_str() {
            "overdue" => {
                let Some(due) = task.get_due() else {
                    return false;
                };
                if due >= now {
                    return false;
                }
            }
            "today" => {
                let Some(due) = task.get_due() else {
                    return false;
                };
                let today = now.date_naive();
                if due.date_naive() != today {
                    return false;
                }
            }
            "week" => {
                let Some(due) = task.get_due() else {
                    return false;
                };
                let week_end = now + taskchampion::chrono::Duration::days(7);
                // Exclude overdue tasks (due < now) and tasks beyond next 7 days
                if due < now || due > week_end {
                    return false;
                }
            }
            _ => {} // Already validated above; unreachable
        }
    }

    true
}
