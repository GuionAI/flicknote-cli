use anyhow::Result;
use clap::{Parser, Subcommand};
use taskchampion::{Operations, PowerSyncStorage, Replica, Task, Uuid};

use crate::config::FlicktaskConfig;

/// Structural fields that hooks must not override via raw set_value.
/// Combined with `tw_json::TIMESTAMP_KEYS` to form the full skip set.
const STRUCTURAL_SKIP: &[&str] = &[
    "uuid",
    "tags",
    "annotations",
    "urgency",
    "id",
    "parent",
    "status",
    "description",
    "priority",
    "project",
    "position",
];

/// Apply UDA fields added/changed by a hook back to the task via `set_value`.
/// Only processes scalar string UDAs (branch, project_path, spawner, pr_id, etc.).
/// Skips structural fields, timestamps, and core task fields — hooks cannot
/// override description/status/priority/parent via this path (they must use typed setters).
pub fn apply_hook_fields(
    final_json: &serde_json::Value,
    pre_json: &serde_json::Value,
    task: &mut Task,
    ops: &mut Operations,
) -> anyhow::Result<()> {
    let Some(obj) = final_json.as_object() else {
        return Ok(());
    };
    for (key, value) in obj {
        if STRUCTURAL_SKIP.contains(&key.as_str())
            || crate::tw_json::TIMESTAMP_KEYS.contains(&key.as_str())
        {
            continue;
        }
        let Some(str_val) = value.as_str() else {
            eprintln!(
                "flicktask: hook returned non-string value for {key:?} — skipping (hooks must use string values for UDAs)"
            );
            continue;
        };
        let pre_val = pre_json.get(key).and_then(|v| v.as_str());
        if pre_val != Some(str_val) {
            task.set_value(key, Some(str_val.to_string()), ops)?;
        }
    }
    Ok(())
}

/// Snapshot → mutate → run on-modify hook → apply hook fields back.
///
/// Extracts the repeated pattern from all single-task modify commands.
pub fn with_on_modify<F>(
    uuid: &Uuid,
    task: &mut Task,
    ops: &mut Operations,
    mutate: F,
) -> anyhow::Result<()>
where
    F: FnOnce(&mut Task, &mut Operations) -> anyhow::Result<()>,
{
    let uuid_str = uuid.to_string();
    let original_json = crate::tw_json::task_to_tw_json(&uuid_str, task);
    mutate(task, ops)?;
    let modified_json = crate::tw_json::task_to_tw_json(&uuid_str, task);
    let final_json = crate::hooks::run_on_modify(&original_json, &modified_json)?;
    apply_hook_fields(&final_json, &modified_json, task, ops)
}

pub mod add;
pub mod annotate;
pub mod delete;
pub mod done;
pub mod edit;
pub mod export;
pub mod find;
pub mod get;
pub mod import;
pub mod list;
pub mod move_task;
pub mod plan;
pub mod start;
pub mod stop;
pub mod tag;
pub mod today;
pub mod tree;
pub mod undo;
pub mod untag;

#[derive(Parser)]
#[command(
    name = "flicktask",
    about = "FlickTask CLI — tree-based task management"
)]
#[command(version)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Add a task
    Add(add::AddArgs),
    /// Show a task with its subtree
    Get(get::GetArgs),
    /// Mark a task as done
    Done(done::DoneArgs),
    /// Delete a task
    Delete(delete::DeleteArgs),
    /// Start tracking time on a task
    Start(start::StartArgs),
    /// Stop tracking time on a task
    Stop(stop::StopArgs),
    /// Edit task properties
    Edit(edit::EditArgs),
    /// Add a tag to a task
    Tag(tag::TagArgs),
    /// Remove a tag from a task
    Untag(untag::UntagArgs),
    /// Add an annotation to a task
    Annotate(annotate::AnnotateArgs),
    /// Move a task to a new parent (or to root)
    #[clap(name = "move")]
    MoveTask(move_task::MoveArgs),
    /// List tasks
    List(list::ListArgs),
    /// Show task tree
    Tree(tree::TreeArgs),
    /// Create subtask tree from markdown (piped via stdin)
    Plan(plan::PlanArgs),
    /// Undo the last change
    Undo(undo::UndoArgs),
    /// Import tasks from taskwarrior export JSON (piped via stdin)
    Import(import::ImportArgs),
    /// Export tasks as taskwarrior-compatible JSON
    Export(export::ExportArgs),
    /// Find tasks by keywords (OR match)
    Find(find::FindArgs),
    /// Manage today's task focus list
    Today(today::TodayArgs),
}

pub async fn dispatch(
    replica: &mut Replica<PowerSyncStorage>,
    config: &FlicktaskConfig,
    cli: Cli,
) -> Result<()> {
    match cli.command {
        Commands::Add(args) => add::run(replica, args).await,
        Commands::Get(args) => get::run(replica, config, args).await,
        Commands::Done(args) => done::run(replica, args).await,
        Commands::Delete(args) => delete::run(replica, args).await,
        Commands::Start(args) => start::run(replica, args).await,
        Commands::Stop(args) => stop::run(replica, args).await,
        Commands::Edit(args) => edit::run(replica, args).await,
        Commands::Tag(args) => tag::run(replica, args).await,
        Commands::Untag(args) => untag::run(replica, args).await,
        Commands::Annotate(args) => annotate::run(replica, args).await,
        Commands::MoveTask(args) => move_task::run(replica, args).await,
        Commands::List(args) => list::run(replica, args).await,
        Commands::Tree(args) => tree::run(replica, args).await,
        Commands::Plan(args) => plan::run(replica, args).await,
        Commands::Undo(args) => undo::run(replica, args).await,
        Commands::Import(args) => import::run(replica, args).await,
        Commands::Export(args) => export::run(replica, args).await,
        Commands::Find(args) => find::run(replica, args).await,
        Commands::Today(args) => today::run(replica, args).await,
    }
}
