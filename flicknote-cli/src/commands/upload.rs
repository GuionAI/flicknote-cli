use clap::Args;
use flicknote_core::backend::{InsertNoteReq, NoteDb};
use flicknote_core::config::Config;
use flicknote_core::error::CliError;
use std::path::PathBuf;

use crate::commands::add::resolve_project;
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
        Some(resolve_project(db, name)?)
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

    println!("Created note {} with file {}", id, filename);
    Ok(())
}
