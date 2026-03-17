use std::collections::HashSet;

use anyhow::{Context, Result};
use clap::{Args, Subcommand};
use taskchampion::chrono::{Local, TimeZone, Utc};
use taskchampion::{Operations, PowerSyncStorage, Replica, Status, Task, Uuid};

use crate::ids::{resolve_id, short_id};

#[derive(Args)]
pub struct TodayArgs {
    #[command(subcommand)]
    pub command: TodayCommands,
}

#[derive(Subcommand)]
pub enum TodayCommands {
    /// Show today's scheduled tasks
    List,
    /// Show tasks completed today
    Completed,
    /// Add tasks to today's focus list
    Add(TodayAddArgs),
    /// Remove tasks from today's focus list
    Remove(TodayRemoveArgs),
}

#[derive(Args)]
pub struct TodayAddArgs {
    /// Task IDs (8-char hex or full UUID)
    pub ids: Vec<String>,
}

#[derive(Args)]
pub struct TodayRemoveArgs {
    /// Task IDs (8-char hex or full UUID)
    pub ids: Vec<String>,
}

pub async fn run(replica: &mut Replica<PowerSyncStorage>, args: TodayArgs) -> Result<()> {
    match args.command {
        TodayCommands::List => list(replica).await,
        TodayCommands::Completed => completed(replica).await,
        TodayCommands::Add(a) => add(replica, a.ids).await,
        TodayCommands::Remove(a) => remove(replica, a.ids).await,
    }
}

/// Parse a unix epoch string to i64 seconds.
fn parse_epoch_secs(s: &str) -> Option<i64> {
    s.parse().ok()
}

/// Parse a unix epoch string to DateTime<Utc>.
fn parse_epoch(s: &str) -> Option<taskchampion::chrono::DateTime<Utc>> {
    Utc.timestamp_opt(parse_epoch_secs(s)?, 0).single()
}

/// Check if a task's scheduled date is on or before today (local timezone).
/// Logs a warning if `scheduled` exists but is unparseable.
fn is_scheduled_today_or_earlier(task: &Task) -> bool {
    let Some(sched_str) = task.get_value("scheduled") else {
        return false;
    };
    let Some(sched) = parse_epoch(sched_str) else {
        eprintln!(
            "flicktask: task has invalid 'scheduled' value {:?} — skipping",
            sched_str
        );
        return false;
    };
    let today = Local::now().date_naive();
    sched.with_timezone(&Local).date_naive() <= today
}

/// Resolve and deduplicate a list of task ID strings into UUIDs.
async fn resolve_unique_ids(
    replica: &mut Replica<PowerSyncStorage>,
    ids: &[String],
) -> Result<Vec<Uuid>> {
    let mut seen = HashSet::new();
    let mut uuids = Vec::with_capacity(ids.len());
    for id in ids {
        let uuid = resolve_id(replica, id).await?;
        if seen.insert(uuid) {
            uuids.push(uuid);
        }
    }
    Ok(uuids)
}

async fn list(replica: &mut Replica<PowerSyncStorage>) -> Result<()> {
    let all_tasks = replica.all_tasks().await.context("Failed to load tasks")?;

    let mut scheduled: Vec<(&Uuid, &Task)> = all_tasks
        .iter()
        .filter(|(_, task)| {
            task.get_status() == Status::Pending && is_scheduled_today_or_earlier(task)
        })
        .collect();

    if scheduled.is_empty() {
        println!("No tasks scheduled for today.");
        return Ok(());
    }

    // Sort by entry date descending (newest first), parsed as i64 for correctness
    scheduled.sort_by(|(_, a), (_, b)| {
        let a_entry: i64 = a
            .get_value("entry")
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        let b_entry: i64 = b
            .get_value("entry")
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        b_entry.cmp(&a_entry)
    });

    println!(
        "{:<10} {:<8} {:<12} {:<10} Description",
        "ID", "Pri", "Due", "Project"
    );
    println!("{}", "-".repeat(70));

    for (uuid, task) in &scheduled {
        let due = task
            .get_due()
            .map(|d| d.format("%Y-%m-%d").to_string())
            .unwrap_or_default();
        let project = task.get_value("project").unwrap_or("");
        println!(
            "{:<10} {:<8} {:<12} {:<10} {}",
            short_id(uuid),
            task.get_priority(),
            due,
            project,
            task.get_description()
        );
    }

    let count = scheduled.len();
    let plural = if count == 1 { "task" } else { "tasks" };
    println!("\n{count} {plural}");
    Ok(())
}

async fn completed(replica: &mut Replica<PowerSyncStorage>) -> Result<()> {
    let all_tasks = replica.all_tasks().await.context("Failed to load tasks")?;
    let today = Local::now().date_naive();

    let mut done_today: Vec<(&Uuid, &Task)> = all_tasks
        .iter()
        .filter(|(_, task)| {
            if task.get_status() != Status::Completed {
                return false;
            }
            match task.get_value("end") {
                None => false,
                Some(s) => match parse_epoch(s) {
                    None => {
                        eprintln!("flicktask: task has invalid 'end' value {:?} — skipping", s);
                        false
                    }
                    Some(end) => end.with_timezone(&Local).date_naive() == today,
                },
            }
        })
        .collect();

    if done_today.is_empty() {
        println!("No tasks completed today.");
        return Ok(());
    }

    // Sort by end time descending (most recent first), parsed as i64 for correctness
    done_today.sort_by(|(_, a), (_, b)| {
        let a_end: i64 = a.get_value("end").and_then(|s| s.parse().ok()).unwrap_or(0);
        let b_end: i64 = b.get_value("end").and_then(|s| s.parse().ok()).unwrap_or(0);
        b_end.cmp(&a_end)
    });

    println!("{:<10} {:<10} Description", "ID", "Project");
    println!("{}", "-".repeat(60));

    for (uuid, task) in &done_today {
        let project = task.get_value("project").unwrap_or("");
        println!(
            "{:<10} {:<10} {}",
            short_id(uuid),
            project,
            task.get_description()
        );
    }

    let count = done_today.len();
    let plural = if count == 1 { "task" } else { "tasks" };
    println!("\n{count} {plural}");
    Ok(())
}

async fn add(replica: &mut Replica<PowerSyncStorage>, ids: Vec<String>) -> Result<()> {
    if ids.is_empty() {
        anyhow::bail!("At least one task ID is required");
    }

    let uuids = resolve_unique_ids(replica, &ids).await?;

    // all_tasks is an owned HashMap — borrows don't conflict with replica.commit_operations
    let mut all_tasks = replica.all_tasks().await.context("Failed to load tasks")?;
    // Local midnight → UTC epoch, so "today" matches the user's timezone.
    // Use .single().context() rather than .expect() to handle DST-ambiguous midnight gracefully.
    let today_epoch = Local::now()
        .date_naive()
        .and_hms_opt(0, 0, 0)
        .expect("midnight valid")
        .and_local_timezone(Local)
        .single()
        .context("DST-ambiguous local midnight — try again in a few minutes")?
        .with_timezone(&Utc)
        .timestamp()
        .to_string();

    for uuid in &uuids {
        let task = all_tasks
            .get_mut(uuid)
            .with_context(|| format!("Task {} not found", short_id(uuid)))?;
        // Fresh ops per iteration — each task gets its own commit
        let mut ops = Operations::new();
        super::with_on_modify(uuid, task, &mut ops, |task, ops| {
            task.set_value("scheduled", Some(today_epoch.clone()), ops)?;
            Ok(())
        })?;
        replica
            .commit_operations(ops)
            .await
            .context("Failed to commit")?;
        println!("Task {} added to today", short_id(uuid));
    }

    Ok(())
}

async fn remove(replica: &mut Replica<PowerSyncStorage>, ids: Vec<String>) -> Result<()> {
    if ids.is_empty() {
        anyhow::bail!("At least one task ID is required");
    }

    let uuids = resolve_unique_ids(replica, &ids).await?;

    // all_tasks is an owned HashMap — borrows don't conflict with replica.commit_operations
    let mut all_tasks = replica.all_tasks().await.context("Failed to load tasks")?;

    for uuid in &uuids {
        let task = all_tasks
            .get_mut(uuid)
            .with_context(|| format!("Task {} not found", short_id(uuid)))?;

        let had_scheduled = task.get_value("scheduled").is_some();

        // Fresh ops per iteration — each task gets its own commit
        let mut ops = Operations::new();
        super::with_on_modify(uuid, task, &mut ops, |task, ops| {
            task.set_value("scheduled", None::<String>, ops)?;
            Ok(())
        })?;
        replica
            .commit_operations(ops)
            .await
            .context("Failed to commit")?;

        if had_scheduled {
            println!("Task {} removed from today", short_id(uuid));
        } else {
            println!("Task {} was not scheduled", short_id(uuid));
        }
    }

    Ok(())
}
