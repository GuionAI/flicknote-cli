use clap::Args;
use flicknote_core::backend::{InsertNoteReq, NoteDb};
use flicknote_core::config::Config;
use flicknote_core::error::CliError;
use std::io::{IsTerminal, Read};

use super::upload_util::{
    cleanup_uploaded_file, is_readable_text_file, is_uploadable_file, mime_from_extension,
    note_type_for_extension, upload_file_blocking,
};
use super::util::resolve_project_arg;

#[derive(Args)]
pub(crate) struct AddArgs {
    /// Note content or URL. Reads from stdin if omitted.
    value: Option<String>,
    /// Assign to project by name
    #[arg(long)]
    project: Option<String>,
}

pub(crate) fn run(db: &dyn NoteDb, config: &Config, args: &AddArgs) -> Result<(), CliError> {
    let id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();

    let mut content = match &args.value {
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

    let from_arg = args.value.is_some();
    let is_url_arg = content.starts_with("http://") || content.starts_with("https://");

    let path = std::path::Path::new(&content);
    let looks_like_file_path =
        from_arg && !is_url_arg && path.extension().is_some() && path.file_name().is_some();

    let is_text_file = from_arg && looks_like_file_path && is_readable_text_file(&content);

    if from_arg && looks_like_file_path && !is_uploadable_file(&content) && !is_text_file {
        return Err(CliError::Other(format!(
            "File not found or unsupported: {}",
            content
        )));
    }

    // Read text file content into `content` so the rest of the function treats it as normal text
    if is_text_file {
        let path = content.clone();
        content = std::fs::read_to_string(&path)
            .map_err(|e| CliError::Other(format!("Failed to read {}: {}", path, e)))?;
        content = content.trim_end().to_string();
    }

    let is_file = from_arg && !is_text_file && is_uploadable_file(&content);
    let is_url = !is_file && is_url_arg;

    let effective_project = resolve_project_arg(&args.project);
    let project_id = if let Some(ref name) = effective_project {
        Some(resolve_project(db, name)?)
    } else {
        None
    };

    if is_file {
        let file_path = std::path::PathBuf::from(&content);
        let filename = file_path
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| CliError::Other("Invalid filename".into()))?
            .to_string();

        upload_file_blocking(config, &id, &file_path)?;

        let note_type = note_type_for_extension(&filename);
        let metadata = serde_json::json!({
            "file": {
                "name": filename,
                "type": mime_from_extension(&filename)
            }
        })
        .to_string();

        if let Err(e) = db.insert_note(&InsertNoteReq {
            id: &id,
            note_type,
            status: "source_queued",
            title: None,
            content: None,
            metadata: Some(&metadata),
            project_id: project_id.as_deref(),
            now: &now,
        }) {
            #[allow(clippy::let_underscore_must_use, clippy::let_underscore_untyped)]
            let _ = cleanup_uploaded_file(config, &id);
            return Err(e);
        }
    } else if is_url {
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
        let (title, stripped_content) = crate::utils::extract_title_and_strip(&content);
        let title_ref = title.as_deref();
        db.insert_note(&InsertNoteReq {
            id: &id,
            note_type: "normal",
            status: "ai_queued",
            title: title_ref,
            content: Some(&stripped_content),
            metadata: None,
            project_id: project_id.as_deref(),
            now: &now,
        })?;
    }

    match effective_project.as_deref() {
        Some(name) => println!("Created note {} in project \"{name}\".", id),
        None => println!("Created note {}.", id),
    }
    Ok(())
}

/// Resolve project by name. Returns an error with a hint if the project doesn't exist.
pub(crate) fn resolve_project(db: &dyn NoteDb, name: &str) -> Result<String, CliError> {
    match db.find_project_by_name(name)? {
        Some(id) => Ok(id),
        None => Err(CliError::ProjectNotFound {
            name: name.to_string(),
        }),
    }
}
