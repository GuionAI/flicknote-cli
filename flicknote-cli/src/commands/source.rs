use clap::{Args, Subcommand};
use flicknote_core::backend::NoteDb;
use flicknote_core::error::CliError;

const SOURCE_HELP: &str = include_str!("../help/source.md");

#[derive(Args)]
#[command(after_help = SOURCE_HELP)]
pub(crate) struct SourceArgs {
    #[command(subcommand)]
    command: SourceCommands,
}

#[derive(Subcommand)]
enum SourceCommands {
    /// Show the raw source stored for a note
    Show(ShowArgs),
}

#[derive(Args)]
struct ShowArgs {
    /// Note ID. Use the numeric short ID shown in list/detail. Full UUIDs are also accepted.
    id: String,
    /// Read an archived note
    #[arg(long)]
    archived: bool,
}

pub(crate) async fn run(db: &dyn NoteDb, args: &SourceArgs) -> Result<(), CliError> {
    match &args.command {
        SourceCommands::Show(args) => show(db, args).await,
    }
}

async fn show(db: &dyn NoteDb, args: &ShowArgs) -> Result<(), CliError> {
    let full_id = if args.archived {
        db.resolve_archived_note_id(&args.id).await?
    } else {
        db.resolve_note_id(&args.id).await?
    };
    let note = if args.archived {
        db.find_archived_note(&full_id).await?
    } else {
        db.find_note(&full_id).await?
    };
    let source = note
        .source
        .ok_or_else(|| CliError::Other("This note has no source data".into()))?;
    print!("{source}");
    if !source.ends_with('\n') {
        println!();
    }
    Ok(())
}
