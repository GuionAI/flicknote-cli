use clap::Args;
use flicknote_core::backend::{InsertNoteReq, NoteDb};
use flicknote_core::config::Config;
use flicknote_core::error::CliError;

use super::add::{AddCreateMode, create_note_with_daemon, daemon_create_request, resolve_project};
use super::upload_util::{
    is_text_import_file, is_uploadable_file, metadata_for_upload, note_type_for_extension,
};
use super::util::{display_inserted_note_id, resolve_project_arg};

const UPLOAD_HELP: &str = include_str!("../help/upload.md");

#[derive(Args)]
#[command(after_help = UPLOAD_HELP)]
pub(crate) struct UploadArgs {
    /// File path to import or upload
    path: String,
    /// Assign to project by name
    #[arg(long)]
    project: Option<String>,
}

pub(crate) async fn run(
    db: &dyn NoteDb,
    config: &Config,
    args: &UploadArgs,
    mode: AddCreateMode,
) -> Result<(), CliError> {
    let is_text_import = is_text_import_file(&args.path);
    let is_attachment = is_uploadable_file(&args.path);
    if !(is_text_import || is_attachment) {
        return Err(CliError::Other(format!(
            "File not found or unsupported: {}",
            args.path
        )));
    }

    let id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();
    let file_path = std::path::PathBuf::from(&args.path);
    let effective_project = resolve_project_arg(&args.project);
    let project_id = if let Some(ref name) = effective_project {
        Some(resolve_project(db, name).await?)
    } else {
        None
    };

    let inserted = if is_text_import {
        let content = std::fs::read_to_string(&file_path).map_err(|e| {
            CliError::Other(format!("Failed to read {}: {}", file_path.display(), e))
        })?;
        let content = content.trim_end().to_string();
        if content.is_empty() {
            return Err(CliError::Other("No content provided".into()));
        }
        let (title, stripped_content) = crate::utils::extract_title_and_strip(&content);
        let title_ref = title.as_deref();
        let req = InsertNoteReq {
            id: &id,
            note_type: "normal",
            status: "ai_queued",
            title: title_ref,
            content: Some(&stripped_content),
            metadata: None,
            project_id: project_id.as_deref(),
            now: &now,
        };
        if mode.uses_daemon() {
            create_note_with_daemon(config, daemon_create_request(&req)).await?
        } else {
            db.insert_note(&req).await?
        }
    } else {
        if !mode.uses_daemon() {
            return Err(CliError::Other(
                "File uploads require the local sync daemon.".to_string(),
            ));
        }
        let filename = file_path
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| CliError::Other("Invalid filename".into()))?
            .to_string();
        let note_type = note_type_for_extension(&filename);
        let metadata = metadata_for_upload(&filename);
        let req = InsertNoteReq {
            id: &id,
            note_type,
            status: "source_queued",
            title: None,
            content: None,
            metadata: Some(&metadata),
            project_id: project_id.as_deref(),
            now: &now,
        };
        create_note_with_daemon(
            config,
            daemon_create_request(&req).with_attachment_path(file_path.to_string_lossy()),
        )
        .await?
    };

    match effective_project.as_deref() {
        Some(name) => println!(
            "Created note {} in project \"{name}\".",
            display_inserted_note_id(&inserted)
        ),
        None => println!("Created note {}.", display_inserted_note_id(&inserted)),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn daemon_request_for_upload_carries_attachment_path() {
        let metadata = metadata_for_upload("report.pdf");
        let req = daemon_create_request(&InsertNoteReq {
            id: "note-id",
            note_type: note_type_for_extension("report.pdf"),
            status: "source_queued",
            title: None,
            content: None,
            metadata: Some(&metadata),
            project_id: Some("project-id"),
            now: "2026-06-26T00:00:00Z",
        })
        .with_attachment_path("/tmp/report.pdf");

        assert_eq!(req.note_type, "file");
        assert_eq!(req.status, "source_queued");
        assert_eq!(req.content, None);
        assert_eq!(req.metadata.as_deref(), Some(metadata.as_str()));
        assert_eq!(req.project_id.as_deref(), Some("project-id"));
        assert_eq!(req.attachment_path.as_deref(), Some("/tmp/report.pdf"));
    }
}
