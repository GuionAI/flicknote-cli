use flicknote_core::config::Config;
use flicknote_core::error::CliError;
use std::path::Path;

use crate::api_client::ApiClient;

pub(crate) fn upload_file_blocking(
    config: &Config,
    note_id: &str,
    file_path: &Path,
) -> Result<(), CliError> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    rt.block_on(async {
        let client = ApiClient::new(config).await?;
        let filename = file_path
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| CliError::Other("Invalid filename".into()))?;
        println!("Uploading {}...", filename);
        client.upload_file(note_id, file_path).await?;
        Ok::<(), CliError>(())
    })
}

pub(crate) fn cleanup_uploaded_file(config: &Config, note_id: &str) -> Result<(), CliError> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    rt.block_on(async {
        let client = ApiClient::new(config).await?;
        client.delete_attachment(note_id).await?;
        Ok::<(), CliError>(())
    })
}

pub(crate) fn mime_from_extension(filename: &str) -> &'static str {
    let ext = extension_of(filename);
    match ext.as_str() {
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
        _ => "application/octet-stream",
    }
}

pub(crate) fn note_type_for_extension(filename: &str) -> &'static str {
    let ext = extension_of(filename);
    match ext.as_str() {
        "png" => "scan",
        _ => "file",
    }
}

const UPLOADABLE_EXTENSIONS: &[&str] = &[
    "png", "jpg", "jpeg", "gif", "webp", "svg", "pdf", "doc", "docx", "ppt", "pptx", "xls", "xlsx",
    "ogg", "mp3", "wav", "m4a", "mp4", "mov", "avi", "webm",
];

fn extension_of(filename: &str) -> String {
    std::path::Path::new(filename)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase()
}

pub(crate) fn is_uploadable_file(value: &str) -> bool {
    let path = std::path::Path::new(value);
    if !path.exists() || !path.is_file() {
        return false;
    }
    let ext = extension_of(value);
    UPLOADABLE_EXTENSIONS.contains(&ext.as_str())
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

    #[test]
    fn test_note_type_scan_for_png() {
        assert_eq!(note_type_for_extension("photo.png"), "scan");
        assert_eq!(note_type_for_extension("photo.PNG"), "scan");
    }

    #[test]
    fn test_note_type_file_for_others() {
        assert_eq!(note_type_for_extension("doc.pdf"), "file");
        assert_eq!(note_type_for_extension("slides.pptx"), "file");
        assert_eq!(note_type_for_extension("song.mp3"), "file");
        assert_eq!(note_type_for_extension("photo.jpg"), "file");
    }

    #[test]
    fn test_is_uploadable_file_with_real_file() {
        let path = std::env::temp_dir().join("test_upload_util.png");
        std::fs::write(&path, b"fake").unwrap();
        let result = is_uploadable_file(path.to_str().unwrap());
        std::fs::remove_file(&path).unwrap();
        assert!(result, "should detect real png file as uploadable");
    }

    #[test]
    fn test_is_uploadable_file_nonexistent() {
        assert!(!is_uploadable_file("nonexistent_file_12345.png"));
        assert!(!is_uploadable_file(""));
        assert!(!is_uploadable_file("just some text"));
        assert!(!is_uploadable_file("https://example.com"));
    }
}
