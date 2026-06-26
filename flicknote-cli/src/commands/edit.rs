use super::add::resolve_project;
use super::add::{AddCreateMode, create_note_with_daemon, daemon_create_request_with_extractions};
use super::util::{
    display_inserted_note_id, display_note_id, print_pending_short_id_hint, resolve_note_id,
    resolve_project_arg,
};
use clap::Args;
use flicknote_core::backend::{InsertNoteReq, NoteDb};
use flicknote_core::config::Config;
use flicknote_core::error::CliError;
use std::io::Write;
#[derive(Args)]
pub(crate) struct EditArgs {
    /// Note ID. Use the numeric short ID shown in list/detail. Pending-sync notes may show a UUID prefix; full UUIDs are also accepted for compatibility. Omit to create a new note.
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
    let display_content = crate::editable_document::load_editable_note(db, &full_id).await?;
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
    let result = crate::editable_document::save_editable_note(db, &full_id, &edited).await?;
    if result.title_changed {
        let note = db.find_note(&full_id).await?;
        println!("Updated title for note {}.", display_note_id(&note));
    }
    if result.content_changed {
        let note = db.find_note(&full_id).await?;
        println!("Updated content for note {}.", display_note_id(&note));
    }
    Ok(())
}
/// Create a new note from editor.
async fn create_from_editor(
    db: &dyn NoteDb,
    config: &Config,
    project_arg: &Option<String>,
    mode: AddCreateMode,
) -> Result<(), CliError> {
    let edited = open_in_editor("")?;
    if edited.is_empty() {
        println!("Empty buffer — no note created.");
        return Ok(());
    }
    let id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();
    let parsed = crate::editable_document::parse_editable_note(&edited)?;
    let effective_project = resolve_project_arg(project_arg);
    let project_id = if let Some(ref name) = effective_project {
        Some(resolve_project(db, name).await?)
    } else {
        None
    };
    let inserted = if matches!(mode, AddCreateMode::DaemonForNonFile) {
        create_note_with_daemon(
            config,
            daemon_create_request_with_extractions(
                &InsertNoteReq {
                    id: &id,
                    note_type: "normal",
                    status: "ai_queued",
                    title: Some(parsed.title.as_str()),
                    content: crate::editable_document::normal_note_content_ref(&parsed),
                    metadata: None,
                    project_id: project_id.as_deref(),
                    now: &now,
                },
                &parsed.topics,
                &parsed.entities,
            ),
        )
        .await?
    } else {
        db.insert_note(&InsertNoteReq {
            id: &id,
            note_type: "normal",
            status: "ai_queued",
            title: Some(parsed.title.as_str()),
            content: crate::editable_document::normal_note_content_ref(&parsed),
            metadata: None,
            project_id: project_id.as_deref(),
            now: &now,
        })
        .await?
    };
    if matches!(mode, AddCreateMode::Local) && !parsed.topics.is_empty() {
        db.set_note_extractions(&id, "topic", &parsed.topics)
            .await?;
    }
    if matches!(mode, AddCreateMode::Local) && !parsed.entities.is_empty() {
        db.set_note_extractions(&id, "entity", &parsed.entities)
            .await?;
    }
    match effective_project.as_deref() {
        Some(name) => println!(
            "Created note {} in project \"{name}\".",
            display_inserted_note_id(&inserted)
        ),
        None => println!("Created note {}.", display_inserted_note_id(&inserted)),
    }
    if inserted.short_id.is_none() {
        print_pending_short_id_hint();
    }
    Ok(())
}
pub(crate) async fn run(
    db: &dyn NoteDb,
    config: &Config,
    args: &EditArgs,
    mode: AddCreateMode,
) -> Result<(), CliError> {
    if args.id.is_some() && args.project.is_some() {
        return Err(CliError::Other(
            "--project is only valid when creating a new note (omit the ID)".into(),
        ));
    }
    match &args.id {
        Some(id) => edit_existing(db, config, id).await,
        None => create_from_editor(db, config, &args.project, mode).await,
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
