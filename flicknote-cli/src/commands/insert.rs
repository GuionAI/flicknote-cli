use clap::Args;
use flicknote_core::backend::NoteDb;
use flicknote_core::config::Config;
use flicknote_core::error::CliError;
use flicknote_core::services::dto::InsertPosition;
use flicknote_core::services::note::NoteService;

use super::util::{display_summary_id, print_section_tree, read_stdin_required};

const INSERT_HELP: &str = include_str!("../help/insert.md");

#[derive(Args)]
#[command(
    group(clap::ArgGroup::new("position").required(true)),
    after_help = INSERT_HELP
)]
pub(crate) struct InsertArgs {
    /// Note ID. Use the numeric short ID shown in list/detail. Full UUIDs are also accepted for compatibility.
    id: String,
    /// Insert before this section
    #[arg(long, group = "position")]
    before: Option<String>,
    /// Insert after this section
    #[arg(long, group = "position")]
    after: Option<String>,
}

pub(crate) async fn run(
    db: &dyn NoteDb,
    _config: &Config,
    args: &InsertArgs,
) -> Result<(), CliError> {
    let (section, position) = match (&args.before, &args.after) {
        (Some(section), None) => (section.as_str(), InsertPosition::Before),
        (None, Some(section)) => (section.as_str(), InsertPosition::After),
        _ => {
            return Err(CliError::Other(
                "Exactly one of --before or --after is required.".into(),
            ));
        }
    };

    let insert_content = read_stdin_required()?;
    let result = NoteService::new(db)
        .insert(&args.id, section, position, &insert_content)
        .await?;
    let position = match position {
        InsertPosition::Before => "before",
        InsertPosition::After => "after",
    };
    println!(
        "Inserted content {position} section {section} in note {}.\n",
        display_summary_id(&result.note)
    );
    print_section_tree(&result.sections);
    Ok(())
}
