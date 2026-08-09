use clap::{Args, Subcommand};
use flicknote_core::error::CliError;
use flicknote_core::types::Keyterm;
use flicknote_sync::ipc::{AppRequest, DaemonClient};

const KEYTERM_HELP: &str = include_str!("../help/keyterm.md");

#[derive(Args)]
#[command(after_help = KEYTERM_HELP)]
pub(crate) struct KeytermArgs {
    #[command(subcommand)]
    command: KeytermCommands,
}

#[derive(Subcommand)]
enum KeytermCommands {
    /// Create a new keyterm set
    Add(AddKeytermArgs),
    /// List all keyterm sets
    List,
    /// Show keyterm set details
    Detail(DetailKeytermArgs),
    /// Modify a keyterm set
    Modify(ModifyKeytermArgs),
    /// Delete a keyterm set
    Delete(DeleteKeytermArgs),
}

#[derive(Args)]
struct AddKeytermArgs {
    /// Keyterm name
    #[arg(long)]
    name: String,
    /// Keyterm content
    #[arg(long)]
    content: Option<String>,
    /// Optional description
    #[arg(long)]
    description: Option<String>,
}

#[derive(Args)]
struct DetailKeytermArgs {
    /// Keyterm ID (full UUID)
    id: String,
}

#[derive(Args)]
struct ModifyKeytermArgs {
    /// Keyterm ID (full UUID)
    id: String,
    /// New name
    #[arg(long)]
    name: Option<String>,
    /// New content
    #[arg(long)]
    content: Option<String>,
    /// New description
    #[arg(long)]
    description: Option<String>,
}

#[derive(Args)]
struct DeleteKeytermArgs {
    /// Keyterm ID (full UUID)
    id: String,
}

pub(crate) async fn run(daemon: &DaemonClient<'_>, args: &KeytermArgs) -> Result<(), CliError> {
    match &args.command {
        KeytermCommands::Add(a) => add(daemon, a).await,
        KeytermCommands::List => list(daemon).await,
        KeytermCommands::Detail(a) => detail(daemon, a).await,
        KeytermCommands::Modify(a) => modify(daemon, a).await,
        KeytermCommands::Delete(a) => delete(daemon, a).await,
    }
}

async fn add(daemon: &DaemonClient<'_>, args: &AddKeytermArgs) -> Result<(), CliError> {
    let keyterm: Keyterm = daemon
        .call(AppRequest::KeytermAdd {
            name: args.name.clone(),
            description: args.description.clone(),
            content: args.content.clone(),
        })
        .await?;
    println!("Created keyterm \"{}\" ({}).", keyterm.name, keyterm.id);
    Ok(())
}

async fn list(daemon: &DaemonClient<'_>) -> Result<(), CliError> {
    let keyterms: Vec<Keyterm> = daemon.call(AppRequest::KeytermList).await?;
    if keyterms.is_empty() {
        println!("No keyterms found.");
        return Ok(());
    }
    println!("{:<36} {:<30} Name", "ID", "Updated");
    println!("{}", "-".repeat(76));
    for k in &keyterms {
        let date = k
            .updated_at
            .as_deref()
            .or(k.created_at.as_deref())
            .and_then(|d| d.get(..10))
            .unwrap_or("-");
        println!("{:<36} {:<30} {}", k.id, date, k.name);
    }
    Ok(())
}

async fn detail(daemon: &DaemonClient<'_>, args: &DetailKeytermArgs) -> Result<(), CliError> {
    let keyterm: Keyterm = daemon
        .call(AppRequest::KeytermGet {
            id: args.id.clone(),
        })
        .await?;

    println!("ID:          {}", keyterm.id);
    println!("Name:        {}", keyterm.name);
    if let Some(ref desc) = keyterm.description {
        println!("Description: {desc}");
    }
    println!(
        "Created:     {}",
        keyterm
            .created_at
            .as_deref()
            .and_then(|d| d.get(..10))
            .unwrap_or("-")
    );
    println!(
        "Updated:     {}",
        keyterm
            .updated_at
            .as_deref()
            .and_then(|d| d.get(..10))
            .unwrap_or("-")
    );
    if let Some(ref content) = keyterm.content {
        println!("\nContent:\n{content}");
    }
    Ok(())
}

async fn modify(daemon: &DaemonClient<'_>, args: &ModifyKeytermArgs) -> Result<(), CliError> {
    let keyterm: Keyterm = daemon
        .call(AppRequest::KeytermModify {
            id: args.id.clone(),
            name: args.name.clone(),
            description: args.description.clone(),
            content: args.content.clone(),
        })
        .await?;
    println!("Updated keyterm {}.", keyterm.id);
    Ok(())
}

async fn delete(daemon: &DaemonClient<'_>, args: &DeleteKeytermArgs) -> Result<(), CliError> {
    let id: String = daemon
        .call(AppRequest::KeytermDelete {
            id: args.id.clone(),
        })
        .await?;
    println!("Deleted keyterm {}.", id);
    Ok(())
}
