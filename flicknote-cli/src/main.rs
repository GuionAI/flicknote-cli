#![allow(clippy::print_stdout, clippy::print_stderr)]

use clap::{Parser, Subcommand};
use flicknote_core::backend::NoteDb;
#[cfg(feature = "powersync")]
use flicknote_core::backend::SqliteBackend;
use flicknote_core::config::Config;
#[cfg(feature = "powersync")]
use flicknote_core::db::Database;
use flicknote_core::error::CliError;
use flicknote_core::pg::PgBackend;

mod api_client;
mod commands;
mod markdown;
mod tui;
mod utils;

#[derive(Parser)]
#[command(
    name = "flicknote",
    about = "FlickNote CLI — local-first note management"
)]
#[command(version)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Add a note (text or URL — auto-detected)
    Add(commands::add::AddArgs),
    /// Append content to an existing note
    Append(commands::append::AppendArgs),
    /// Archive a note (soft-delete)
    Archive(commands::archive::ArchiveArgs),
    /// Unarchive a note (restore from archive)
    Unarchive(commands::unarchive::UnarchiveArgs),
    /// List notes
    List(commands::list::ListArgs),
    /// Find notes by keyword (OR match across title, content, summary)
    Find(commands::find::FindArgs),
    /// Get a note by ID
    Get(commands::get::GetArgs),
    /// Manage projects
    Project(commands::project::ProjectArgs),
    /// Authenticate with FlickNote
    Login(commands::login::LoginArgs),
    /// Log out — remove saved session
    Logout,
    /// Interactive TUI for browsing notes
    Tui,
    /// Manage sync daemon
    Sync(commands::sync::SyncArgs),
    /// Import markdown files as notes
    Import(commands::import::ImportArgs),
    /// Upload a file and create a file-type note
    Upload(commands::upload::UploadArgs),
    /// Interact with FlickNote API directly
    Api(commands::api::ApiArgs),
    /// Replace entire note content
    Replace(commands::replace::ReplaceArgs),
    /// Remove a section from a note by heading name
    Remove(commands::remove::RemoveArgs),
    /// Rename a section heading in a note
    Rename(commands::rename::RenameArgs),
    /// Insert content before or after a section
    Insert(commands::insert::InsertArgs),
    /// Modify note metadata (e.g. move to another project)
    Modify(commands::modify::ModifyArgs),
}

fn main() {
    if let Err(e) = run() {
        eprintln!("Error: {e}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), CliError> {
    let cli = Cli::parse();
    let config = Config::load()?;

    if let Ok(pg_url) = std::env::var("FLICKNOTE_PG_URL") {
        // Reject unsupported commands before attempting connection
        if let Some(ref cmd) = cli.command {
            let unsupported = matches!(
                cmd,
                Commands::Login(_) | Commands::Logout | Commands::Sync(_) | Commands::Tui
            );
            if unsupported {
                return Err(CliError::Other(
                    "This command is not available in PG mode (FLICKNOTE_PG_URL is set)".into(),
                ));
            }
        }

        let user_id = std::env::var("FLICKNOTE_USER_ID").map_err(|_| {
            CliError::Other(
                "FLICKNOTE_USER_ID must be set when using FLICKNOTE_PG_URL \
                 (e.g. FLICKNOTE_USER_ID=<your-uuid>)"
                    .into(),
            )
        })?;
        uuid::Uuid::parse_str(&user_id).map_err(|_| {
            CliError::Other(format!(
                "FLICKNOTE_USER_ID must be a valid UUID, got: {user_id:?}"
            ))
        })?;
        let backend = PgBackend::connect(&pg_url, user_id)?;
        dispatch(&cli, &config, &backend, true)
    } else {
        #[cfg(not(feature = "powersync"))]
        return Err(CliError::Other(
            "No local database available — set FLICKNOTE_PG_URL to connect to Postgres".into(),
        ));
        #[cfg(feature = "powersync")]
        {
            let db = Database::open_local(&config)?;
            let user_id = flicknote_core::session::get_user_id(&config)?;
            let backend = SqliteBackend { db, user_id };
            dispatch(&cli, &config, &backend, false)
        }
    }
}

fn dispatch(cli: &Cli, config: &Config, db: &dyn NoteDb, pg_mode: bool) -> Result<(), CliError> {
    let Some(ref command) = cli.command else {
        if pg_mode {
            return Err(CliError::Other("TUI not supported in PG mode".into()));
        }
        return commands::tui::run(config, db);
    };

    match command {
        Commands::Add(args) => commands::add::run(db, config, args),
        Commands::Append(args) => commands::append::run(db, config, args),
        Commands::Archive(args) => commands::archive::run(db, args),
        Commands::Unarchive(args) => commands::unarchive::run(db, args),
        Commands::List(args) => commands::list::run(db, args),
        Commands::Find(args) => commands::find::run(db, args),
        Commands::Get(args) => commands::get::run(db, args),
        Commands::Project(args) => commands::project::run(db, args),
        Commands::Replace(args) => commands::replace::run(db, config, args),
        Commands::Remove(args) => commands::remove::run(db, config, args),
        Commands::Rename(args) => commands::rename::run(db, config, args),
        Commands::Insert(args) => commands::insert::run(db, config, args),
        Commands::Modify(args) => commands::modify::run(db, config, args),
        Commands::Import(args) => commands::import::run(db, config, args),
        Commands::Upload(args) => commands::upload::run(db, config, args),
        Commands::Login(args) => commands::login::run(config, args),
        Commands::Logout => commands::logout::run(config),
        Commands::Tui => commands::tui::run(config, db),
        Commands::Sync(args) => commands::sync::run(config, args),
        Commands::Api(args) => commands::api::run(config, args),
    }
}
