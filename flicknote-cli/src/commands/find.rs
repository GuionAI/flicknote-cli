use clap::Args;
use flicknote_core::backend::{NoteDb, NoteFilter};
use flicknote_core::error::CliError;

use super::util::{print_notes_table, resolve_project_arg};

#[derive(Args)]
pub(crate) struct FindArgs {
    /// Keywords to search (OR match across title, content, summary)
    #[arg(required = true)]
    keywords: Vec<String>,
    /// Filter by project name
    #[arg(long)]
    project: Option<String>,
    /// Include archived notes
    #[arg(long)]
    archived: bool,
    /// Maximum number of results
    #[arg(long, default_value = "20")]
    limit: u32,
    /// Output as JSON
    #[arg(long)]
    json: bool,
}

pub(crate) fn run(db: &dyn NoteDb, args: &FindArgs) -> Result<(), CliError> {
    let effective_project = resolve_project_arg(&args.project);

    let project_id: Option<String> = if let Some(ref name) = effective_project {
        if args.project.is_none() {
            eprintln!("Filtering by project \"{name}\" from $FLICKNOTE_PROJECT.");
        }
        match db.find_project_by_name(name)? {
            Some(id) => Some(id),
            None => {
                return Err(CliError::Other(format!(
                    "no project found with name \"{name}\""
                )));
            }
        }
    } else {
        None
    };

    let notes = db.search_notes(
        &args.keywords,
        &NoteFilter {
            project_id: project_id.as_deref(),
            note_type: None,
            archived: args.archived,
            limit: args.limit,
        },
    )?;

    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&notes).map_err(CliError::Json)?
        );
    } else if notes.is_empty() {
        println!("No notes found matching: {}", args.keywords.join(", "));
    } else {
        print_notes_table(&notes);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    // build_find_sql logic is now internal to SqliteBackend / PgBackend.
    // The following tests verify the search behavior via the NoteDb trait
    // in backend::tests. No pure-SQL tests needed here.
}
