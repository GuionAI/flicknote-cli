use clap::{Args, Subcommand};
use flicknote_core::error::CliError;
use flicknote_core::services::dto::{Patch, ProjectAddInput, ProjectDto, ProjectModifyInput};
use flicknote_core::types::{Keyterm, Project};
use flicknote_sync::ipc::{AppRequest, DaemonClient};

const PROJECT_HELP: &str = include_str!("../help/project.md");

#[derive(Args)]
#[command(after_help = PROJECT_HELP)]
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
    /// Get or create a share link for a project
    Share(ShareProjectArgs),
    /// Revoke the share link for a project
    Unshare(ShareProjectArgs),
    /// Modify project metadata
    Modify(ModifyProjectArgs),
    /// Delete (archive) a project
    Delete(DeleteProjectArgs),
}

#[derive(Args)]
struct AddProjectArgs {
    /// Project name
    name: String,
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
    /// Project ID (full UUID)
    id: String,
}

#[derive(Args)]
struct ShareProjectArgs {
    /// Project ID (full UUID)
    id: String,
}

#[derive(Args)]
struct ModifyProjectArgs {
    /// Project ID (full UUID)
    id: String,
    /// Associate a keyterm set by ID (use "none" to clear)
    #[arg(long)]
    keyterm: Option<String>,
    /// Color hex code (use "none" to clear)
    #[arg(long)]
    color: Option<String>,
}

#[derive(Args)]
struct DeleteProjectArgs {
    /// Project ID (full UUID)
    id: String,
}

pub(crate) async fn run(daemon: &DaemonClient<'_>, args: &ProjectArgs) -> Result<(), CliError> {
    match &args.command {
        ProjectCommands::List(a) => list(daemon, a).await,
        ProjectCommands::Add(a) => add(daemon, a).await,
        ProjectCommands::Detail(a) => detail(daemon, a).await,
        ProjectCommands::Share(a) => super::share::run_project(daemon, &a.id).await,
        ProjectCommands::Unshare(a) => super::share::run_unshare_project(daemon, &a.id).await,
        ProjectCommands::Modify(a) => modify(daemon, a).await,
        ProjectCommands::Delete(a) => delete(daemon, a).await,
    }
}

async fn add(daemon: &DaemonClient<'_>, args: &AddProjectArgs) -> Result<(), CliError> {
    let project: ProjectDto = daemon
        .call(AppRequest::ProjectAdd(ProjectAddInput {
            name: args.name.clone(),
            keyterm: args.keyterm.clone(),
            color: args.color.clone(),
        }))
        .await?;
    println!("Created project \"{}\" ({}).", project.name, project.id);
    Ok(())
}

async fn list(daemon: &DaemonClient<'_>, args: &ListArgs) -> Result<(), CliError> {
    let projects: Vec<ProjectDto> = daemon
        .call(AppRequest::ProjectList {
            include_archived: args.include_archived,
        })
        .await?;

    if args.json {
        let values: Vec<Project> = daemon
            .call(AppRequest::ProjectRecords {
                include_archived: args.include_archived,
            })
            .await?;
        println!(
            "{}",
            serde_json::to_string_pretty(&values).map_err(CliError::Json)?
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
            let status = if p.archived { "archived" } else { "active" };
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

async fn detail(daemon: &DaemonClient<'_>, args: &DetailArgs) -> Result<(), CliError> {
    let project: ProjectDto = daemon
        .call(AppRequest::ProjectGet {
            id: args.id.clone(),
        })
        .await?;

    println!("ID:      {}", project.id);
    println!("Name:    {}", project.name);
    if let Some(ref color) = project.color {
        println!("Color:   {color}");
    }
    if let Some(ref keyterm_id) = project.keyterm_id {
        match daemon
            .call::<Keyterm>(AppRequest::KeytermGet {
                id: keyterm_id.clone(),
            })
            .await
        {
            Ok(keyterm) => println!("Keyterm: {} ({keyterm_id})", keyterm.name),
            Err(error) => {
                eprintln!("warning: could not look up keyterm {keyterm_id} ({error})")
            }
        }
    }
    let status = if project.archived {
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

async fn modify(daemon: &DaemonClient<'_>, args: &ModifyProjectArgs) -> Result<(), CliError> {
    let patch = |value: &Option<String>| match value.as_deref() {
        None => Patch::Missing,
        Some("none") => Patch::Null,
        Some(value) => Patch::Value(value.to_string()),
    };
    let project: ProjectDto = daemon
        .call(AppRequest::ProjectModify(ProjectModifyInput {
            id: args.id.clone(),
            keyterm: patch(&args.keyterm),
            color: patch(&args.color),
        }))
        .await?;
    println!("Updated project {}.", project.id);
    Ok(())
}

async fn delete(daemon: &DaemonClient<'_>, args: &DeleteProjectArgs) -> Result<(), CliError> {
    let project: ProjectDto = daemon
        .call(AppRequest::ProjectArchive {
            id: args.id.clone(),
        })
        .await?;
    println!("Deleted project {}.", project.id);
    Ok(())
}
