use clap::{Args, Subcommand};
use flicknote_core::backend::NoteDb;
use flicknote_core::error::CliError;
use flicknote_core::types::Project;

#[derive(Args)]
pub(crate) struct ProjectArgs {
    #[command(subcommand)]
    command: ProjectCommands,
}

#[derive(Subcommand)]
enum ProjectCommands {
    /// List projects
    List(ListArgs),
    /// Create a new project
    Add(AddProjectArgs),
    /// Show project details
    Detail(DetailArgs),
    /// Modify project metadata
    Modify(ModifyProjectArgs),
    /// Delete (archive) a project
    Delete(DeleteProjectArgs),
}

#[derive(Args)]
struct AddProjectArgs {
    /// Project name
    name: String,
    /// Associate a prompt by ID
    #[arg(long)]
    prompt: Option<String>,
    /// Associate a keyterm set by ID
    #[arg(long)]
    keyterm: Option<String>,
    /// Color hex code (e.g. #FF5733)
    #[arg(long)]
    color: Option<String>,
}

#[derive(Args)]
struct ListArgs {
    /// Include archived projects
    #[arg(long)]
    include_archived: bool,
    /// Output as JSON
    #[arg(long)]
    json: bool,
}

#[derive(Args)]
struct DetailArgs {
    /// Project ID (full UUID or prefix)
    id: String,
}

#[derive(Args)]
struct ModifyProjectArgs {
    /// Project ID (full UUID or prefix)
    id: String,
    /// Associate a prompt by ID (use "none" to clear)
    #[arg(long)]
    prompt: Option<String>,
    /// Associate a keyterm set by ID (use "none" to clear)
    #[arg(long)]
    keyterm: Option<String>,
    /// Color hex code (use "none" to clear)
    #[arg(long)]
    color: Option<String>,
}

#[derive(Args)]
struct DeleteProjectArgs {
    /// Project ID (full UUID or prefix)
    id: String,
}

pub(crate) fn run(db: &dyn NoteDb, args: &ProjectArgs) -> Result<(), CliError> {
    match &args.command {
        ProjectCommands::List(a) => list(db, a),
        ProjectCommands::Add(a) => add(db, a),
        ProjectCommands::Detail(a) => detail(db, a),
        ProjectCommands::Modify(a) => modify(db, a),
        ProjectCommands::Delete(a) => delete(db, a),
    }
}

fn add(db: &dyn NoteDb, args: &AddProjectArgs) -> Result<(), CliError> {
    if db.find_project_by_name(&args.name)?.is_some() {
        return Err(CliError::ProjectAlreadyExists {
            name: args.name.clone(),
        });
    }
    let id = db.create_project(&args.name)?;

    // Resolve and validate FK IDs before storing.
    let resolved_prompt = args
        .prompt
        .as_deref()
        .map(|v| db.resolve_prompt_id(v))
        .transpose()?;
    let resolved_keyterm = args
        .keyterm
        .as_deref()
        .map(|v| db.resolve_keyterm_id(v))
        .transpose()?;
    let color_opt = args.color.as_deref().map(Some);

    let prompt_id_opt = resolved_prompt.as_deref().map(Some);
    let keyterm_id_opt = resolved_keyterm.as_deref().map(Some);

    if prompt_id_opt.is_some() || keyterm_id_opt.is_some() || color_opt.is_some() {
        db.update_project(&id, prompt_id_opt, keyterm_id_opt, color_opt)?;
    }

    println!("Created project \"{}\" ({}).", args.name, id);
    Ok(())
}

fn list(db: &dyn NoteDb, args: &ListArgs) -> Result<(), CliError> {
    let projects: Vec<Project> = if args.include_archived {
        let mut all = db.list_projects(false)?;
        all.extend(db.list_projects(true)?);
        all.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        all
    } else {
        db.list_projects(false)?
    };

    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&projects).map_err(CliError::Json)?
        );
    } else if args.include_archived {
        println!("{:<36} {:<30} {:<10} Created", "ID", "Name", "Status");
        println!("{}", "-".repeat(88));
        for p in &projects {
            let date = p
                .created_at
                .as_deref()
                .and_then(|d| d.get(..10))
                .unwrap_or("-");
            let status = if p.is_archived.unwrap_or(0) != 0 {
                "archived"
            } else {
                "active"
            };
            println!("{:<36} {:<30} {:<10} {}", p.id, p.name, status, date);
        }
    } else {
        println!("{:<36} {:<30} Created", "ID", "Name");
        println!("{}", "-".repeat(76));
        for p in &projects {
            let date = p
                .created_at
                .as_deref()
                .and_then(|d| d.get(..10))
                .unwrap_or("-");
            println!("{:<36} {:<30} {}", p.id, p.name, date);
        }
    }

    Ok(())
}

fn detail(db: &dyn NoteDb, args: &DetailArgs) -> Result<(), CliError> {
    let full_id = db.resolve_project_id(&args.id)?;
    let project = db.find_project(&full_id)?;

    println!("ID:      {}", project.id);
    println!("Name:    {}", project.name);
    if let Some(ref color) = project.color {
        println!("Color:   {color}");
    }
    if let Some(ref pid) = project.prompt_id {
        match db.find_prompt(pid) {
            Ok(prompt) => println!("Prompt:  {} ({})", prompt.title, pid),
            Err(e) => eprintln!("warning: could not look up prompt {pid} ({e})"),
        }
    }
    if let Some(ref kid) = project.keyterm_id {
        match db.find_keyterm(kid) {
            Ok(keyterm) => println!("Keyterm: {} ({})", keyterm.name, kid),
            Err(e) => eprintln!("warning: could not look up keyterm {kid} ({e})"),
        }
    }
    let status = if project.is_archived.unwrap_or(0) != 0 {
        "archived"
    } else {
        "active"
    };
    println!("Status:  {status}");
    println!(
        "Created: {}",
        project
            .created_at
            .as_deref()
            .and_then(|d| d.get(..10))
            .unwrap_or("-")
    );

    Ok(())
}

fn parse_clearable(val: &Option<String>) -> Option<Option<&str>> {
    val.as_deref()
        .map(|v| if v == "none" { None } else { Some(v) })
}

fn modify(db: &dyn NoteDb, args: &ModifyProjectArgs) -> Result<(), CliError> {
    let full_id = db.resolve_project_id(&args.id)?;

    if args.prompt.is_none() && args.keyterm.is_none() && args.color.is_none() {
        return Err(CliError::Other(
            "Nothing to modify. Use --prompt, --keyterm, or --color.".into(),
        ));
    }

    // Resolve FK IDs: "none" clears the field, any other value is resolved.
    let resolved_prompt: Option<Option<String>> = args
        .prompt
        .as_deref()
        .map(|v| {
            if v == "none" {
                Ok(None)
            } else {
                db.resolve_prompt_id(v).map(Some)
            }
        })
        .transpose()?;
    let resolved_keyterm: Option<Option<String>> = args
        .keyterm
        .as_deref()
        .map(|v| {
            if v == "none" {
                Ok(None)
            } else {
                db.resolve_keyterm_id(v).map(Some)
            }
        })
        .transpose()?;
    let color = parse_clearable(&args.color);

    let prompt_id = resolved_prompt.as_ref().map(|opt| opt.as_deref());
    let keyterm_id = resolved_keyterm.as_ref().map(|opt| opt.as_deref());

    db.update_project(&full_id, prompt_id, keyterm_id, color)?;
    println!("Updated project {}.", full_id);
    Ok(())
}

fn delete(db: &dyn NoteDb, args: &DeleteProjectArgs) -> Result<(), CliError> {
    let full_id = db.resolve_project_id(&args.id)?;
    db.delete_project(&full_id)?;
    println!("Deleted project {}.", full_id);
    Ok(())
}
