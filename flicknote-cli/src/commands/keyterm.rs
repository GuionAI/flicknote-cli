use clap::{Args, Subcommand};
use flicknote_core::backend::NoteDb;
use flicknote_core::error::CliError;

#[derive(Args)]
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
    /// Keyterm ID (full UUID or prefix)
    id: String,
}

#[derive(Args)]
struct ModifyKeytermArgs {
    /// Keyterm ID (full UUID or prefix)
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
    /// Keyterm ID (full UUID or prefix)
    id: String,
}

pub(crate) fn run(db: &dyn NoteDb, args: &KeytermArgs) -> Result<(), CliError> {
    match &args.command {
        KeytermCommands::Add(a) => add(db, a),
        KeytermCommands::List => list(db),
        KeytermCommands::Detail(a) => detail(db, a),
        KeytermCommands::Modify(a) => modify(db, a),
        KeytermCommands::Delete(a) => delete(db, a),
    }
}

fn add(db: &dyn NoteDb, args: &AddKeytermArgs) -> Result<(), CliError> {
    let id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();
    db.insert_keyterm(
        &id,
        &args.name,
        args.description.as_deref(),
        args.content.as_deref(),
        &now,
    )?;
    println!("Created keyterm \"{}\" ({}).", args.name, id);
    Ok(())
}

fn list(db: &dyn NoteDb) -> Result<(), CliError> {
    let keyterms = db.list_keyterms()?;
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

fn detail(db: &dyn NoteDb, args: &DetailKeytermArgs) -> Result<(), CliError> {
    let full_id = db.resolve_keyterm_id(&args.id)?;
    let keyterm = db.find_keyterm(&full_id)?;

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

fn modify(db: &dyn NoteDb, args: &ModifyKeytermArgs) -> Result<(), CliError> {
    let full_id = db.resolve_keyterm_id(&args.id)?;

    if args.name.is_none() && args.content.is_none() && args.description.is_none() {
        return Err(CliError::Other(
            "Nothing to modify. Use --name, --content, or --description.".into(),
        ));
    }

    db.update_keyterm(
        &full_id,
        args.name.as_deref(),
        args.description.as_deref(),
        args.content.as_deref(),
    )?;
    println!("Updated keyterm {}.", full_id);
    Ok(())
}

fn delete(db: &dyn NoteDb, args: &DeleteKeytermArgs) -> Result<(), CliError> {
    let full_id = db.resolve_keyterm_id(&args.id)?;
    db.delete_keyterm(&full_id)?;
    println!("Deleted keyterm {}.", full_id);
    Ok(())
}
