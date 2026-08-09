use clap::{Args, Subcommand};
use flicknote_core::ENTITY_EXTRACTION_KEYS;
use flicknote_core::error::CliError;
use flicknote_sync::ipc::{AppRequest, DaemonClient};

const ENTITY_HELP: &str = include_str!("../help/entity.md");
const ENTITY_LIST_HELP: &str = "Examples:
  flicknote entity list
  flicknote entity list --type person
  flicknote entity list --type company

Prints known entities as a comma-separated list.";

#[derive(Args)]
#[command(after_help = ENTITY_HELP)]
pub(crate) struct EntityArgs {
    #[command(subcommand)]
    command: EntityCommands,
}

#[derive(Subcommand)]
enum EntityCommands {
    /// List known entities
    List(ListArgs),
}

#[derive(Args)]
#[command(after_help = ENTITY_LIST_HELP)]
struct ListArgs {
    /// Filter by entity type
    #[arg(long = "type", value_parser = ["person", "company", "location", "product"])]
    entity_type: Option<String>,
}

pub(crate) async fn run(daemon: &DaemonClient<'_>, args: &EntityArgs) -> Result<(), CliError> {
    match &args.command {
        EntityCommands::List(args) => list(daemon, args).await,
    }
}

async fn list(daemon: &DaemonClient<'_>, args: &ListArgs) -> Result<(), CliError> {
    let keys = if let Some(ref entity_type) = args.entity_type {
        vec![format!("::{entity_type}")]
    } else {
        ENTITY_EXTRACTION_KEYS
            .iter()
            .map(|key| (*key).to_string())
            .collect()
    };
    let values: Vec<String> = daemon
        .call(AppRequest::ExtractionValues {
            keys,
            archived: false,
        })
        .await?;
    println!("{}", values.join(", "));
    Ok(())
}
