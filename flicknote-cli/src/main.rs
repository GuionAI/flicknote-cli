#![allow(clippy::print_stdout, clippy::print_stderr)]

use clap::{CommandFactory, Parser, Subcommand, error::ErrorKind};
use flicknote_core::config::Config;
use flicknote_core::error::CliError;
use flicknote_sync::ipc::DaemonClient;
use std::ffi::OsStr;

const ROOT_HELP: &str = include_str!("help/root.md");

mod commands;
mod gateway;
mod mcp;
mod recall;

#[derive(Parser)]
#[command(
    name = "flicknote",
    about = "FlickNote CLI — local-first note management",
    after_help = ROOT_HELP
)]
#[command(version)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Run the local MCP server over stdio
    Mcp,
    /// Add a note (text or URL — auto-detected)
    Add(commands::add::AddArgs),
    /// Import or upload a file as a note
    Upload(commands::upload::UploadArgs),
    /// Append content to an existing note
    Append(commands::append::AppendArgs),
    /// Delete (archive) a note
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
    /// Recall historical note candidates by text, or process a Codex hook event
    Recall(commands::recall::RecallArgs),
    /// Discover topics
    Topic(commands::topic::TopicArgs),
    /// Discover entities
    Entity(commands::entity::EntityArgs),
    /// Make a safe authenticated request to the configured Gateway
    Gateway(commands::gateway::GatewayArgs),
    /// Install host lifecycle hooks
    Hook(commands::hook::HookArgs),
    /// Inspect raw note sources
    Source(commands::source::SourceArgs),
    /// Show note details with full metadata
    Detail(commands::detail::DetailArgs),
    /// Show note content
    Content(commands::content::ContentArgs),
    /// Get or create a share link for a note
    Share(commands::share::ShareArgs),
    /// Revoke the share link for a note
    Unshare(commands::share::UnshareArgs),
    /// Manage projects
    Project(commands::project::ProjectArgs),
    /// Authenticate with FlickNote
    Login(commands::login::LoginArgs),
    /// Log out — remove the daemon, session, and local data
    Logout(commands::logout::LogoutArgs),
    /// Manage the FlickNote daemon
    Daemon(commands::daemon::DaemonArgs),
    /// Install agent skills
    Skill(commands::skill::SkillArgs),
    /// Import markdown files as notes
    Import(commands::import::ImportArgs),
    /// Modify note metadata
    Modify(commands::modify::ModifyArgs),
    /// Open a note in the browser
    Open(commands::open::OpenArgs),
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let hook_invocation = recall_hook_argv();
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(error)
            if hook_invocation
                && !matches!(
                    error.kind(),
                    ErrorKind::DisplayHelp | ErrorKind::DisplayVersion
                ) =>
        {
            eprintln!("{error}");
            std::process::exit(1);
        }
        Err(error) => error.exit(),
    };
    let hook_invocation = matches!(
        cli.command.as_ref(),
        Some(Commands::Recall(args)) if args.hook
    );
    if let Err(error) = run(cli).await {
        if !hook_invocation
            && std::env::var_os("FLICKNOTE_DAEMON_MANAGED").is_some()
            && matches!(error, CliError::Json(_))
        {
            eprintln!("Permanent daemon startup failure: {error:#}");
            return;
        }
        eprintln!("Error: {error:#}");
        std::process::exit(1);
    }
}

async fn run(cli: Cli) -> Result<(), CliError> {
    if cli.command.is_none() {
        Cli::command()
            .print_help()
            .map_err(|error| CliError::Other(error.to_string()))?;
        return Ok(());
    }

    if let Some(Commands::Hook(args)) = cli.command.as_ref() {
        return commands::hook::run(args);
    }
    let config = Config::load()?;

    // Commands that don't need a database connection or session
    if let Some(ref cmd) = cli.command {
        match cmd {
            Commands::Login(args) => return commands::login::run(&config, args).await,
            Commands::Logout(args) => return commands::logout::run(&config, args).await,
            Commands::Daemon(args) => return commands::daemon::run(&config, args).await,
            Commands::Skill(args) => return commands::skill::run(args),
            Commands::Gateway(args) => return commands::gateway::run(&config, args).await,
            _ => {}
        }
    }

    if let Some(Commands::Recall(args)) = cli.command.as_ref() {
        return commands::recall::run(&config, args).await;
    }

    let daemon = DaemonClient::new(&config);
    daemon.health().await?;
    if matches!(cli.command, Some(Commands::Mcp)) {
        return mcp::serve(std::sync::Arc::new(config)).await;
    }
    dispatch(&cli, &daemon).await
}

async fn dispatch(cli: &Cli, daemon: &DaemonClient<'_>) -> Result<(), CliError> {
    let Some(ref command) = cli.command else {
        Cli::command()
            .print_help()
            .map_err(|e| CliError::Other(e.to_string()))?;
        return Ok(());
    };
    match command {
        Commands::Mcp => unreachable!("MCP is dispatched before regular CLI commands"),
        Commands::Add(args) => commands::add::run(daemon, args).await,
        Commands::Upload(args) => commands::upload::run(daemon, args).await,
        Commands::Append(args) => commands::append::run(daemon, args).await,
        Commands::Delete(args) => commands::delete::run(daemon, args).await,
        Commands::Edit(args) => commands::edit::run(daemon, args).await,
        Commands::Restore(args) => commands::restore::run(daemon, args).await,
        Commands::List(args) => commands::list::run(daemon, args).await,
        Commands::Count(args) => commands::count::run(daemon, args).await,
        Commands::Find(args) => commands::find::run(daemon, args).await,
        Commands::Recall(_) => unreachable!("recall is dispatched before daemon setup"),
        Commands::Topic(args) => commands::topic::run(daemon, args).await,
        Commands::Entity(args) => commands::entity::run(daemon, args).await,
        Commands::Gateway(_) => unreachable!("Gateway is dispatched before database setup"),
        Commands::Hook(_) => unreachable!("Hook is dispatched before configuration setup"),
        Commands::Source(args) => commands::source::run(daemon, args).await,
        Commands::Detail(args) => commands::detail::run(daemon, args).await,
        Commands::Content(args) => commands::content::run(daemon, args).await,
        Commands::Share(args) => commands::share::run_note(daemon, args).await,
        Commands::Unshare(args) => commands::share::run_unshare_note(daemon, args).await,
        Commands::Project(args) => commands::project::run(daemon, args).await,
        Commands::Modify(args) => commands::modify::run(daemon, args).await,
        Commands::Open(args) => commands::open::run(daemon, args).await,
        Commands::Import(args) => commands::import::run(daemon, args).await,
        // Login/Logout/Daemon/Skill are handled before dispatch() is called
        Commands::Login(_) | Commands::Logout(_) | Commands::Daemon(_) | Commands::Skill(_) => {
            unreachable!()
        }
    }
}

fn recall_hook_argv() -> bool {
    let mut saw_recall = false;
    for argument in std::env::args_os().skip(1) {
        if !saw_recall {
            saw_recall = argument == OsStr::new("recall");
            continue;
        }
        if argument == OsStr::new("--") {
            return false;
        }
        if argument == OsStr::new("--hook")
            || argument
                .to_str()
                .is_some_and(|value| value.starts_with("--hook="))
        {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod main_tests;
