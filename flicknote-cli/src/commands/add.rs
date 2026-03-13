use clap::Args;
use flicknote_core::backend::{InsertNoteReq, NoteDb};
use flicknote_core::config::Config;
use flicknote_core::error::CliError;
use flicknote_core::hooks;
use flicknote_core::types::Note;
use std::io::{IsTerminal, Read};

use super::upload_util::{
    cleanup_uploaded_file, is_uploadable_file, mime_from_extension, note_type_for_extension,
    upload_file_blocking,
};
use super::util::resolve_project_arg;

#[derive(Args)]
pub(crate) struct AddArgs {
    /// Note content or URL. Reads from stdin if omitted.
    value: Option<String>,
    /// Assign to project by name (creates project if it doesn't exist)
    #[arg(long)]
    project: Option<String>,
}

pub(crate) fn run(db: &dyn NoteDb, config: &Config, args: &AddArgs) -> Result<(), CliError> {
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

    let from_arg = args.value.is_some();
    let is_url_arg = content.starts_with("http://") || content.starts_with("https://");

    let path = std::path::Path::new(&content);
    let looks_like_file_path =
        from_arg && !is_url_arg && path.extension().is_some() && path.file_name().is_some();

    if from_arg && looks_like_file_path && !is_uploadable_file(&content) {
        return Err(CliError::Other(format!(
            "File not found or unsupported: {}",
            content
        )));
    }

    let is_file = from_arg && is_uploadable_file(&content);
    let is_url = !is_file && is_url_arg;

    let effective_project = resolve_project_arg(&args.project);
    let project_id = if let Some(ref name) = effective_project {
        Some(resolve_or_create_project(db, name)?)
    } else {
        None
    };

    let config_dir = config.paths.config_dir.to_string_lossy();

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

        let note_for_hook = build_hook_note(
            &id,
            db.user_id(),
            note_type,
            "source_queued",
            project_id.clone(),
            None,
            None,
            Some(metadata.clone()),
            &now,
        );
        let note_json = serde_json::to_string(&note_for_hook)?;
        hooks::run_on_add(&config.paths.hooks_dir, &note_json, &config_dir)?;
    } else if is_url {
        let metadata = serde_json::json!({ "link": { "url": &content } }).to_string();
        let note_for_hook = build_hook_note(
            &id,
            db.user_id(),
            "link",
            "source_queued",
            project_id.clone(),
            None,
            None,
            Some(metadata.clone()),
            &now,
        );
        let note_json = serde_json::to_string(&note_for_hook)?;
        hooks::run_on_add(&config.paths.hooks_dir, &note_json, &config_dir)?;
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
        let note_for_hook = build_hook_note(
            &id,
            db.user_id(),
            "normal",
            "ai_queued",
            project_id.clone(),
            title.clone(),
            Some(content.clone()),
            None,
            &now,
        );
        let note_json = serde_json::to_string(&note_for_hook)?;
        hooks::run_on_add(&config.paths.hooks_dir, &note_json, &config_dir)?;
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

/// Build a `Note` for the on-add hook payload.
#[allow(clippy::too_many_arguments)]
pub(crate) fn build_hook_note(
    id: &str,
    user_id: &str,
    note_type: &str,
    status: &str,
    project_id: Option<String>,
    title: Option<String>,
    content: Option<String>,
    metadata: Option<String>,
    now: &str,
) -> Note {
    Note {
        id: id.to_string(),
        user_id: user_id.to_string(),
        r#type: note_type.to_string(),
        status: status.to_string(),
        title,
        content,
        summary: None,
        is_flagged: None,
        project_id,
        metadata,
        source: None,
        external_id: None,
        created_at: Some(now.to_string()),
        updated_at: Some(now.to_string()),
        deleted_at: None,
    }
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
