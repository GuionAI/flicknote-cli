use clap::Args;
use flicknote_core::config::Config;
use flicknote_core::db::Database;
use flicknote_core::error::CliError;
use flicknote_core::session;
use rusqlite::params;
use std::path::PathBuf;

use crate::api_client::ApiClient;
use crate::commands::add::resolve_or_create_project;
use crate::commands::util::resolve_project_arg;

#[derive(Args)]
pub(crate) struct UploadArgs {
    /// Path to the file to upload
    file: PathBuf,
    /// Assign to project by name
    #[arg(long)]
    project: Option<String>,
}

pub(crate) fn run(db: &Database, config: &Config, args: &UploadArgs) -> Result<(), CliError> {
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

    let user_id = session::get_user_id(config)?;
    let id = uuid::Uuid::new_v4().to_string();

    // Upload file to R2 first — don't create note until upload succeeds.
    // Trade-off: if upload succeeds but DB insert fails, an orphaned file remains in R2.
    // This is preferred over the alternative (DB first) which would leave a dangling note record.
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    rt.block_on(async {
        let client = ApiClient::new(config).await?;
        println!("Uploading {}...", filename);
        client.upload_file(&id, &args.file).await?;
        Ok::<(), CliError>(())
    })?;

    // Upload succeeded — now create the note locally
    let now = chrono::Utc::now().to_rfc3339();
    let metadata = serde_json::json!({
        "file": {
            "name": filename,
            "type": mime_from_extension(&filename)
        }
    })
    .to_string();

    let project_id = if let Some(ref name) = resolve_project_arg(&args.project) {
        Some(resolve_or_create_project(db, &user_id, name)?)
    } else {
        None
    };

    db.write(|conn| {
        conn.execute(
            "INSERT INTO notes (id, user_id, type, status, metadata, project_id, created_at, updated_at)
             VALUES (?, ?, 'file', 'source_queued', ?, ?, ?, ?)",
            params![id, user_id, metadata, project_id, now, now],
        )?;

        Ok(())
    })?;

    println!("Created note {} with file {}", &id[..8], filename);
    Ok(())
}

fn mime_from_extension(filename: &str) -> &'static str {
    let ext = std::path::Path::new(filename)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    match ext.as_str() {
        // Images
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        // Audio
        "ogg" => "audio/ogg",
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        "m4a" => "audio/mp4",
        // Video
        "mp4" => "video/mp4",
        "mov" => "video/quicktime",
        "avi" => "video/x-msvideo",
        "webm" => "video/webm",
        // Documents
        "pdf" => "application/pdf",
        "doc" => "application/msword",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "ppt" => "application/vnd.ms-powerpoint",
        "pptx" => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        "xls" => "application/vnd.ms-excel",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_common_image_types() {
        assert_eq!(mime_from_extension("photo.jpg"), "image/jpeg");
        assert_eq!(mime_from_extension("photo.jpeg"), "image/jpeg");
        assert_eq!(mime_from_extension("image.png"), "image/png");
        assert_eq!(mime_from_extension("anim.gif"), "image/gif");
        assert_eq!(mime_from_extension("pic.webp"), "image/webp");
        assert_eq!(mime_from_extension("icon.svg"), "image/svg+xml");
    }

    #[test]
    fn test_audio_types() {
        assert_eq!(mime_from_extension("song.mp3"), "audio/mpeg");
        assert_eq!(mime_from_extension("clip.wav"), "audio/wav");
        assert_eq!(mime_from_extension("voice.m4a"), "audio/mp4");
        assert_eq!(mime_from_extension("track.ogg"), "audio/ogg");
    }

    #[test]
    fn test_video_types() {
        assert_eq!(mime_from_extension("movie.mp4"), "video/mp4");
        assert_eq!(mime_from_extension("clip.mov"), "video/quicktime");
        assert_eq!(mime_from_extension("old.avi"), "video/x-msvideo");
        assert_eq!(mime_from_extension("stream.webm"), "video/webm");
    }

    #[test]
    fn test_document_types() {
        assert_eq!(mime_from_extension("file.pdf"), "application/pdf");
        assert_eq!(
            mime_from_extension("doc.docx"),
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
        );
    }

    #[test]
    fn test_unknown_extension() {
        assert_eq!(mime_from_extension("file.xyz"), "application/octet-stream");
    }

    #[test]
    fn test_no_extension() {
        assert_eq!(mime_from_extension("README"), "application/octet-stream");
    }

    #[test]
    fn test_case_insensitive() {
        assert_eq!(mime_from_extension("photo.JPG"), "image/jpeg");
        assert_eq!(mime_from_extension("file.PDF"), "application/pdf");
    }

    #[test]
    fn test_dotfile() {
        assert_eq!(
            mime_from_extension(".gitignore"),
            "application/octet-stream"
        );
    }

    #[test]
    fn test_multiple_dots() {
        assert_eq!(
            mime_from_extension("archive.tar.gz"),
            "application/octet-stream"
        );
    }
}
