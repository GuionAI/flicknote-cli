use clap::Args;
use flicknote_core::backend::{InsertNoteReq, NoteDb};
use flicknote_core::config::Config;
use flicknote_core::error::CliError;
use std::io::{IsTerminal, Read};

use super::util::resolve_project_arg;

#[derive(Args)]
pub(crate) struct AddArgs {
    /// Note content or URL. Reads from stdin if omitted.
    value: Option<String>,
    /// Assign to project by name (creates project if it doesn't exist)
    #[arg(long)]
    project: Option<String>,
}

pub(crate) fn run(db: &dyn NoteDb, _config: &Config, args: &AddArgs) -> Result<(), CliError> {
    let id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();

    let content = match &args.value {
        Some(v) => v.to_owned(),
        None => {
            if std::io::stdin().is_terminal() {
                return Err(CliError::Other(
                    "No content provided. Pass a value or pipe from stdin.".into(),
                ));
            }
            let mut buf = String::new();
            std::io::stdin().read_to_string(&mut buf)?;
            let trimmed = buf.trim_end().to_string();
            if trimmed.is_empty() {
                return Err(CliError::Other("No content provided".into()));
            }
            trimmed
        }
    };

    let is_url = content.starts_with("http://") || content.starts_with("https://");

    let effective_project = resolve_project_arg(&args.project);
    let project_id = if let Some(ref name) = effective_project {
        Some(resolve_or_create_project(db, name)?)
    } else {
        None
    };

    if is_url {
        let metadata = serde_json::json!({ "link": { "url": &content } }).to_string();
        db.insert_note(&InsertNoteReq {
            id: &id,
            note_type: "link",
            status: "source_queued",
            title: None,
            content: None,
            metadata: Some(&metadata),
            project_id: project_id.as_deref(),
            now: &now,
        })?;
    } else {
        let title = crate::utils::extract_title(&content);
        let title_ref = title.as_deref();
        db.insert_note(&InsertNoteReq {
            id: &id,
            note_type: "normal",
            status: "ai_queued",
            title: title_ref,
            content: Some(&content),
            metadata: None,
            project_id: project_id.as_deref(),
            now: &now,
        })?;
    }

    match effective_project.as_deref() {
        Some(name) => println!("Created note {} in project \"{name}\".", &id[..8]),
        None => println!("Created note {}.", &id[..8]),
    }
    Ok(())
}

/// Resolve project by name, creating it if it doesn't exist.
pub(crate) fn resolve_or_create_project(db: &dyn NoteDb, name: &str) -> Result<String, CliError> {
    if let Some(id) = db.find_project_by_name(name)? {
        return Ok(id);
    }
    let id = db.create_project(name)?;
    println!("Created project \"{name}\".");
    Ok(id)
}
