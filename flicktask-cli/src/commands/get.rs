use anyhow::{Context, Result};
use clap::Args;
use taskchampion::{PowerSyncStorage, Replica, Status, Task, Uuid};

use crate::{
    config::FlicktaskConfig,
    display::print_subtree,
    ids::{resolve_id, short_id},
    task_tree::TaskTree,
};

// Standard properties shown in dedicated fields (not UDA section)
const KNOWN_PROPS: &[&str] = &[
    "status",
    "description",
    "entry",
    "modified",
    "due",
    "wait",
    "scheduled",
    "start",
    "end",
    "priority",
    "project",
    "parent",
    "tags",
    "position",
];

#[derive(Args)]
pub struct GetArgs {
    /// Task ID (8-char hex or full UUID)
    pub id: String,

    /// Max subtask depth to show (default: unlimited)
    #[arg(long)]
    pub depth: Option<usize>,
}

pub async fn run(
    replica: &mut Replica<PowerSyncStorage>,
    config: &FlicktaskConfig,
    args: GetArgs,
) -> Result<()> {
    let uuid = resolve_id(replica, &args.id).await?;
    let all_tasks = replica.all_tasks().await.context("Failed to load tasks")?;

    let task = all_tasks
        .get(&uuid)
        .with_context(|| format!("Task {} not found", args.id))?;

    print_task_full(task, config);

    let tree = TaskTree::from_tasks(&all_tasks);
    print_subtree(
        &all_tasks,
        &tree,
        uuid,
        args.depth,
        0,
        "",
        &|child_uuid, child| {
            let sid = short_id(&child_uuid);
            let status = format_status_short(&child.get_status());
            let mut line = format!("[{sid}] {status}{}", child.get_description());
            // Show first annotation inline
            if let Some(ann) = child.get_annotations().next() {
                let first_line = ann.description.lines().next().unwrap_or("");
                if !first_line.is_empty() {
                    line.push_str(&format!(" # {first_line}"));
                }
            }
            Some(line)
        },
    );

    Ok(())
}

fn print_task_full(task: &Task, config: &FlicktaskConfig) {
    let uuid = task.get_uuid();

    println!("ID:          {}", short_id(&uuid));
    println!("UUID:        {uuid}");
    println!("Description: {}", task.get_description());
    println!("Status:      {}", format_status(&task.get_status()));

    if !task.get_priority().is_empty() {
        println!("Priority:    {}", task.get_priority());
    }

    if let Some(due) = task.get_due() {
        println!("Due:         {}", due.format("%Y-%m-%d"));
    }

    if let Some(wait) = task.get_wait() {
        println!("Wait:        {}", wait.format("%Y-%m-%d"));
    }

    if let Some(project) = task.get_value("project") {
        println!("Project:     {project}");
    }

    if let Some(parent_str) = task.get_value("parent") {
        // Show the short hex ID if it looks like a UUID
        let parent_short = if parent_str.len() >= 32 {
            Uuid::parse_str(parent_str)
                .map(|u| short_id(&u))
                .unwrap_or_else(|_| parent_str.to_string())
        } else {
            parent_str.to_string()
        };
        println!("Parent:      {parent_short}");
    }

    let tags: Vec<String> = task.get_tags().map(|t| t.to_string()).collect();
    if !tags.is_empty() {
        println!("Tags:        {}", tags.join(", "));
    }

    if let Some(entry) = task.get_entry() {
        println!("Created:     {}", entry.format("%Y-%m-%d %H:%M:%S"));
    }

    // UDAs from config
    let mut udas: Vec<(String, String)> = Vec::new();
    for key in config.uda.keys() {
        if KNOWN_PROPS.contains(&key.as_str()) {
            continue;
        }
        if let Some(value) = task.get_value(key.as_str()) {
            udas.push((config.uda_label(key).to_string(), value.to_string()));
        }
    }
    if !udas.is_empty() {
        println!("---");
        for (label, value) in udas {
            println!("{label}: {value}");
        }
    }

    let annotations: Vec<_> = task.get_annotations().collect();
    if !annotations.is_empty() {
        println!("---");
        for ann in annotations {
            println!("[{}] {}", ann.entry.format("%Y-%m-%d"), ann.description);
        }
    }
}

pub fn format_status(status: &Status) -> &'static str {
    match status {
        Status::Pending => "pending",
        Status::Completed => "completed",
        Status::Deleted => "deleted",
        Status::Recurring => "recurring",
        _ => "unknown",
    }
}

pub fn format_status_short(status: &Status) -> &'static str {
    match status {
        Status::Pending => "",
        Status::Completed => "[done] ",
        Status::Deleted => "[del] ",
        Status::Recurring => "[rec] ",
        _ => "",
    }
}
