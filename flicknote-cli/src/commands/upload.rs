use clap::Args;
use flicknote_core::backend::{InsertNoteReq, NoteDb};
use flicknote_core::config::Config;
use flicknote_core::error::CliError;
use flicknote_core::hooks;
use std::path::PathBuf;

use crate::commands::add::{build_hook_note, resolve_or_create_project};
use crate::commands::upload_util::{
    cleanup_uploaded_file, mime_from_extension, note_type_for_extension, upload_file_blocking,
};
use crate::commands::util::resolve_project_arg;

#[derive(Args)]
pub(crate) struct UploadArgs {
    /// Path to the file to upload
    file: PathBuf,
    /// Assign to project by name
    #[arg(long)]
    project: Option<String>,
}

pub(crate) fn run(db: &dyn NoteDb, config: &Config, args: &UploadArgs) -> Result<(), CliError> {
    if !args.file.exists() {
        return Err(CliError::Other(format!(
            "File not found: {}",
            args.file.display()
        )));
    }

    let filename = args
        .file
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| CliError::Other("Invalid filename".into()))?
        .to_string();

    let id = uuid::Uuid::new_v4().to_string();

    upload_file_blocking(config, &id, &args.file)?;

    let now = chrono::Utc::now().to_rfc3339();
    let metadata = serde_json::json!({
        "file": {
            "name": filename,
            "type": mime_from_extension(&filename)
        }
    })
    .to_string();

    let project_id = if let Some(ref name) = resolve_project_arg(&args.project) {
        Some(resolve_or_create_project(db, name)?)
    } else {
        None
    };

    let note_type = note_type_for_extension(&filename);

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

    let config_dir = config.paths.config_dir.to_string_lossy();
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

    println!("Created note {} with file {}", &id[..8], filename);
    Ok(())
}
