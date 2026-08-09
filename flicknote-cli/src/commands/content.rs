use clap::Args;
use flicknote_core::error::CliError;
use flicknote_core::services::dto::{NoteDetail, NoteSectionResult};
use flicknote_sync::ipc::{AppRequest, DaemonClient};

const CONTENT_HELP: &str = include_str!("../help/content.md");

#[derive(Args)]
#[command(after_help = CONTENT_HELP)]
pub(crate) struct ContentArgs {
    /// Note ID. Use the numeric short ID shown in list/detail. Full UUIDs are also accepted.
    id: String,
    /// Extract a specific section by section ID
    #[arg(short = 's', long = "section")]
    section: Option<String>,
}

pub(crate) async fn run(daemon: &DaemonClient<'_>, args: &ContentArgs) -> Result<(), CliError> {
    let output = match args.section.as_deref() {
        Some(section) => {
            daemon
                .call::<NoteSectionResult>(AppRequest::NoteGetSection {
                    id: args.id.clone(),
                    section: section.to_string(),
                })
                .await?
                .content
        }
        None => {
            daemon
                .call::<NoteDetail>(AppRequest::NoteGet {
                    id: args.id.clone(),
                    archived: false,
                })
                .await?
                .content
        }
    };
    print!("{output}");
    Ok(())
}
