use anyhow::{Context, Result};
use clap::Args;
use std::io::Read;
use taskchampion::{Annotation, Operations, PowerSyncStorage, Replica};

use crate::ids::{resolve_id, short_id};

#[derive(Args)]
pub struct AnnotateArgs {
    /// Task ID (8-char hex or full UUID)
    pub id: String,

    /// Annotation text (if omitted, reads from stdin)
    pub message: Option<String>,
}

pub async fn run(replica: &mut Replica<PowerSyncStorage>, args: AnnotateArgs) -> Result<()> {
    let uuid = resolve_id(replica, &args.id).await?;

    let message = if let Some(msg) = args.message {
        msg
    } else {
        let mut buf = String::new();
        std::io::stdin()
            .read_to_string(&mut buf)
            .context("Failed to read annotation from stdin")?;
        buf.trim_end().to_string()
    };

    if message.is_empty() {
        anyhow::bail!("Annotation message cannot be empty");
    }

    let mut all_tasks = replica.all_tasks().await.context("Failed to load tasks")?;

    let task = all_tasks
        .get_mut(&uuid)
        .with_context(|| format!("Task {} not found", args.id))?;

    let mut ops = Operations::new();
    task.add_annotation(
        Annotation {
            entry: taskchampion::chrono::Utc::now(),
            description: message,
        },
        &mut ops,
    )?;

    replica
        .commit_operations(ops)
        .await
        .context("Failed to commit")?;

    println!("Annotated: {}", short_id(&uuid));
    Ok(())
}
