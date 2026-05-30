use clap::Args;
use flicknote_core::backend::{InsertNoteReq, NoteDb};
use flicknote_core::config::Config;
use flicknote_core::error::CliError;
use std::io::Write;
use super::add::resolve_project;
use super::util::{resolve_note_id, resolve_project_arg, write_content};
#[derive(Args)]
pub(crate) struct EditArgs {
    /// Note ID (full UUID or prefix). Omit to create a new note.
    id: Option<String>,
    /// Assign to project by name (only for new notes)
    #[arg(long)]
    project: Option<String>,
}
/// Resolve the editor command from $EDITOR or $VISUAL.
fn resolve_editor() -> Result<String, CliError> {
    std::env::var("EDITOR")
        .or_else(|_| std::env::var("VISUAL"))
        .map_err(|_| {
            CliError::Other(
                "$EDITOR is not set. Set it with: export EDITOR=vim (or nano, code --wait, etc.)"
                    .into(),
            )
        })
}
/// Write content to a temp file, open $EDITOR, return the edited content.
///
/// Note: `split_whitespace` is used to parse the editor command, which handles
/// "code --wait" style editors but won't handle paths with spaces (e.g.
/// "/Applications/Sublime Text.app/..."). This matches git's behavior.
fn open_in_editor(initial_content: &str) -> Result<String, CliError> {
    let editor = resolve_editor()?;
    // Create temp file with .md extension for syntax highlighting
    let mut tmp = tempfile::Builder::new()
        .prefix("flicknote-")
        .suffix(".md")
        .tempfile()
        .map_err(|e| CliError::Other(format!("Failed to create temp file: {e}")))?;
    tmp.write_all(initial_content.as_bytes())
        .map_err(|e| CliError::Other(format!("Failed to write temp file: {e}")))?;
    tmp.flush()
        .map_err(|e| CliError::Other(format!("Failed to flush temp file: {e}")))?;
    tmp.as_file()
        .sync_all()
        .map_err(|e| CliError::Other(format!("Failed to sync temp file: {e}")))?;
    let path = tmp.path().to_path_buf();
    // Parse editor command — handle "code --wait" style editors
    let parts: Vec<&str> = editor.split_whitespace().collect();
    let (cmd, args) = parts
        .split_first()
        .ok_or_else(|| CliError::Other("$EDITOR is empty".into()))?;
    let status = std::process::Command::new(cmd)
        .args(args)
        .arg(&path)
        .status()
        .map_err(|e| CliError::Other(format!("Failed to launch editor '{editor}': {e}")))?;
    if !status.success() {
        let code = status
            .code()
            .map_or_else(|| "killed by signal".to_string(), |c| c.to_string());
        return Err(CliError::Other(format!("Editor exited with status {code}")));
    }
    let content = std::fs::read_to_string(&path)
        .map_err(|e| CliError::Other(format!("Failed to read temp file: {e}")))?;
    Ok(content.trim_end().to_string())
}
/// Edit an existing note.
async fn edit_existing(db: &dyn NoteDb, _config: &Config, id: &str) -> Result<(), CliError> {
    let full_id = resolve_note_id(db, id).await?;
    let note = db.find_note(&full_id).await?;
    let content = note.content.as_deref().unwrap_or("");
    // Build editable document with frontmatter
    let extractions = db
        .list_note_extractions(&[&full_id], &["topic", "entity"])
        .await?;
    let note_extractions = extractions.get(&full_id);
    let mut topics: Vec<String> = Vec::new();
    let mut entities: Vec<String> = Vec::new();
    if let Some(pairs) = note_extractions {
        for (ext_type, value) in pairs {
            match ext_type.as_str() {
                "topic" => topics.push(value.clone()),
                "entity" => entities.push(value.clone()),
                _ => {}
            }
        }
    }
    let (stored_frontmatter, body_without_fm) = crate::frontmatter::split_frontmatter(content);
    let display_content = crate::frontmatter::build_editable_content(
        note.title.as_deref(),
        body_without_fm,
        &topics,
        &entities,
        stored_frontmatter,
    );
    let edited = open_in_editor(&display_content)?;
    if edited == display_content.trim_end() {
        println!("No changes.");
        return Ok(());
    }
    if edited.is_empty() {
        return Err(CliError::Other(
            "Edited content is empty — aborting. Use `flicknote delete` to remove a note.".into(),
        ));
    }
    // Parse the editable document
    let doc = crate::frontmatter::parse_editable_doc(&edited);
    // Validate: full-note write requires a non-empty H1 title
    crate::frontmatter::validate_title_required(&doc).map_err(|e| {
        CliError::Other(e.message)
    })?;
    // Update title
    if let Some(ref new_title) = doc.title {
        let old_title = note.title.as_deref();
        if Some(new_title.as_str()) != old_title {
            db.update_note_title(&full_id, new_title).await?;
            println!("Updated title for note {}.", full_id);
        }
    }
    // Update extractions
    db.set_note_extractions(&full_id, "topic", &doc.topics).await?;
    db.set_note_extractions(&full_id, "entity", &doc.entities).await?;
    // Store body
    let stored_content = if let Some(ref fm) = doc.unmanaged_frontmatter {
        if doc.body.is_empty() {
            fm.clone()
        } else {
            format!("{}\n\n{}", fm, doc.body)
        }
    } else {
        doc.body.clone()
    };
    let old_content = note.content.as_deref().unwrap_or("");
    if stored_content != old_content {
        write_content(db, &full_id, &stored_content).await?;
        println!("Updated content for note {}.", full_id);
    }
    Ok(())
}
/// Create a new note from editor.
async fn create_from_editor(
    db: &dyn NoteDb,
    _config: &Config,
    project_arg: &Option<String>,
) -> Result<(), CliError> {
    let edited = open_in_editor("")?;
    if edited.is_empty() {
        println!("Empty buffer — no note created.");
        return Ok(());
    }
    let id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();
    // Parse editable document
    let doc = crate::frontmatter::parse_editable_doc(&edited);
    // Validate: new notes require a non-empty H1 title
    crate::frontmatter::validate_title_required(&doc).map_err(|e| {
        CliError::Other(e.message)
    })?;
    let effective_project = resolve_project_arg(project_arg);
    let project_id = if let Some(ref name) = effective_project {
        Some(resolve_project(db, name).await?)
    } else {
        None
    };
    let title_ref = doc.title.as_deref();
    let stored_content = if let Some(ref fm) = doc.unmanaged_frontmatter {
        if doc.body.is_empty() {
            fm.clone()
        } else {
            format!("{}\n\n{}", fm, doc.body)
        }
    } else {
        doc.body.clone()
    };
    let content_ref = if stored_content.is_empty() {
        None
    } else {
        Some(stored_content.as_str())
    };
    db.insert_note(&InsertNoteReq {
        id: &id,
        note_type: "normal",
        status: "ai_queued",
        title: title_ref,
        content: content_ref,
        metadata: None,
        project_id: project_id.as_deref(),
        now: &now,
    })
    .await?;
    // Insert extraction rows
    if !doc.topics.is_empty() {
        db.set_note_extractions(&id, "topic", &doc.topics).await?;
    }
    if !doc.entities.is_empty() {
        db.set_note_extractions(&id, "entity", &doc.entities).await?;
    }
    match effective_project.as_deref() {
        Some(name) => println!("Created note {} in project \"{name}\".", id),
        None => println!("Created note {}.", id),
    }
    Ok(())
}
pub(crate) async fn run(db: &dyn NoteDb, config: &Config, args: &EditArgs) -> Result<(), CliError> {
    if args.id.is_some() && args.project.is_some() {
        return Err(CliError::Other(
            "--project is only valid when creating a new note (omit the ID)".into(),
        ));
    }
    match &args.id {
        Some(id) => edit_existing(db, config, id).await,
        None => create_from_editor(db, config, &args.project).await,
    }
}
#[cfg(test)]
#[allow(unsafe_code)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    /// Guard for tests that mutate $EDITOR / $VISUAL env vars.
    static EDITOR_LOCK: Mutex<()> = Mutex::new(());
    #[test]
    fn test_resolve_editor_from_editor_var() {
        let _lock = EDITOR_LOCK.lock().unwrap();
        let orig_editor = std::env::var("EDITOR").ok();
        let orig_visual = std::env::var("VISUAL").ok();
        // SAFETY: test holds EDITOR_LOCK, preventing concurrent env mutation
        unsafe {
            std::env::set_var("EDITOR", "vim");
            std::env::remove_var("VISUAL");
        }
        let result = resolve_editor();
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "vim");
        // Restore
        // SAFETY: test holds EDITOR_LOCK, preventing concurrent env mutation
        unsafe {
            match orig_editor {
                Some(v) => std::env::set_var("EDITOR", v),
                None => std::env::remove_var("EDITOR"),
            }
            match orig_visual {
                Some(v) => std::env::set_var("VISUAL", v),
                None => std::env::remove_var("VISUAL"),
            }
        }
    }
    #[test]
    fn test_resolve_editor_falls_back_to_visual() {
        let _lock = EDITOR_LOCK.lock().unwrap();
        let orig_editor = std::env::var("EDITOR").ok();
        let orig_visual = std::env::var("VISUAL").ok();
        // SAFETY: test holds EDITOR_LOCK, preventing concurrent env mutation
        unsafe {
            std::env::remove_var("EDITOR");
            std::env::set_var("VISUAL", "nano");
        }
        let result = resolve_editor();
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "nano");
        // Restore
        // SAFETY: test holds EDITOR_LOCK, preventing concurrent env mutation
        unsafe {
            match orig_editor {
                Some(v) => std::env::set_var("EDITOR", v),
                None => std::env::remove_var("EDITOR"),
            }
            match orig_visual {
                Some(v) => std::env::set_var("VISUAL", v),
                None => std::env::remove_var("VISUAL"),
            }
        }
    }
    #[test]
    fn test_resolve_editor_errors_when_unset() {
        let _lock = EDITOR_LOCK.lock().unwrap();
        let orig_editor = std::env::var("EDITOR").ok();
        let orig_visual = std::env::var("VISUAL").ok();
        // SAFETY: test holds EDITOR_LOCK, preventing concurrent env mutation
        unsafe {
            std::env::remove_var("EDITOR");
            std::env::remove_var("VISUAL");
        }
        let result = resolve_editor();
        assert!(result.is_err());
        let msg = format!("{}", result.unwrap_err());
        assert!(
            msg.contains("$EDITOR"),
            "error should mention $EDITOR: {msg}"
        );
        // Restore
        // SAFETY: test holds EDITOR_LOCK, preventing concurrent env mutation
        unsafe {
            match orig_editor {
                Some(v) => std::env::set_var("EDITOR", v),
                None => std::env::remove_var("EDITOR"),
            }
            match orig_visual {
                Some(v) => std::env::set_var("VISUAL", v),
                None => std::env::remove_var("VISUAL"),
            }
        }
    }
}
