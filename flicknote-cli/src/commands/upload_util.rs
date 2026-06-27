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
        "csv" => "text/csv",
        _ => "application/octet-stream",
    }
}

pub(crate) fn note_type_for_extension(filename: &str) -> &'static str {
    let ext = extension_of(filename);
    match ext.as_str() {
        "ogg" | "mp3" | "wav" | "m4a" => "voice",
        "png" => "scan",
        _ => "file",
    }
}

pub(crate) fn metadata_for_upload(filename: &str) -> String {
    if note_type_for_extension(filename) == "voice" {
        return serde_json::json!({
            "voice": {
                "duration": 0
            }
        })
        .to_string();
    }

    serde_json::json!({
        "file": {
            "name": filename,
            "type": mime_from_extension(filename)
        }
    })
    .to_string()
}

const UPLOADABLE_EXTENSIONS: &[&str] = &[
    "png", "jpg", "jpeg", "gif", "webp", "svg", "pdf", "doc", "docx", "ppt", "pptx", "xls", "xlsx",
    "ogg", "mp3", "wav", "m4a", "mp4", "mov", "avi", "webm", "csv",
];

fn extension_of(filename: &str) -> String {
    std::path::Path::new(filename)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase()
}

fn file_has_extension(value: &str, allowed: &[&str]) -> bool {
    let path = std::path::Path::new(value);
    if !path.exists() || !path.is_file() {
        return false;
    }
    allowed.contains(&extension_of(value).as_str())
}

pub(crate) fn is_uploadable_file(value: &str) -> bool {
    file_has_extension(value, UPLOADABLE_EXTENSIONS)
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
        assert_eq!(mime_from_extension("data.csv"), "text/csv");
    }

    #[test]
    fn test_is_uploadable_csv_file() {
        let path = std::env::temp_dir().join("test_upload_util.csv");
        std::fs::write(&path, b"a,b,c").unwrap();
        let result = is_uploadable_file(path.to_str().unwrap());
        std::fs::remove_file(&path).unwrap();
        assert!(result, "should detect real csv file as uploadable");
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
    fn test_note_type_voice_for_audio() {
        assert_eq!(note_type_for_extension("song.mp3"), "voice");
        assert_eq!(note_type_for_extension("clip.wav"), "voice");
        assert_eq!(note_type_for_extension("voice.m4a"), "voice");
        assert_eq!(note_type_for_extension("track.ogg"), "voice");
    }

    #[test]
    fn test_note_type_file_for_others() {
        assert_eq!(note_type_for_extension("doc.pdf"), "file");
        assert_eq!(note_type_for_extension("slides.pptx"), "file");
        assert_eq!(note_type_for_extension("photo.jpg"), "file");
    }

    #[test]
    fn test_upload_metadata_voice_for_audio() {
        assert_eq!(
            metadata_for_upload("clip.wav"),
            serde_json::json!({ "voice": { "duration": 0 } }).to_string()
        );
    }

    #[test]
    fn test_upload_metadata_file_for_documents() {
        assert_eq!(
            metadata_for_upload("doc.pdf"),
            serde_json::json!({
                "file": {
                    "name": "doc.pdf",
                    "type": "application/pdf"
                }
            })
            .to_string()
        );
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

    #[test]
    fn test_png_not_readable_text() {
        let dir = tempfile::tempdir().unwrap();
        let png_path = dir.path().join("image.png");
        std::fs::write(&png_path, [0x89, 0x50, 0x4E, 0x47]).unwrap();

        let path_str = png_path.to_str().unwrap();
        assert!(is_uploadable_file(path_str));
    }
}
