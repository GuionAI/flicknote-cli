use clap::{Args, Subcommand};
use flicknote_core::backend::NoteDb;
use flicknote_core::error::CliError;

const PROMPT_HELP: &str = include_str!("../help/prompt.md");

#[derive(Args)]
#[command(after_help = PROMPT_HELP)]
pub(crate) struct PromptArgs {
    #[command(subcommand)]
    command: PromptCommands,
}

#[derive(Subcommand)]
enum PromptCommands {
    /// Create a new prompt
    Add(AddPromptArgs),
    /// List all prompts
    List,
    /// Show prompt details
    Detail(DetailPromptArgs),
    /// Modify a prompt
    Modify(ModifyPromptArgs),
    /// Delete a prompt
    Delete(DeletePromptArgs),
}

#[derive(Args)]
struct AddPromptArgs {
    /// Prompt title
    #[arg(long)]
    title: String,
    /// Prompt text content
    #[arg(long)]
    prompt: String,
    /// Optional description
    #[arg(long)]
    description: Option<String>,
}

#[derive(Args)]
struct DetailPromptArgs {
    /// Prompt ID (full UUID)
    id: String,
}

#[derive(Args)]
struct ModifyPromptArgs {
    /// Prompt ID (full UUID)
    id: String,
    /// New title
    #[arg(long)]
    title: Option<String>,
    /// New prompt text
    #[arg(long)]
    prompt: Option<String>,
    /// New description
    #[arg(long)]
    description: Option<String>,
}

#[derive(Args)]
struct DeletePromptArgs {
    /// Prompt ID (full UUID)
    id: String,
}

pub(crate) async fn run(db: &dyn NoteDb, args: &PromptArgs) -> Result<(), CliError> {
    match &args.command {
        PromptCommands::Add(a) => add(db, a).await,
        PromptCommands::List => list(db).await,
        PromptCommands::Detail(a) => detail(db, a).await,
        PromptCommands::Modify(a) => modify(db, a).await,
        PromptCommands::Delete(a) => delete(db, a).await,
    }
}

async fn add(db: &dyn NoteDb, args: &AddPromptArgs) -> Result<(), CliError> {
    let id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();
    db.insert_prompt(
        &id,
        &args.title,
        args.description.as_deref(),
        &args.prompt,
        &now,
    )
    .await?;
    println!("Created prompt \"{}\" ({}).", args.title, id);
    Ok(())
}

async fn list(db: &dyn NoteDb) -> Result<(), CliError> {
    let prompts = db.list_prompts().await?;
    if prompts.is_empty() {
        println!("No prompts found.");
        return Ok(());
    }
    println!("{:<36} {:<30} Title", "ID", "Created");
    println!("{}", "-".repeat(76));
    for p in &prompts {
        let date = p
            .created_at
            .as_deref()
            .and_then(|d| d.get(..10))
            .unwrap_or("-");
        println!("{:<36} {:<30} {}", p.id, date, p.title);
    }
    Ok(())
}

async fn detail(db: &dyn NoteDb, args: &DetailPromptArgs) -> Result<(), CliError> {
    let full_id = db.resolve_prompt_id(&args.id).await?;
    let prompt = db.find_prompt(&full_id).await?;

    println!("ID:          {}", prompt.id);
    println!("Title:       {}", prompt.title);
    if let Some(ref desc) = prompt.description {
        println!("Description: {desc}");
    }
    println!(
        "Created:     {}",
        prompt
            .created_at
            .as_deref()
            .and_then(|d| d.get(..10))
            .unwrap_or("-")
    );
    println!("\nPrompt:\n{}", prompt.prompt);
    Ok(())
}

async fn modify(db: &dyn NoteDb, args: &ModifyPromptArgs) -> Result<(), CliError> {
    let full_id = db.resolve_prompt_id(&args.id).await?;

    if args.title.is_none() && args.prompt.is_none() && args.description.is_none() {
        return Err(CliError::Other(
            "Nothing to modify. Use --title, --prompt, or --description.".into(),
        ));
    }

    db.update_prompt(
        &full_id,
        args.title.as_deref(),
        args.description.as_deref(),
        args.prompt.as_deref(),
    )
    .await?;
    println!("Updated prompt {}.", full_id);
    Ok(())
}

async fn delete(db: &dyn NoteDb, args: &DeletePromptArgs) -> Result<(), CliError> {
    let full_id = db.resolve_prompt_id(&args.id).await?;
    db.delete_prompt(&full_id).await?;
    println!("Deleted prompt {}.", full_id);
    Ok(())
}
