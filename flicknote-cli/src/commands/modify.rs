use clap::Args;
use flicknote_core::backend::NoteDb;
use flicknote_core::config::Config;
use flicknote_core::error::CliError;
use flicknote_core::hooks;

use super::add::resolve_or_create_project;
use super::util::resolve_note_id;

#[derive(Args)]
pub(crate) struct ModifyArgs {
    /// Note ID (full UUID or prefix)
    id: String,
    /// Move note to this project (creates project if it doesn't exist)
    #[arg(short = 'p', long = "project")]
    project: Option<String>,
}

pub(crate) fn run(db: &dyn NoteDb, config: &Config, args: &ModifyArgs) -> Result<(), CliError> {
    let full_id = resolve_note_id(db, &args.id)?;
    let now = chrono::Utc::now().to_rfc3339();

    let old_note = db.find_note(&full_id)?;

    let Some(ref project_name) = args.project else {
        return Err(CliError::Other(
            "Nothing to modify. Use --project <name> to change the note's project.".into(),
        ));
    };

    let old_project_id = old_note.project_id.clone();
    let new_project_id = resolve_or_create_project(db, project_name)?;

    // No-op if already in same project
    if old_project_id.as_deref() == Some(new_project_id.as_str()) {
        println!(
            "Note {} is already in project \"{}\".",
            &full_id[..8],
            project_name
        );
        return Ok(());
    }

    // Run on-modify hook before writing
    let mut new_note = old_note.clone();
    new_note.project_id = Some(new_project_id.clone());
    new_note.updated_at = Some(now.clone());

    let old_json = serde_json::to_string(&old_note)?;
    let new_json = serde_json::to_string(&new_note)?;
    let config_dir = config.paths.config_dir.to_string_lossy();
    hooks::run_on_modify(
        &config.paths.hooks_dir,
        &old_json,
        &new_json,
        "modify",
        &config_dir,
    )?;

    // Atomic: update note project + delete old project if now empty
    let deleted_name =
        db.move_note_to_project(&full_id, &new_project_id, old_project_id.as_deref())?;

    println!(
        "Moved note {} to project \"{}\".",
        &full_id[..8],
        project_name
    );

    if let Some(name) = deleted_name {
        println!("Deleted empty project \"{}\".", name);
    }

    Ok(())
}
