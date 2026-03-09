use clap::Args;
use flicknote_core::db::Database;
use flicknote_core::error::CliError;
use flicknote_core::types::Note;
use rusqlite::params;

use super::util::{print_notes_table, resolve_project_arg};

#[derive(Args)]
pub(crate) struct FindArgs {
    /// Keywords to search (OR match across title, content, summary)
    #[arg(required = true)]
    keywords: Vec<String>,
    /// Filter by project name
    #[arg(long)]
    project: Option<String>,
    /// Include archived notes
    #[arg(long)]
    archived: bool,
    /// Maximum number of results
    #[arg(long, default_value = "20")]
    limit: u32,
    /// Output as JSON
    #[arg(long)]
    json: bool,
}

/// Build the find SQL query. Extracted for unit-testability without a real DB.
/// Each keyword produces one `(title LIKE ? OR content LIKE ? OR summary LIKE ?)` block.
/// Multiple keywords are joined with OR. Project filter appended if `has_project`.
///
/// # Panics
/// Panics if `keywords` is empty — callers must ensure at least one keyword.
pub(crate) fn build_find_sql(keywords: &[String], archived: bool, has_project: bool) -> String {
    assert!(
        !keywords.is_empty(),
        "build_find_sql requires at least one keyword"
    );

    let archive_cond = if archived {
        "deleted_at IS NOT NULL"
    } else {
        "deleted_at IS NULL"
    };

    let keyword_blocks: Vec<String> = keywords
        .iter()
        .map(|_| "(title LIKE ? OR content LIKE ? OR summary LIKE ?)".to_string())
        .collect();
    let keywords_clause = keyword_blocks.join(" OR ");

    let mut sql = format!("SELECT * FROM notes WHERE {archive_cond} AND ({keywords_clause})");

    if has_project {
        sql.push_str(" AND project_id = ?");
    }

    sql.push_str(" ORDER BY updated_at DESC LIMIT ?");
    sql
}

pub(crate) fn run(db: &Database, args: &FindArgs) -> Result<(), CliError> {
    let effective_project = resolve_project_arg(&args.project);

    let project_id: Option<String> = if let Some(ref name) = effective_project {
        if args.project.is_none() {
            eprintln!("Filtering by project \"{name}\" from $FLICKNOTE_PROJECT.");
        }
        db.read(|conn| {
            match conn
                .prepare(
                    "SELECT id FROM projects WHERE name = ? AND (is_archived = 0 OR is_archived IS NULL) LIMIT 1",
                )?
                .query_row(params![name], |row| row.get::<_, String>(0))
            {
                Ok(id) => Ok(Some(id)),
                Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
                Err(e) => Err(CliError::Sqlite(e)),
            }
        })?
    } else {
        None
    };

    // Project requested but not found — error with non-zero exit
    if let Some(name) = effective_project
        && project_id.is_none()
    {
        return Err(CliError::Other(format!(
            "no project found with name \"{name}\""
        )));
    }

    let sql = build_find_sql(&args.keywords, args.archived, project_id.is_some());

    let notes = db.read(|conn| {
        let mut params_vec: Vec<Box<dyn rusqlite::types::ToSql>> = vec![];

        // 3 params per keyword (title, content, summary)
        for kw in &args.keywords {
            let pattern = format!("%{kw}%");
            params_vec.push(Box::new(pattern.clone()));
            params_vec.push(Box::new(pattern.clone()));
            params_vec.push(Box::new(pattern));
        }
        if let Some(pid) = project_id {
            params_vec.push(Box::new(pid));
        }
        params_vec.push(Box::new(args.limit));

        let mut stmt = conn.prepare(&sql)?;
        let param_refs: Vec<&dyn rusqlite::types::ToSql> =
            params_vec.iter().map(std::convert::AsRef::as_ref).collect();
        let rows = stmt.query_map(param_refs.as_slice(), Note::from_row)?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| CliError::Other(format!("Failed to read note rows: {e}")))
    })?;

    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&notes).map_err(CliError::Json)?
        );
    } else if notes.is_empty() {
        println!("No notes found matching: {}", args.keywords.join(", "));
    } else {
        print_notes_table(&notes);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_find_sql_single_keyword() {
        let sql = build_find_sql(&["rust".to_string()], false, false);
        assert!(sql.contains("title LIKE ?"), "should search title");
        assert!(sql.contains("content LIKE ?"), "should search content");
        assert!(sql.contains("summary LIKE ?"), "should search summary");
        assert!(
            sql.contains("deleted_at IS NULL"),
            "should exclude archived"
        );
    }

    #[test]
    fn test_build_find_sql_multiple_keywords() {
        let sql = build_find_sql(&["rust".to_string(), "effect".to_string()], false, false);
        let or_count = sql.match_indices(" OR (").count();
        assert_eq!(or_count, 1, "two keywords → one OR between them");
    }

    #[test]
    fn test_build_find_sql_archived() {
        let sql = build_find_sql(&["rust".to_string()], true, false);
        assert!(sql.contains("deleted_at IS NOT NULL"));
    }

    #[test]
    fn test_build_find_sql_with_project_placeholder() {
        let sql = build_find_sql(&["rust".to_string()], false, true);
        assert!(sql.contains("project_id = ?"));
    }

    #[test]
    fn test_build_find_sql_placeholder_count() {
        // 2 keywords × 3 fields + 1 project + 1 limit = 8 placeholders
        let sql = build_find_sql(&["a".to_string(), "b".to_string()], false, true);
        let placeholder_count = sql.match_indices('?').count();
        assert_eq!(
            placeholder_count, 8,
            "2 keywords × 3 + project + limit = 8 placeholders"
        );
    }

    #[test]
    fn test_build_find_sql_order_by() {
        let sql = build_find_sql(&["rust".to_string()], false, false);
        assert!(
            sql.contains("ORDER BY updated_at DESC"),
            "find sorts by updated_at, not created_at"
        );
    }

    #[test]
    #[should_panic(expected = "requires at least one keyword")]
    fn test_build_find_sql_panics_on_empty_keywords() {
        build_find_sql(&[], false, false);
    }
}
