use std::path::Path;

use crate::error::CliError;

const ATTACHMENT_EXTENSIONS: &[&str] = &[
    "png", "jpg", "jpeg", "gif", "webp", "svg", "pdf", "doc", "docx", "ppt", "pptx", "xls", "xlsx",
    "ogg", "mp3", "wav", "m4a", "mp4", "mov", "avi", "webm", "csv",
];
const TEXT_EXTENSIONS: &[&str] = &["md", "markdown", "txt"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UploadKind {
    Text,
    Attachment {
        note_type: &'static str,
        metadata: String,
    },
}

pub fn classify(path: &Path) -> Result<UploadKind, CliError> {
    if !path.is_file() {
        return Err(CliError::Other(format!(
            "File not found or unsupported: {}",
            path.display()
        )));
    }
    let extension = extension_of(path);
    if TEXT_EXTENSIONS.contains(&extension.as_str()) {
        return Ok(UploadKind::Text);
    }
    if !ATTACHMENT_EXTENSIONS.contains(&extension.as_str()) {
        return Err(CliError::Other(format!(
            "File not found or unsupported: {}",
            path.display()
        )));
    }
    let filename = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| CliError::Other("Invalid filename".to_string()))?;
    let note_type = note_type_for_extension(filename);
    Ok(UploadKind::Attachment {
        note_type,
        metadata: metadata_for_upload(filename),
    })
}

pub fn note_type_for_extension(filename: &str) -> &'static str {
    match extension_of(Path::new(filename)).as_str() {
        "ogg" | "mp3" | "wav" | "m4a" => "meeting",
        "png" => "scan",
        _ => "file",
    }
}

pub fn metadata_for_upload(filename: &str) -> String {
    if note_type_for_extension(filename) == "meeting" {
        return serde_json::json!({ "meeting": { "duration": 0 } }).to_string();
    }
    serde_json::json!({
        "file": {
            "name": filename,
            "type": mime_from_extension(filename),
        }
    })
    .to_string()
}

pub fn mime_from_extension(filename: &str) -> &'static str {
    match extension_of(Path::new(filename)).as_str() {
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "ogg" => "audio/ogg",
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        "m4a" => "audio/mp4",
        "mp4" => "video/mp4",
        "mov" => "video/quicktime",
        "avi" => "video/x-msvideo",
        "webm" => "video/webm",
        "pdf" => "application/pdf",
        "doc" => "application/msword",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "ppt" => "application/vnd.ms-powerpoint",
        "pptx" => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        "xls" => "application/vnd.ms-excel",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "csv" => "text/csv",
        _ => "application/octet-stream",
    }
}

fn extension_of(path: &Path) -> String {
    path.extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or("")
        .to_lowercase()
}
