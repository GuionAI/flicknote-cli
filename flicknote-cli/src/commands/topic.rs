use clap::{Args, Subcommand};
use flicknote_core::TOPIC_EXTRACTION_KEY;
use flicknote_core::backend::NoteDb;
use flicknote_core::error::CliError;

const TOPIC_HELP: &str = include_str!("../help/topic.md");

#[derive(Args)]
#[command(after_help = TOPIC_HELP)]
pub(crate) struct TopicArgs {
    #[command(subcommand)]
    command: TopicCommands,
}

#[derive(Subcommand)]
enum TopicCommands {
    /// List known topics
    List(ListArgs),
}

#[derive(Args)]
struct ListArgs {}

pub(crate) async fn run(db: &dyn NoteDb, args: &TopicArgs) -> Result<(), CliError> {
    match &args.command {
        TopicCommands::List(args) => list(db, args).await,
    }
}

async fn list(db: &dyn NoteDb, _args: &ListArgs) -> Result<(), CliError> {
    let values = db
        .list_extraction_values(&[TOPIC_EXTRACTION_KEY], false)
        .await?;
    println!("{}", values.join(", "));
    Ok(())
}
