#![allow(clippy::print_stdout, clippy::print_stderr)]

use clap::{CommandFactory, Parser, Subcommand};
use flicknote_core::backend::NoteDb;
#[cfg(feature = "powersync")]
use flicknote_core::backend::SqliteBackend;
use flicknote_core::config::Config;
#[cfg(feature = "powersync")]
use flicknote_core::db::Database;
use flicknote_core::error::CliError;

mod api_client;
mod commands;
mod markdown;
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

    /// Launch interactive TUI
    #[arg(short = 't', long = "tui")]
    tui: bool,

    /// Project filter (used with -t)
    #[arg(long = "project", requires = "tui")]
    project: Option<String>,
}

#[derive(Subcommand)]
enum Commands {
    /// Add a note (text or URL — auto-detected)
    Add(commands::add::AddArgs),
    /// Append content to an existing note
    Append(commands::append::AppendArgs),
    /// Delete a note (soft-delete) or remove a section
    Delete(commands::delete::DeleteArgs),
    /// Edit a note in $EDITOR, or create a new note from editor
    Edit(commands::edit::EditArgs),
    /// Restore a deleted note
    Restore(commands::restore::RestoreArgs),
    /// List notes
    List(commands::list::ListArgs),
    /// Count notes matching filters
    Count(commands::count::CountArgs),
    /// Find notes by keyword (OR match across title, content, summary)
    Find(commands::find::FindArgs),
    /// Show note details with full metadata
    Detail(commands::detail::DetailArgs),
    /// Show note content with section IDs
    Content(commands::content::ContentArgs),
    /// Manage projects
    Project(commands::project::ProjectArgs),
    /// Manage prompts
    Prompt(commands::prompt::PromptArgs),
    /// Manage keyterm sets
    Keyterm(commands::keyterm::KeytermArgs),
    /// Authenticate with FlickNote
    Login(commands::login::LoginArgs),
    /// Log out — remove saved session
    Logout,
    /// Manage sync daemon
    Sync(commands::sync::SyncArgs),
    /// Import markdown files as notes
    Import(commands::import::ImportArgs),
    /// Upload a file and create a file-type note
    Upload(commands::upload::UploadArgs),
    /// Interact with FlickNote API directly
    Api(commands::api::ApiArgs),
    /// Rename a section heading in a note
    Rename(commands::rename::RenameArgs),
    /// Insert content before or after a section
    Insert(commands::insert::InsertArgs),
    /// Modify note content (via stdin) and/or metadata
    Modify(commands::modify::ModifyArgs),
    /// Open a note in the browser
    Open(commands::open::OpenArgs),
}

fn main() {
    if let Err(e) = run() {
        eprintln!("Error: {e:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), CliError> {
    let cli = Cli::parse();
    let config = Config::load()?;

    // Commands that don't need a database connection or session
    if let Some(ref cmd) = cli.command {
        match cmd {
            Commands::Login(args) => return commands::login::run(&config, args),
            Commands::Logout => return commands::logout::run(&config),
            Commands::Sync(args) => return commands::sync::run(&config, args),
            _ => {}
        }
    }

    if cli.tui {
        let mut cmd = std::process::Command::new("flicknote-tui");
        if let Some(ref project) = cli.project {
            cmd.arg("--project").arg(project);
        }
        let status = cmd.status().map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                CliError::Other(
                    "flicknote-tui not found — install it with: make install-tui".into(),
                )
            } else {
                CliError::Other(format!("failed to launch flicknote-tui: {e}"))
            }
        })?;
        std::process::exit(status.code().unwrap_or(1));
    }

    // Backend selection: FLICKNOTE_TOKEN → pgwire, else → SQLite
    #[cfg(feature = "storage-pgwire")]
    if let Ok(token) = std::env::var("FLICKNOTE_TOKEN") {
        let database_url = std::env::var("DATABASE_URL").map_err(|_| {
            CliError::Other("pgwire mode (FLICKNOTE_TOKEN set) requires DATABASE_URL".into())
        })?;
        let backend = flicknote_core::pgwire::PgWireBackend::connect(&database_url, &token)?;
        return dispatch(&cli, &config, &backend);
    }

    #[cfg(not(feature = "powersync"))]
    return Err(CliError::Other(
        "No storage backend available — set FLICKNOTE_TOKEN + DATABASE_URL for pgwire, \
         or build with powersync feature for local storage"
            .into(),
    ));

    #[cfg(feature = "powersync")]
    {
        let db = Database::open_local(&config)?;
        let user_id = flicknote_core::session::get_user_id(&config)?;
        let backend = SqliteBackend { db, user_id };
        dispatch(&cli, &config, &backend)
    }
}

fn dispatch(cli: &Cli, config: &Config, db: &dyn NoteDb) -> Result<(), CliError> {
    let Some(ref command) = cli.command else {
        Cli::command()
            .print_help()
            .map_err(|e| CliError::Other(e.to_string()))?;
        return Ok(());
    };

    match command {
        Commands::Add(args) => commands::add::run(db, config, args),
        Commands::Append(args) => commands::append::run(db, config, args),
        Commands::Delete(args) => commands::delete::run(db, config, args),
        Commands::Edit(args) => commands::edit::run(db, config, args),
        Commands::Restore(args) => commands::restore::run(db, config, args),
        Commands::List(args) => commands::list::run(db, args),
        Commands::Count(args) => commands::count::run(db, args),
        Commands::Find(args) => commands::find::run(db, args),
        Commands::Detail(args) => commands::detail::run(db, config, args),
        Commands::Content(args) => commands::content::run(db, args),
        Commands::Project(args) => commands::project::run(db, args),
        Commands::Prompt(args) => commands::prompt::run(db, args),
        Commands::Keyterm(args) => commands::keyterm::run(db, args),
        Commands::Rename(args) => commands::rename::run(db, config, args),
        Commands::Insert(args) => commands::insert::run(db, config, args),
        Commands::Modify(args) => commands::modify::run(db, config, args),
        Commands::Open(args) => commands::open::run(db, config, args),
        Commands::Import(args) => commands::import::run(db, config, args),
        Commands::Upload(args) => commands::upload::run(db, config, args),
        Commands::Api(args) => commands::api::run(config, args),
        // Login/Logout/Sync are handled before dispatch() is called
        Commands::Login(_) | Commands::Logout | Commands::Sync(_) => unreachable!(),
    }
}
