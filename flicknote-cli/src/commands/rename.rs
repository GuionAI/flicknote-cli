use clap::Args;
use flicknote_core::backend::NoteDb;
use flicknote_core::config::Config;
use flicknote_core::error::CliError;
use flicknote_core::services::note::NoteService;

use super::util::{display_summary_id, print_section_tree};

#[derive(Args)]
pub(crate) struct RenameArgs {
    /// Note ID. Use the numeric short ID shown in list/detail. Full UUIDs are also accepted for compatibility.
    id: String,
    /// Section heading to rename (case-insensitive contains match)
    #[arg(short = 's', long = "section")]
    section: String,
    /// New heading text (without # prefix — level is preserved)
    name: String,
}

pub(crate) async fn run(
    db: &dyn NoteDb,
    _config: &Config,
    args: &RenameArgs,
) -> Result<(), CliError> {
    let result = NoteService::new(db)
        .rename_section(&args.id, &args.section, &args.name)
        .await?;
    println!(
        "Renamed section {} → '{}' in note {}.\n",
        args.section,
        args.name,
        display_summary_id(&result.note)
    );
    print_section_tree(&result.sections);
    Ok(())
}
