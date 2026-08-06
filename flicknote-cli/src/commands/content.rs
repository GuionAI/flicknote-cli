use clap::Args;
use flicknote_core::backend::NoteDb;
use flicknote_core::error::CliError;
use flicknote_core::services::note::NoteService;

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

pub(crate) async fn run(db: &dyn NoteDb, args: &ContentArgs) -> Result<(), CliError> {
    let service = NoteService::new(db);
    let output = match args.section.as_deref() {
        Some(section) => service.get_section(&args.id, section).await?.content,
        None => service.get(&args.id, false).await?.content,
    };
    print!("{output}");
    Ok(())
}
