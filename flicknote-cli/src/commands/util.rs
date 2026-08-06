use flicknote_core::backend::InsertedNote;
use flicknote_core::backend::NoteDb;
use flicknote_core::error::CliError;
use flicknote_core::services::dto::{NoteSummary, SectionDto};
use flicknote_core::types::Note;
use std::io::{IsTerminal, Read};

pub(crate) async fn resolve_note_id(db: &dyn NoteDb, prefix: &str) -> Result<String, CliError> {
    db.resolve_note_id(prefix).await
}

pub(crate) fn display_note_id(note: &Note) -> String {
    note.short_id
        .map(|id| id.to_string())
        .unwrap_or_else(|| note.id.clone())
}

pub(crate) fn display_inserted_note_id(note: &InsertedNote) -> String {
    note.short_id
        .map(|id| id.to_string())
        .unwrap_or_else(|| note.uuid.clone())
}

pub(crate) fn display_summary_id(note: &NoteSummary) -> String {
    note.short_id
        .map(|id| id.to_string())
        .unwrap_or_else(|| note.uuid.clone())
}

pub(crate) fn note_json(note: &Note, project_name: Option<&str>) -> serde_json::Value {
    serde_json::json!({
        "id": note.short_id,
        "uuid": note.id,
        "type": note.r#type,
        "status": note.status,
        "title": note.title,
        "project": project_name,
        "project_id": note.project_id,
        "summary": note.summary,
        "content": note.content,
        "is_flagged": note.is_flagged,
        "created_at": note.created_at,
        "updated_at": note.updated_at,
        "deleted_at": note.deleted_at,
    })
}

pub(crate) async fn note_summaries_json(
    db: &dyn NoteDb,
    notes: &[NoteSummary],
    archived: bool,
) -> Result<Vec<serde_json::Value>, CliError> {
    let mut values = Vec::with_capacity(notes.len());
    for summary in notes {
        let note = if archived {
            db.find_archived_note(&summary.uuid).await?
        } else {
            db.find_note(&summary.uuid).await?
        };
        values.push(note_json(&note, None));
    }
    Ok(values)
}

pub(crate) fn print_section_tree(sections: &[SectionDto]) {
    fn render(node: &SectionDto, prefix: &str, is_last: bool, output: &mut String) {
        let connector = if is_last { "└─ " } else { "├─ " };
        let marker = "#".repeat(node.level);
        let label = if node.level > 1 {
            format!("[{}] {} {}", node.id, marker, node.title)
        } else {
            format!("{} {}", marker, node.title)
        };
        output.push_str(&format!("{prefix}{connector}{label}\n"));
        let child_prefix = format!("{prefix}{}   ", if is_last { " " } else { "│" });
        for (index, child) in node.children.iter().enumerate() {
            render(
                child,
                &child_prefix,
                index + 1 == node.children.len(),
                output,
            );
        }
    }

    let mut output = String::new();
    for (index, section) in sections.iter().enumerate() {
        render(section, "", index + 1 == sections.len(), &mut output);
    }
    print!("{output}");
}

pub(crate) fn print_summaries_table(notes: &[NoteSummary]) {
    println!(
        "{:<8} {:<8} {:<30} {:<15} {:<20} {:<7} Created",
        "ID", "Type", "Title", "Project", "Topics", "Flagged"
    );
    println!("{}", "-".repeat(102));
    for note in notes {
        let date = note
            .created_at
            .as_deref()
            .and_then(|date| date.get(..10))
            .unwrap_or("-");
        let title = note.title.as_deref().unwrap_or("(untitled)");
        let title = if title.chars().count() > 28 {
            format!("{}…", title.chars().take(27).collect::<String>())
        } else {
            title.to_string()
        };
        let project = note.project.as_deref().unwrap_or("-");
        let project = if project.chars().count() > 13 {
            format!("{}…", project.chars().take(12).collect::<String>())
        } else {
            project.to_string()
        };
        let topics = note.topics.join(", ");
        let topics = if topics.chars().count() > 18 {
            format!("{}…", topics.chars().take(17).collect::<String>())
        } else {
            topics
        };
        println!(
            "{:<8} {:<8} {:<30} {:<15} {:<20} {:<7} {}",
            display_summary_id(note),
            note.note_type,
            title,
            project,
            topics,
            if note.flagged { "✓" } else { "" },
            date
        );
    }
}

/// Return the effective project name: arg wins, then $FLICKNOTE_PROJECT, then None.
pub(crate) fn resolve_project_arg(arg: &Option<String>) -> Option<String> {
    if arg.is_some() {
        return arg.clone();
    }
    std::env::var("FLICKNOTE_PROJECT")
        .ok()
        .filter(|s| !s.is_empty())
}

/// Read content from stdin. Errors if stdin is a terminal or if the input is empty.
pub(crate) fn read_stdin_required() -> Result<String, CliError> {
    if std::io::stdin().is_terminal() {
        return Err(CliError::Other(
            "No content provided. Pipe content from stdin.".into(),
        ));
    }
    let mut buf = String::new();
    std::io::stdin().read_to_string(&mut buf)?;
    let trimmed = buf.trim_end().to_string();
    if trimmed.is_empty() {
        return Err(CliError::Other("No content provided".into()));
    }
    Ok(trimmed)
}

/// Read optional stdin content. Returns `Ok(None)` when stdin is a terminal or empty.
pub(crate) fn try_read_stdin() -> Result<Option<String>, CliError> {
    if std::io::stdin().is_terminal() {
        return Ok(None);
    }
    let mut buf = String::new();
    std::io::stdin().read_to_string(&mut buf)?;
    Ok(classify_stdin_buf(&buf))
}

/// Classify a freshly-read stdin buffer. Pure helper, testable without a TTY.
pub(crate) fn classify_stdin_buf(buf: &str) -> Option<String> {
    let trimmed = buf.trim_end_matches(|c: char| c.is_ascii_whitespace());
    let trimmed = trimmed.trim_end_matches(' ');
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note_with_ids(id: &str, short_id: Option<i64>) -> Note {
        Note {
            id: id.to_string(),
            short_id,
            user_id: "test-user".to_string(),
            r#type: "normal".to_string(),
            status: "ai_queued".to_string(),
            title: None,
            content: None,
            summary: None,
            is_flagged: None,
            project_id: None,
            metadata: None,
            source: None,
            created_at: None,
            updated_at: None,
            deleted_at: None,
        }
    }

    #[test]
    fn test_display_note_id_prefers_short_id() {
        let note = note_with_ids("550e8400-e29b-41d4-a716-446655440000", Some(42));

        assert_eq!(display_note_id(&note), "42");
    }

    #[test]
    fn test_display_note_id_uses_full_uuid_without_short_id() {
        let note = note_with_ids("550e8400-e29b-41d4-a716-446655440000", None);

        assert_eq!(
            display_note_id(&note),
            "550e8400-e29b-41d4-a716-446655440000"
        );
    }
}
