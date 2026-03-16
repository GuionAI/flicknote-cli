use anyhow::{Context, Result};
use clap::Args;
use std::io::Read;
use taskchampion::{Annotation, Operations, PowerSyncStorage, Replica, Status, Uuid};

use crate::{
    display::print_subtree,
    ids::{resolve_id, short_id},
    task_tree::TaskTree,
};

#[derive(Args)]
pub struct PlanArgs {
    /// Parent task ID to attach the plan under
    pub id: String,

    /// Replace existing subtasks before creating from markdown
    #[arg(long)]
    pub replace: bool,
}

struct Section {
    level: usize,
    heading: String,
    body: String,
}

/// A task to be created (planned synchronously, created asynchronously in batch).
struct TaskSpec {
    uuid: Uuid,
    parent: Uuid,
    description: String,
    annotation: Option<String>,
}

pub async fn run(replica: &mut Replica<PowerSyncStorage>, args: PlanArgs) -> Result<()> {
    let uuid = resolve_id(replica, &args.id).await?;

    // Read markdown from stdin
    let mut input = String::new();
    std::io::stdin()
        .read_to_string(&mut input)
        .context("Failed to read markdown from stdin")?;

    let sections = parse_markdown(&input);
    if sections.is_empty() {
        anyhow::bail!("No headings found in markdown input");
    }

    let all_tasks = replica.all_tasks().await.context("Failed to load tasks")?;
    let tree = TaskTree::from_tasks(&all_tasks);

    // Check for existing children
    let existing_children = tree.children(uuid);
    if !existing_children.is_empty() && !args.replace {
        anyhow::bail!(
            "Task already has {} subtask(s). Use --replace to delete them first.",
            existing_children.len()
        );
    }

    // Delete existing descendants if --replace
    if args.replace && !existing_children.is_empty() {
        let mut all_tasks_mut = replica.all_tasks().await.context("Failed to load tasks")?;
        let mut descendants = tree.descendants(uuid);
        descendants.reverse(); // leaves first
        let mut ops = Operations::new();
        for desc_uuid in &descendants {
            if let Some(task) = all_tasks_mut.get_mut(desc_uuid) {
                task.set_status(Status::Deleted, &mut ops)?;
            }
        }
        replica
            .commit_operations(ops)
            .await
            .context("Failed to delete existing subtasks")?;
    }

    // Plan the task tree synchronously (no async, no recursion)
    let specs = plan_tasks(uuid, &sections, 0, sections.len());

    // Create all tasks in a single batch
    let now = taskchampion::chrono::Utc::now();
    let mut ops = Operations::new();
    for spec in &specs {
        let mut task = replica
            .create_task(spec.uuid, &mut ops)
            .await
            .context("Failed to create task")?;

        task.set_description(spec.description.clone(), &mut ops)?;
        task.set_status(Status::Pending, &mut ops)?;
        task.set_value("entry", Some(now.timestamp().to_string()), &mut ops)?;
        task.set_value("parent", Some(spec.parent.to_string()), &mut ops)?;

        if let Some(ref ann_text) = spec.annotation {
            task.add_annotation(
                Annotation {
                    entry: now,
                    description: ann_text.clone(),
                },
                &mut ops,
            )?;
        }
    }
    replica
        .commit_operations(ops)
        .await
        .context("Failed to commit plan tasks")?;

    // Show resulting subtree
    let all_tasks = replica.all_tasks().await.context("Failed to load tasks")?;
    let tree = TaskTree::from_tasks(&all_tasks);
    let root_desc = all_tasks
        .get(&uuid)
        .map(taskchampion::Task::get_description)
        .unwrap_or("?");
    println!("[{}] {root_desc}", short_id(&uuid));
    print_subtree(
        &all_tasks,
        &tree,
        uuid,
        None,
        0,
        "",
        &|child_uuid, child| {
            Some(format!(
                "[{}] {}",
                short_id(&child_uuid),
                child.get_description()
            ))
        },
    );

    Ok(())
}

/// Build a flat list of TaskSpecs from a section slice.
/// Purely synchronous — no async, no recursion via Box::pin.
fn plan_tasks(parent: Uuid, sections: &[Section], start: usize, end: usize) -> Vec<TaskSpec> {
    let mut result = Vec::new();
    let mut i = start;

    while i < end {
        let section = &sections[i];
        let task_uuid = Uuid::new_v4();

        result.push(TaskSpec {
            uuid: task_uuid,
            parent,
            description: section.heading.clone(),
            annotation: if section.body.trim().is_empty() {
                None
            } else {
                Some(section.body.trim().to_string())
            },
        });

        // Find child sections (deeper level immediately following)
        let child_start = i + 1;
        let mut child_end = child_start;
        while child_end < end && sections[child_end].level > section.level {
            child_end += 1;
        }

        if child_end > child_start {
            let mut children = plan_tasks(task_uuid, sections, child_start, child_end);
            result.append(&mut children);
        }

        i = child_end;
    }

    result
}

fn parse_markdown(input: &str) -> Vec<Section> {
    let mut sections = Vec::new();
    let mut current_level: Option<usize> = None;
    let mut current_heading = String::new();
    let mut current_body = String::new();

    for line in input.lines() {
        if let Some(level) = heading_level(line) {
            // Save previous section (skip if heading is empty)
            if let Some(prev_level) = current_level {
                let heading = current_heading.trim().to_string();
                if !heading.is_empty() {
                    sections.push(Section {
                        level: prev_level,
                        heading,
                        body: current_body.trim_end().to_string(),
                    });
                } else {
                    eprintln!("Warning: empty heading at level {prev_level} — skipped");
                }
            }
            current_level = Some(level);
            current_heading = strip_heading(line).to_string();
            current_body = String::new();
        } else if current_level.is_some() {
            current_body.push_str(line);
            current_body.push('\n');
        }
    }

    // Save last section
    if let Some(level) = current_level {
        let heading = current_heading.trim().to_string();
        if !heading.is_empty() {
            sections.push(Section {
                level,
                heading,
                body: current_body.trim_end().to_string(),
            });
        }
    }

    sections
}

fn heading_level(line: &str) -> Option<usize> {
    if !line.starts_with('#') {
        return None;
    }
    let level = line.chars().take_while(|&c| c == '#').count();
    let rest = &line[level..];
    if rest.starts_with(' ') || rest.is_empty() {
        Some(level)
    } else {
        None
    }
}

fn strip_heading(line: &str) -> &str {
    line.trim_start_matches('#').trim_start()
}
