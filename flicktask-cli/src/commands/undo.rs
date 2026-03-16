use anyhow::{Context, Result};
use clap::Args;
use taskchampion::{PowerSyncStorage, Replica};

#[derive(Args)]
pub struct UndoArgs {}

pub async fn run(replica: &mut Replica<PowerSyncStorage>, _args: UndoArgs) -> Result<()> {
    let ops = replica
        .get_undo_operations()
        .await
        .context("Failed to get undo operations")?;

    if ops.is_empty() {
        println!("Nothing to undo");
        return Ok(());
    }

    let count = ops.len();
    let undone = replica
        .commit_reversed_operations(ops)
        .await
        .context("Failed to commit reversed operations")?;

    if undone {
        println!("Undone: {count} operation(s)");
    } else {
        println!("Nothing to undo");
    }

    Ok(())
}
