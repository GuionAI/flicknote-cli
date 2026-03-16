use anyhow::Result;
use clap::{Parser, Subcommand};
use taskchampion::{PowerSyncStorage, Replica};

use crate::config::FlicktaskConfig;

pub mod add;
pub mod annotate;
pub mod delete;
pub mod done;
pub mod edit;
pub mod get;
pub mod list;
pub mod move_task;
pub mod plan;
pub mod start;
pub mod stop;
pub mod tag;
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
    }
}
