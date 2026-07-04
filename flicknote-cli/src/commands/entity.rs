use clap::{Args, Subcommand};
use flicknote_core::ENTITY_EXTRACTION_KEYS;
use flicknote_core::backend::NoteDb;
use flicknote_core::error::CliError;

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

pub(crate) async fn run(db: &dyn NoteDb, args: &EntityArgs) -> Result<(), CliError> {
    match &args.command {
        EntityCommands::List(args) => list(db, args).await,
    }
}

async fn list(db: &dyn NoteDb, args: &ListArgs) -> Result<(), CliError> {
    let typed_key;
    let keys = if let Some(ref entity_type) = args.entity_type {
        typed_key = format!("::{entity_type}");
        vec![typed_key.as_str()]
    } else {
        ENTITY_EXTRACTION_KEYS.to_vec()
    };
    let values = db.list_extraction_values(&keys, false).await?;
    println!("{}", values.join(", "));
    Ok(())
}
