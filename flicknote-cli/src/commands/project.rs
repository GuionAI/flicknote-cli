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

pub(crate) fn run(db: &dyn NoteDb, args: &ProjectArgs) -> Result<(), CliError> {
    match &args.command {
        ProjectCommands::List(a) => list(db, a),
    }
}

fn list(db: &dyn NoteDb, args: &ListArgs) -> Result<(), CliError> {
    // list_projects(archived=true) returns archived projects only.
    // For include_archived, we need to fetch both. As a simple approach,
    // fetch all via two queries and combine.
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
        println!("{:<10} {:<30} {:<10} Created", "ID", "Name", "Status");
        println!("{}", "-".repeat(62));
        for p in &projects {
            let id = &p.id[..8.min(p.id.len())];
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
            println!("{:<10} {:<30} {:<10} {}", id, p.name, status, date);
        }
    } else {
        println!("{:<10} {:<30} Created", "ID", "Name");
        println!("{}", "-".repeat(50));
        for p in &projects {
            let id = &p.id[..8.min(p.id.len())];
            let date = p
                .created_at
                .as_deref()
                .and_then(|d| d.get(..10))
                .unwrap_or("-");
            println!("{:<10} {:<30} {}", id, p.name, date);
        }
    }

    Ok(())
}
