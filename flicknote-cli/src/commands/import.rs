use std::path::{Path, PathBuf};

use clap::Args;
use flicknote_core::config::Config;
use flicknote_core::db::Database;
use flicknote_core::error::CliError;
use flicknote_core::session;
use rusqlite::params;

#[derive(Args)]
pub(crate) struct ImportArgs {
    /// Path to a .md file or directory of .md files
    path: PathBuf,
    /// Assign to project by name (creates if it doesn't exist)
    #[arg(long)]
    project: Option<String>,
    /// Link to a taskwarrior task by UUID
    #[arg(long)]
    task: Option<String>,
}

pub(crate) fn run(db: &Database, config: &Config, args: &ImportArgs) -> Result<(), CliError> {
    let user_id = session::get_user_id(config)?;

    // Collect .md files
    let files = collect_md_files(&args.path)?;
    if files.is_empty() {
        println!("No .md files found at {:?}.", args.path);
        return Ok(());
    }

    // Resolve project if specified
    let project_id = if let Some(ref name) = args.project {
        Some(crate::commands::add::resolve_or_create_project(
            db, &user_id, name,
        )?)
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
        let title = extract_title(&content);
        let created_at = file_created_time(file);
        let now = chrono::Utc::now().to_rfc3339();

        db.write(|conn| {
            conn.execute(
                "INSERT INTO notes (id, user_id, type, status, title, content, project_id, created_at, updated_at)
                 VALUES (?, ?, 'normal', 'ai_queued', ?, ?, ?, ?, ?)",
                params![id, user_id, title, content, project_id, created_at, now],
            )?;

            // Link to task if --task provided
            if let Some(ref tw_uuid) = args.task {
                let link_id = uuid::Uuid::new_v4().to_string();
                let external_id = serde_json::json!({ "tw": tw_uuid }).to_string();
                let title = file.file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("Imported note")
                    .to_string();
                conn.execute(
                    "INSERT INTO note_tasks (id, note_id, user_id, title, external_id, created_at)
                     VALUES (?, ?, ?, ?, ?, ?)",
                    params![link_id, id, user_id, title, external_id, created_at],
                )?;
            }

            Ok(())
        })?;

        imported.push((id, title, file.clone()));
    }

    for (id, title, file) in &imported {
        let filename = file.file_name().and_then(|s| s.to_str()).unwrap_or("?");
        let display_title = title.as_deref().unwrap_or("(untitled)");
        println!("Imported {} → {} — {}", filename, &id[..8], display_title);
    }
    println!("Imported {} note(s).", imported.len());

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

/// Extract title from first markdown H1 heading (# Title).
/// Returns None if no heading found.
fn extract_title(content: &str) -> Option<String> {
    for line in content.lines() {
        let trimmed = line.trim();
        if let Some(title) = trimmed.strip_prefix("# ") {
            let title = title.trim();
            if !title.is_empty() {
                return Some(title.to_string());
            }
        }
    }
    None
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_title_normal_heading() {
        assert_eq!(
            extract_title("# My Title\nsome body"),
            Some("My Title".into())
        );
    }

    #[test]
    fn extract_title_no_heading() {
        assert_eq!(extract_title("just some text\nno heading here"), None);
    }

    #[test]
    fn extract_title_empty_h1() {
        assert_eq!(extract_title("# \nsome body"), None);
    }

    #[test]
    fn extract_title_h2_skipped() {
        assert_eq!(extract_title("## Not a title\n### Also not"), None);
    }

    #[test]
    fn extract_title_trims_whitespace() {
        assert_eq!(
            extract_title("#   Spaced Title   \nbody"),
            Some("Spaced Title".into())
        );
    }

    #[test]
    fn extract_title_first_h1_anywhere() {
        assert_eq!(
            extract_title("some preamble\n\n# First\n\n# Second"),
            Some("First".into()),
        );
    }
}
