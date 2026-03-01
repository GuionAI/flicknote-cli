use clap::{Args, Subcommand};
use flicknote_core::db::Database;
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

pub(crate) fn run(db: &Database, args: &ProjectArgs) -> Result<(), CliError> {
    match &args.command {
        ProjectCommands::List(a) => list(db, a),
    }
}

fn list(db: &Database, args: &ListArgs) -> Result<(), CliError> {
    let projects = db.read(|conn| {
        let sql = if args.include_archived {
            "SELECT * FROM projects ORDER BY created_at DESC"
        } else {
            "SELECT * FROM projects WHERE is_archived = 0 OR is_archived IS NULL ORDER BY created_at DESC"
        };
        let mut stmt = conn.prepare(sql)?;
        let rows = stmt.query_map([], Project::from_row)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(CliError::from)
    })?;

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
