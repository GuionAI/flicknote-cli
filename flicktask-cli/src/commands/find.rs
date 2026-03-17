use anyhow::{Context, Result};
use clap::Args;
use taskchampion::{PowerSyncStorage, Replica, Status};

use crate::ids::short_id;

#[derive(Args)]
pub struct FindArgs {
    /// Keywords to search for (OR match)
    pub keywords: Vec<String>,

    /// Show completed tasks instead of pending
    #[arg(long)]
    pub completed: bool,
}

pub async fn run(replica: &mut Replica<PowerSyncStorage>, args: FindArgs) -> Result<()> {
    if args.keywords.is_empty() {
        anyhow::bail!("At least one keyword is required");
    }

    let target_status = if args.completed {
        Status::Completed
    } else {
        Status::Pending
    };

    let all_tasks = replica.all_tasks().await.context("Failed to load tasks")?;

    let keywords_lower: Vec<String> = args.keywords.iter().map(|k| k.to_lowercase()).collect();

    let mut matches: Vec<_> = all_tasks
        .iter()
        .filter(|(_, task)| {
            task.get_status() == target_status && {
                let desc = task.get_description().to_lowercase();
                keywords_lower.iter().any(|kw| desc.contains(kw.as_str()))
            }
        })
        .collect();

    if matches.is_empty() {
        let status_str = if args.completed {
            "completed"
        } else {
            "pending"
        };
        eprintln!(
            "No {status_str} tasks found matching: {}",
            args.keywords.join(" | ")
        );
        return Ok(());
    }

    // Sort by description for stable output
    matches.sort_by_key(|(_, task)| task.get_description().to_string());

    // Print table
    println!("{:<10} {:<8} {:<10} Description", "ID", "Pri", "Project");
    println!("{}", "-".repeat(60));

    for (uuid, task) in &matches {
        let project = task.get_value("project").unwrap_or("");
        println!(
            "{:<10} {:<8} {:<10} {}",
            short_id(uuid),
            task.get_priority(),
            project,
            task.get_description()
        );
    }

    let count = matches.len();
    let plural = if count == 1 { "task" } else { "tasks" };
    println!("\n{count} {plural}");
    Ok(())
}
