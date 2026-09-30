use clap::{Args, Subcommand};
use flicknote_client::{AppRequest, DaemonClient};
use flicknote_core::TOPIC_EXTRACTION_KEY;
use flicknote_core::error::CliError;

const TOPIC_HELP: &str = include_str!("../help/topic.md");
const TOPIC_LIST_HELP: &str = "Examples:
  flicknote topic list

Prints known topics as a comma-separated list.";

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
#[command(after_help = TOPIC_LIST_HELP)]
struct ListArgs {}

pub(crate) async fn run(daemon: &DaemonClient, args: &TopicArgs) -> Result<(), CliError> {
    match &args.command {
        TopicCommands::List(args) => list(daemon, args).await,
    }
}

async fn list(daemon: &DaemonClient, _args: &ListArgs) -> Result<(), CliError> {
    let values: Vec<String> = daemon
        .call(AppRequest::ExtractionValues {
            keys: vec![TOPIC_EXTRACTION_KEY.to_string()],
            archived: false,
        })
        .await?;
    println!("{}", values.join(", "));
    Ok(())
}
