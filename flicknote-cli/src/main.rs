#![allow(clippy::print_stdout, clippy::print_stderr)]

use clap::{Parser, Subcommand};
use flicknote_core::config::Config;
use flicknote_core::db::Database;
use flicknote_core::error::CliError;

mod api_client;
mod commands;
mod tui;

#[derive(Parser)]
#[command(
    name = "flicknote",
    about = "FlickNote CLI — local-first note management"
)]
#[command(version)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Add a note (text or URL — auto-detected)
    Add(commands::add::AddArgs),
    /// Archive a note (soft-delete)
    Archive(commands::archive::ArchiveArgs),
    /// List notes
    List(commands::list::ListArgs),
    /// Get a note by ID
    Get(commands::get::GetArgs),
    /// Link an existing note to a taskwarrior task
    Link(commands::link::LinkArgs),
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
    /// Interact with FlickNote API directly
    Api(commands::api::ApiArgs),
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

    match cli.command {
        Commands::Add(args) => commands::add::run(&Database::open_local(&config)?, &config, &args),
        Commands::Archive(args) => commands::archive::run(&Database::open_local(&config)?, &args),
        Commands::List(args) => commands::list::run(&Database::open_local(&config)?, &args),
        Commands::Get(args) => commands::get::run(&Database::open_local(&config)?, &args),
        Commands::Link(args) => {
            commands::link::run(&Database::open_local(&config)?, &config, &args)
        }
        Commands::Project(args) => commands::project::run(&Database::open_local(&config)?, &args),
        Commands::Login(args) => commands::login::run(&config, &args),
        Commands::Logout => commands::logout::run(&config),
        Commands::Tui => commands::tui::run(&config),
        Commands::Sync(args) => commands::sync::run(&config, &args),
        Commands::Import(args) => {
            commands::import::run(&Database::open_local(&config)?, &config, &args)
        }
        Commands::Api(args) => commands::api::run(&config, &args),
    }
}
