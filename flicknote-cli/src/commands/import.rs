use std::path::{Path, PathBuf};

use clap::Args;
use flicknote_core::backend::{InsertNoteReq, NoteDb};
use flicknote_core::config::Config;
use flicknote_core::error::CliError;

use super::add::resolve_project;
use super::util::resolve_project_arg;

#[derive(Args)]
pub(crate) struct ImportArgs {
    /// Path to a .md file or directory of .md files
    path: PathBuf,
    /// Assign to project by name
    #[arg(long)]
    project: Option<String>,
}

pub(crate) fn run(db: &dyn NoteDb, _config: &Config, args: &ImportArgs) -> Result<(), CliError> {
    // Collect .md files
    let files = collect_md_files(&args.path)?;
    if files.is_empty() {
        println!("No .md files found at {:?}.", args.path);
        return Ok(());
    }

    // Resolve project if specified
    let effective_project = resolve_project_arg(&args.project);
    let project_id = if let Some(ref name) = effective_project {
        Some(resolve_project(db, name)?)
    } else {
        None
    };

    let mut imported = Vec::new();

    for file in &files {
        let content = std::fs::read_to_string(file)
            .map_err(|e| CliError::Other(format!("Failed to read {}: {}", file.display(), e)))?;

        if content.trim().is_empty() {
            continue;
        }

        let id = uuid::Uuid::new_v4().to_string();
        let title = crate::utils::extract_title(&content);
        let created_at = file_created_time(file);

        db.insert_note(&InsertNoteReq {
            id: &id,
            note_type: "normal",
            status: "ai_queued",
            title: title.as_deref(),
            content: Some(&content),
            metadata: None,
            project_id: project_id.as_deref(),
            now: &created_at,
        })?;

        imported.push((id, title, file.clone()));
    }

    for (id, title, file) in &imported {
        let filename = file.file_name().and_then(|s| s.to_str()).unwrap_or("?");
        let display_title = title.as_deref().unwrap_or("(untitled)");
        println!("Imported {} → {} — {}", filename, &id[..8], display_title);
    }
    match effective_project.as_deref() {
        Some(name) => {
            println!(
                "Imported {} note(s) into project \"{name}\".",
                imported.len()
            )
        }
        None => println!("Imported {} note(s).", imported.len()),
    }

    Ok(())
}

fn collect_md_files(path: &Path) -> Result<Vec<PathBuf>, CliError> {
    if path.is_file() {
        if path.extension().and_then(|e| e.to_str()) == Some("md") {
            return Ok(vec![path.to_path_buf()]);
        }
        return Err(CliError::Other(format!(
            "{} is not a .md file",
            path.display()
        )));
    }

    if path.is_dir() {
        let mut files = Vec::new();
        collect_md_recursive(path, &mut files)?;
        files.sort();
        return Ok(files);
    }

    Err(CliError::Other(format!(
        "{} does not exist",
        path.display()
    )))
}

fn file_created_time(path: &Path) -> String {
    let metadata = std::fs::metadata(path).ok();
    let system_time = metadata
        .as_ref()
        .and_then(|m| m.created().ok())
        .or_else(|| metadata.as_ref().and_then(|m| m.modified().ok()));

    match system_time {
        Some(t) => {
            let datetime: chrono::DateTime<chrono::Utc> = t.into();
            datetime.to_rfc3339()
        }
        None => {
            eprintln!(
                "Warning: could not read creation time for {}, using current time",
                path.display()
            );
            chrono::Utc::now().to_rfc3339()
        }
    }
}

fn collect_md_recursive(dir: &Path, files: &mut Vec<PathBuf>) -> Result<(), CliError> {
    let entries = std::fs::read_dir(dir)
        .map_err(|e| CliError::Other(format!("Failed to read dir {}: {}", dir.display(), e)))?;

    for entry in entries {
        let entry = entry.map_err(|e| CliError::Other(e.to_string()))?;
        let path = entry.path();
        if path.is_dir() {
            collect_md_recursive(&path, files)?;
        } else if path.extension().and_then(|e| e.to_str()) == Some("md") {
            files.push(path);
        }
    }
    Ok(())
}
