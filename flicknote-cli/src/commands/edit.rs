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
fn edit_existing(db: &dyn NoteDb, _config: &Config, id: &str) -> Result<(), CliError> {
    let full_id = resolve_note_id(db, id)?;
    let note = db.find_note(&full_id)?;

    let content = note.content.as_deref().unwrap_or("");

    // Synthesize display content with title as H1 (same as content command).
    // When note has no title, just use raw content (no synthesized heading).
    let display_content = if let Some(ref t) = note.title {
        if content.is_empty() {
            format!("# {t}")
        } else {
            format!("# {t}\n\n{content}")
        }
    } else {
        content.to_string()
    };

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

    // Separate title from content
    let (new_title, new_body) = crate::utils::extract_title_and_strip(&edited);

    // Update title if it changed
    let old_title = note.title.as_deref();
    match (new_title.as_deref(), old_title) {
        (Some(t), old) if Some(t) != old => {
            db.update_note_title(&full_id, t)?;
            println!("Updated title for note {}.", full_id);
        }
        (None, Some(_)) => {
            return Err(CliError::Other(
                "Removing the title is not supported via edit — use `flicknote modify <id> --title \"\"` instead".into(),
            ));
        }
        _ => {}
    }

    // Update content if it changed
    let old_content = note.content.as_deref().unwrap_or("");
    if new_body != old_content {
        write_content(db, &full_id, &new_body)?;
        println!("Updated content for note {}.", full_id);
    }

    Ok(())
}

/// Create a new note from editor.
fn create_from_editor(
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

    let (title, stripped_content) = crate::utils::extract_title_and_strip(&edited);

    let effective_project = resolve_project_arg(project_arg);
    let project_id = if let Some(ref name) = effective_project {
        Some(resolve_project(db, name)?)
    } else {
        None
    };

    let title_ref = title.as_deref();
    db.insert_note(&InsertNoteReq {
        id: &id,
        note_type: "normal",
        status: "ai_queued",
        title: title_ref,
        content: Some(&stripped_content),
        metadata: None,
        project_id: project_id.as_deref(),
        now: &now,
    })?;

    match effective_project.as_deref() {
        Some(name) => println!("Created note {} in project \"{name}\".", id),
        None => println!("Created note {}.", id),
    }
    Ok(())
}

pub(crate) fn run(db: &dyn NoteDb, config: &Config, args: &EditArgs) -> Result<(), CliError> {
    if args.id.is_some() && args.project.is_some() {
        return Err(CliError::Other(
            "--project is only valid when creating a new note (omit the ID)".into(),
        ));
    }
    match &args.id {
        Some(id) => edit_existing(db, config, id),
        None => create_from_editor(db, config, &args.project),
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
