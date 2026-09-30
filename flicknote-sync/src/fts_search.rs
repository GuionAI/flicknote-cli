//! FTS5 search projection on PowerSync's physical notes table.
//!
//! The pinned PowerSync core names its remote-backed table `ps_data__notes`;
//! both view writes and downloaded rows reach its triggers.

use powersync::PowerSyncDatabase;
use rusqlite::{Connection, Result, params};

use flicknote_client::dto::{NoteFindInput, SearchHit, SearchSnippet, SnippetSegment};

const BACKING_TABLE: &str = "ps_data__notes";
pub const FTS_SCHEMA_VERSION: i64 = 1;

/// Install an FTS index and triggers on the pinned PowerSync backing table.
/// Reopening an already indexed database leaves existing rows intact.
pub fn install(connection: &Connection) -> Result<()> {
    let exists: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1)",
        [BACKING_TABLE],
        |row| row.get(0),
    )?;
    if !exists {
        return Err(rusqlite::Error::InvalidQuery);
    }
    let transaction = connection.unchecked_transaction()?;
    transaction.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS note_search_meta (
            id INTEGER PRIMARY KEY CHECK (id = 1),
            schema_version INTEGER NOT NULL
        );
        CREATE VIRTUAL TABLE IF NOT EXISTS note_search_fts USING fts5(
            uuid UNINDEXED, short_id UNINDEXED, project_id UNINDEXED,
            created_at UNINDEXED, updated_at UNINDEXED,
            title, summary, content,
            tokenize='better_trigram', detail=full
        );
        CREATE TRIGGER IF NOT EXISTS note_search_insert AFTER INSERT ON ps_data__notes
        WHEN json_extract(new.data, '$.deleted_at') IS NULL
        BEGIN
            INSERT INTO note_search_fts
                (rowid, uuid, short_id, project_id, created_at, updated_at, title, summary, content)
            VALUES (new.rowid, new.id,
                json_extract(new.data, '$.short_id'), json_extract(new.data, '$.project_id'),
                json_extract(new.data, '$.created_at'), json_extract(new.data, '$.updated_at'),
                json_extract(new.data, '$.title'), json_extract(new.data, '$.summary'),
                json_extract(new.data, '$.content'));
        END;
        CREATE TRIGGER IF NOT EXISTS note_search_update AFTER UPDATE ON ps_data__notes
        BEGIN
            DELETE FROM note_search_fts WHERE rowid=old.rowid;
            INSERT INTO note_search_fts
                (rowid, uuid, short_id, project_id, created_at, updated_at, title, summary, content)
            SELECT new.rowid, new.id,
                json_extract(new.data, '$.short_id'), json_extract(new.data, '$.project_id'),
                json_extract(new.data, '$.created_at'), json_extract(new.data, '$.updated_at'),
                json_extract(new.data, '$.title'), json_extract(new.data, '$.summary'),
                json_extract(new.data, '$.content')
            WHERE json_extract(new.data, '$.deleted_at') IS NULL;
        END;
        CREATE TRIGGER IF NOT EXISTS note_search_delete AFTER DELETE ON ps_data__notes
        BEGIN
            DELETE FROM note_search_fts WHERE rowid=old.rowid;
        END;
        INSERT INTO note_search_fts
            (rowid, uuid, short_id, project_id, created_at, updated_at, title, summary, content)
        SELECT n.rowid, n.id,
            json_extract(n.data, '$.short_id'), json_extract(n.data, '$.project_id'),
            json_extract(n.data, '$.created_at'), json_extract(n.data, '$.updated_at'),
            json_extract(n.data, '$.title'), json_extract(n.data, '$.summary'),
            json_extract(n.data, '$.content')
        FROM ps_data__notes n
        WHERE json_extract(n.data, '$.deleted_at') IS NULL
          AND NOT EXISTS (SELECT 1 FROM note_search_fts f WHERE f.rowid=n.rowid);
        "#,
    )?;
    transaction.execute(
        "INSERT OR IGNORE INTO note_search_meta (id, schema_version) VALUES (1, ?1)",
        [FTS_SCHEMA_VERSION],
    )?;
    transaction.commit()
}

fn reset_schema(connection: &Connection) -> Result<()> {
    let transaction = connection.unchecked_transaction()?;
    transaction.execute_batch(
        "DROP TRIGGER IF EXISTS note_search_insert; \
         DROP TRIGGER IF EXISTS note_search_update; \
         DROP TRIGGER IF EXISTS note_search_delete; \
         DROP TABLE IF EXISTS note_search_fts; \
         DROP TABLE IF EXISTS note_search_meta;",
    )?;
    transaction.commit()
}

/// Recover a missing or damaged disposable FTS index from canonical rows.
pub fn rebuild(connection: &Connection) -> Result<()> {
    let transaction = connection.unchecked_transaction()?;
    transaction.execute_batch(
        r#"
        DELETE FROM note_search_fts;
        INSERT INTO note_search_fts
            (rowid, uuid, short_id, project_id, created_at, updated_at, title, summary, content)
        SELECT n.rowid, n.id,
            json_extract(n.data, '$.short_id'), json_extract(n.data, '$.project_id'),
            json_extract(n.data, '$.created_at'), json_extract(n.data, '$.updated_at'),
            json_extract(n.data, '$.title'), json_extract(n.data, '$.summary'),
            json_extract(n.data, '$.content')
        FROM ps_data__notes n
        WHERE json_extract(n.data, '$.deleted_at') IS NULL;
        "#,
    )?;
    transaction.commit()
}

fn parse_snippet(value: &str) -> SearchSnippet {
    const OPEN: char = '\u{1f}';
    const CLOSE: char = '\u{1e}';
    let mut segments = Vec::new();
    let mut rest = value;
    while let Some(start) = rest.find(OPEN) {
        if start > 0 {
            segments.push(SnippetSegment {
                text: rest[..start].to_owned(),
                highlighted: false,
            });
        }
        rest = &rest[start + OPEN.len_utf8()..];
        let Some(end) = rest.find(CLOSE) else {
            // Malformed markup is kept as ordinary text rather than exposed
            // as a dangling highlight span.
            segments.push(SnippetSegment {
                text: rest.to_owned(),
                highlighted: false,
            });
            return SearchSnippet { segments };
        };
        segments.push(SnippetSegment {
            text: rest[..end].to_owned(),
            highlighted: true,
        });
        rest = &rest[end + CLOSE.len_utf8()..];
    }
    if !rest.is_empty() {
        segments.push(SnippetSegment {
            text: rest.to_owned(),
            highlighted: false,
        });
    }
    SearchSnippet { segments }
}

/// Retrieve non-draft indexed candidates and rank by literal term coverage (3/2/1).
/// FTS markup is converted to the public segmented snippet contract.
pub fn search(
    connection: &Connection,
    terms: &[String],
    project: Option<&str>,
    limit: usize,
) -> Result<Vec<SearchHit>> {
    // The ranked candidates and their snippets must see the same FTS rows.
    let snapshot = connection.unchecked_transaction()?;
    let hits = search_internal(
        &snapshot,
        terms,
        SearchFilters::project(project),
        limit,
        true,
    )?;
    snapshot.commit()?;
    Ok(hits)
}

/// Measure candidate retrieval and coverage ranking without snippet work.
pub fn search_ids(
    connection: &Connection,
    terms: &[String],
    project: Option<&str>,
    limit: usize,
) -> Result<Vec<SearchHit>> {
    search_internal(
        connection,
        terms,
        SearchFilters::project(project),
        limit,
        false,
    )
}

#[derive(Clone, Copy)]
struct SearchFilters<'a> {
    project: Option<&'a str>,
    created_after: Option<i64>,
    created_before: Option<i64>,
    human: bool,
}

impl<'a> SearchFilters<'a> {
    fn project(project: Option<&'a str>) -> Self {
        Self {
            project,
            created_after: None,
            created_before: None,
            human: false,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
enum QueryTerm<'a> {
    Exact(&'a str),
    Prefix(&'a str),
    CjkFragment(&'a str),
}

fn contains_cjk(term: &str) -> bool {
    term.chars().any(|character| {
        matches!(character,
            '\u{3400}'..='\u{9fff}' | '\u{f900}'..='\u{faff}' |
            '\u{3040}'..='\u{30ff}' | '\u{ac00}'..='\u{d7af}')
    })
}

fn plan_query(terms: &[String]) -> Vec<QueryTerm<'_>> {
    let last = terms.len().saturating_sub(1);
    terms
        .iter()
        .enumerate()
        .map(|(index, term)| {
            if contains_cjk(term) {
                QueryTerm::CjkFragment(term)
            } else if index == last
                && term.chars().count() >= 2
                && term
                    .chars()
                    .any(|character| character.is_ascii_alphabetic())
            {
                QueryTerm::Prefix(term)
            } else {
                QueryTerm::Exact(term)
            }
        })
        .collect()
}

fn match_expression(terms: &[String]) -> String {
    plan_query(terms)
        .into_iter()
        .map(|term| {
            let (text, prefix) = match term {
                QueryTerm::Exact(text) | QueryTerm::CjkFragment(text) => (text, false),
                QueryTerm::Prefix(text) => (text, true),
            };
            let escaped = text.replace('"', "\"\"");
            format!("\"{escaped}\"{}", if prefix { "*" } else { "" })
        })
        .collect::<Vec<_>>()
        .join(" OR ")
}

const SEARCH_QUERY: &str = r#"
        SELECT f.rowid, f.short_id, f.title, f.summary,
            f.created_at, f.updated_at, f.project_id,
            json_extract(n.data, '$.type'),
            length(CAST(coalesce(json_extract(n.data, '$.content'), '') AS BLOB)),
            json_extract(n.data, '$.status') = 'draft',
            (SELECT coalesce(sum(CASE
                WHEN instr(lower(coalesce(f.title, '')), lower(t.value)) > 0 THEN 3
                WHEN instr(lower(coalesce(f.summary, '')), lower(t.value)) > 0 THEN 2
                WHEN instr(lower(coalesce(f.content, '')), lower(t.value)) > 0 THEN 1
                ELSE 0 END), 0)
             FROM json_each(?2) t) AS coverage
        FROM note_search_fts f
        JOIN ps_data__notes n ON n.rowid = f.rowid
        WHERE note_search_fts MATCH ?1
          AND json_extract(n.data, '$.status') IS NOT 'draft'
          AND (?4 IS NULL OR f.project_id IN (SELECT id FROM projects WHERE name=?4))
          AND (?5 IS NULL OR
               CAST(strftime('%s', f.created_at) AS INTEGER) * 1000000 +
               CASE WHEN substr(f.created_at, 20, 1) = '.'
                    THEN CAST(round(CAST(substr(f.created_at, 20) AS REAL) * 1000000) AS INTEGER)
                    ELSE 0 END >= ?5)
          AND (?6 IS NULL OR
               CAST(strftime('%s', f.created_at) AS INTEGER) * 1000000 +
               CASE WHEN substr(f.created_at, 20, 1) = '.'
                    THEN CAST(round(CAST(substr(f.created_at, 20) AS REAL) * 1000000) AS INTEGER)
                    ELSE 0 END < ?6)
          AND (NOT ?7 OR json_extract(json_extract(n.data, '$.metadata'), '$.created_by') IS NULL)
        ORDER BY coverage DESC, f.updated_at DESC, f.short_id DESC, f.rowid DESC
        LIMIT ?3
        "#;

fn search_internal(
    connection: &Connection,
    terms: &[String],
    filters: SearchFilters<'_>,
    limit: usize,
    with_snippets: bool,
) -> Result<Vec<SearchHit>> {
    if terms.is_empty()
        || limit == 0
        || (terms.len() == 1
            && terms[0].chars().count() == 1
            && terms[0]
                .chars()
                .all(|character| character.is_ascii_alphabetic()))
    {
        return Ok(Vec::new());
    }
    let expression = match_expression(terms);
    let terms_json = serde_json::to_string(terms)
        .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))?;
    let mut statement = connection.prepare(SEARCH_QUERY)?;
    let mut hits: Vec<(i64, SearchHit)> = statement
        .query_map(
            params![
                expression,
                terms_json,
                limit as i64,
                filters.project,
                filters.created_after,
                filters.created_before,
                filters.human
            ],
            |row| {
                Ok((
                    row.get(0)?,
                    SearchHit {
                        short_id: row.get(1)?,
                        note_type: row.get(7)?,
                        content_bytes: row.get::<_, i64>(8)? as u64,
                        draft: row.get(9)?,
                        title: row.get(2)?,
                        summary: row.get(3)?,
                        created_at: row.get(4)?,
                        updated_at: row.get(5)?,
                        project_id: row.get(6)?,
                        snippet: SearchSnippet::default(),
                    },
                ))
            },
        )?
        .collect::<Result<_>>()?;
    if !with_snippets {
        return Ok(hits.into_iter().map(|(_, hit)| hit).collect());
    }
    let snippet_ids = hits.iter().map(|(rowid, _)| *rowid).collect::<Vec<_>>();
    if !snippet_ids.is_empty() {
        let ids_json = serde_json::to_string(&snippet_ids)
            .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))?;
        let mut snippets = connection.prepare(
            "SELECT rowid, coalesce(snippet(note_search_fts, -1, char(31), char(30), '…', 48), '') \
             FROM note_search_fts WHERE note_search_fts MATCH ?1 \
             AND rowid IN (SELECT value FROM json_each(?2))",
        )?;
        let snippets = snippets
            .query_map(params![expression, ids_json], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
            })?
            .collect::<Result<std::collections::HashMap<_, _>>>()?;
        for (rowid, hit) in &mut hits {
            let snippet = snippets
                .get(rowid)
                .ok_or(rusqlite::Error::QueryReturnedNoRows)?;
            hit.snippet = parse_snippet(snippet);
        }
    }
    Ok(hits.into_iter().map(|(_, hit)| hit).collect())
}

/// Daemon-side FTS path using PowerSync reader and writer leases.
#[derive(Clone)]
pub struct FtsSearchService {
    db: PowerSyncDatabase,
}

#[async_trait::async_trait]
impl crate::search::NoteSearch for FtsSearchService {
    async fn find(&self, input: &NoteFindInput) -> Result<Vec<SearchHit>, String> {
        Self::find(self, input).await
    }
}

impl FtsSearchService {
    pub fn new(db: PowerSyncDatabase) -> Self {
        Self { db }
    }

    /// Verify or install the trigger-maintained index before accepting find
    /// requests. Version and trigger mismatches discard the disposable index.
    pub async fn prepare(&self) -> std::result::Result<(), String> {
        let writer = self.db.writer().await.map_err(|error| error.to_string())?;
        let exists: bool = writer
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='note_search_fts')",
                [],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())?;
        let schema_version = writer
            .query_row(
                "SELECT schema_version FROM note_search_meta WHERE id=1",
                [],
                |row| row.get::<_, i64>(0),
            )
            .ok();
        let trigger_count: i64 = writer
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE type='trigger' \
                 AND tbl_name='ps_data__notes' \
                 AND name IN ('note_search_insert', 'note_search_update', 'note_search_delete')",
                [],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())?;
        let integrity_ok = exists
            && writer
                .execute(
                    "INSERT INTO note_search_fts(note_search_fts) VALUES('integrity-check')",
                    [],
                )
                .is_ok();
        if !exists
            || schema_version != Some(FTS_SCHEMA_VERSION)
            || trigger_count != 3
            || !integrity_ok
        {
            log::warn!(
                "FTS schema requires rebuild: version={schema_version:?} triggers={trigger_count} integrity_ok={integrity_ok}"
            );
            reset_schema(&writer).map_err(|error| error.to_string())?;
        }
        install(&writer).map_err(|error| error.to_string())?;
        let source_count: i64 = writer
            .query_row(
                "SELECT count(*) FROM ps_data__notes WHERE json_extract(data, '$.deleted_at') IS NULL",
                [],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())?;
        let indexed_count: i64 = writer
            .query_row("SELECT count(*) FROM note_search_fts", [], |row| row.get(0))
            .map_err(|error| error.to_string())?;
        if source_count != indexed_count {
            log::warn!("FTS row count differs from source; rebuilding index");
            rebuild(&writer).map_err(|error| error.to_string())?;
        }
        Ok(())
    }

    pub async fn find(&self, input: &NoteFindInput) -> std::result::Result<Vec<SearchHit>, String> {
        if input.archived || !input.extractions.is_empty() {
            return Err("FTS search supports active keyword searches only".into());
        }
        let terms = input
            .keywords
            .iter()
            .map(|keyword| keyword.trim())
            .filter(|keyword| !keyword.is_empty())
            .map(str::to_owned)
            .collect::<Vec<_>>();
        let (created_after, created_before) =
            flicknote_core::services::note::validate_created_range(
                input.created_after.as_deref(),
                input.created_before.as_deref(),
            )
            .map_err(|error| error.to_string())?;
        let reader = self.db.reader().await.map_err(|error| error.to_string())?;
        let snapshot = reader
            .unchecked_transaction()
            .map_err(|error| error.to_string())?;
        let hits = search_internal(
            &snapshot,
            &terms,
            SearchFilters {
                project: input.project.as_deref(),
                created_after,
                created_before,
                human: input.human,
            },
            input.limit as usize,
            true,
        )
        .map_err(|error| error.to_string())?;
        snapshot.commit().map_err(|error| error.to_string())?;
        Ok(hits)
    }
}

#[cfg(test)]
mod tests {
    use self::mock_sync::{
        MockSyncService, TestConnector, send_checkpoint, send_complete, send_op,
    };
    use super::*;
    use crate::test_support::{test_powersync_db, test_powersync_db_at};
    use flicknote_core::schema::app_schema;
    use powersync::{ConnectionPool, PowerSyncDatabase, SyncOptions, env::PowerSyncEnvironment};
    use rusqlite::params;
    use std::sync::Arc;
    use std::time::Duration;

    #[test]
    fn snippet_segments_keep_unicode_graphemes_and_plain_markup() {
        let snippet = parse_snippet("<b> \u{1f}👩‍💻\u{1e} and \u{1f}a\u{301}\u{1e} \u{1f}中文\u{1e}");
        assert_eq!(snippet.segments.len(), 6);
        assert_eq!(snippet.segments[0].text, "<b> ");
        assert!(!snippet.segments[0].highlighted);
        assert_eq!(snippet.segments[1].text, "👩‍💻");
        assert!(snippet.segments[1].highlighted);
        assert_eq!(snippet.segments[3].text, "a\u{301}");
        assert!(snippet.segments[3].highlighted);
        assert_eq!(snippet.segments[5].text, "中文");
        assert!(snippet.segments[5].highlighted);
    }

    #[test]
    fn final_latin_term_uses_fts_prefix_and_cjk_fragments_stay_exact() {
        assert_eq!(
            match_expression(&["office".into(), "pla".into()]),
            "\"office\" OR \"pla\"*"
        );
        assert_eq!(match_expression(&["o".into()]), "\"o\"");
        assert_eq!(match_expression(&["中文".into()]), "\"中文\"");
        assert_eq!(
            match_expression(&["office".into(), "中".into()]),
            "\"office\" OR \"中\""
        );
        assert_eq!(
            match_expression(&["design-review".into(), "pla".into()]),
            "\"design-review\" OR \"pla\"*"
        );
        assert_eq!(match_expression(&["a\"b".into()]), "\"a\"\"b\"*");
    }

    #[tokio::test]
    #[expect(
        clippy::too_many_lines,
        reason = "one fixture verifies search filtering and snippets"
    )]
    async fn prefix_search_keeps_project_filter_and_snippet() {
        let (_directory, db) = test_powersync_db().await;
        let writer = db.writer().await.unwrap();
        install(&writer).unwrap();
        writer
            .execute(
                "INSERT INTO projects (id, name) VALUES ('project-1', 'lab')",
                [],
            )
            .unwrap();
        writer
            .execute(
                "INSERT INTO notes (id, type, status, title, content, project_id) VALUES ('first', 'normal', 'ready', 'Office planning', '中文 notes design-review', 'project-1')",
                [],
            )
            .unwrap();
        writer
            .execute(
                "INSERT INTO notes (id, type, status, title, content) VALUES ('second', 'normal', 'ready', 'Officer handbook', 'unrelated')",
                [],
            )
            .unwrap();

        let hits = search(&writer, &["offi".into()], Some("lab"), 10).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].title.as_deref(), Some("Office planning"));
        assert_eq!(hits[0].title.as_deref(), Some("Office planning"));
        assert_eq!(hits[0].project_id.as_deref(), Some("project-1"));
        assert!(
            hits[0]
                .snippet
                .segments
                .iter()
                .any(|segment| segment.highlighted)
        );
        assert_eq!(
            search_ids(&writer, &["offi".into()], None, 10)
                .unwrap()
                .len(),
            2
        );
        assert!(search(&writer, &["o".into()], None, 10).unwrap().is_empty());
        assert_eq!(search(&writer, &["中".into()], None, 10).unwrap().len(), 1);
        assert_eq!(
            search(&writer, &["中文".into()], None, 10).unwrap().len(),
            1
        );
        assert_eq!(
            search(&writer, &["Office".into(), "中文".into()], Some("lab"), 10)
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            search(&writer, &["design-review".into()], Some("lab"), 10)
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            search(&writer, &["Office".into()], Some("missing"), 10)
                .unwrap()
                .len(),
            0
        );
        writer
            .execute(
                r#"UPDATE notes SET metadata='{"created_by":"mcp"}' WHERE id='first'"#,
                [],
            )
            .unwrap();
        drop(writer);
        let service = FtsSearchService::new(db);
        let mut input = NoteFindInput {
            keywords: vec!["中文".into(), "offi".into()],
            extractions: Vec::new(),
            project: Some("lab".into()),
            created_after: None,
            created_before: None,
            human: false,
            archived: false,
            limit: 10,
        };
        let hits = service.find(&input).await.unwrap();
        assert_eq!(hits.len(), 1);
        input.human = true;
        assert!(service.find(&input).await.unwrap().is_empty());
        input.human = false;
        input.created_after = Some("2099-01-01T00:00:00Z".into());
        assert!(service.find(&input).await.unwrap().is_empty());
        input.created_after = None;
        assert_eq!(hits[0].title.as_deref(), Some("Office planning"));
        assert!(
            hits[0]
                .snippet
                .segments
                .iter()
                .any(|segment| segment.highlighted)
        );
        input.keywords = vec!["中".into()];
        assert_eq!(service.find(&input).await.unwrap().len(), 1);
        input.keywords = vec!["o".into()];
        assert!(service.find(&input).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn every_returned_hit_has_a_segmented_snippet() {
        let (_directory, db) = test_powersync_db().await;
        let writer = db.writer().await.unwrap();
        install(&writer).unwrap();
        for index in 0..12 {
            writer
                .execute(
                    "INSERT INTO notes (id, short_id, type, status, title, content) VALUES (?1, ?2, 'normal', 'ready', 'Office', 'planning')",
                    params![format!("note-{index}"), index],
                )
                .unwrap();
        }
        let hits = search(&writer, &["offi".into()], None, 12).unwrap();
        assert_eq!(hits.len(), 12);
        assert_eq!(hits[0].short_id, Some(11));
        assert_eq!(hits[11].short_id, Some(0));
        assert!(hits.iter().all(|hit| {
            hit.snippet
                .segments
                .iter()
                .any(|segment| segment.highlighted)
        }));
        let serialized = serde_json::to_value(&hits[0]).unwrap();
        for field in [
            "short_id",
            "type",
            "content_bytes",
            "draft",
            "title",
            "summary",
            "created_at",
            "updated_at",
            "project_id",
            "snippet",
        ] {
            assert!(serialized.get(field).is_some(), "missing {field}");
        }
        assert!(serialized.get("score").is_none());
        assert!(serialized.get("uuid").is_none());
        assert!(serialized.get("content").is_none());
        assert!(serialized.get("preview").is_none());
        assert_eq!(serialized["snippet"]["segments"][0]["highlighted"], true);
    }

    #[tokio::test]
    async fn search_reads_canonical_metadata_and_excludes_drafts_without_changing_rank() {
        let (_directory, db) = test_powersync_db().await;
        let writer = db.writer().await.unwrap();
        install(&writer).unwrap();
        writer
            .execute(
                "INSERT INTO projects (id, name) VALUES ('project-1', 'lab')",
                [],
            )
            .unwrap();
        for (id, short_id, note_type, status, title, content) in [
            ("short", 1, "normal", "ready", "Office", "é"),
            (
                "long",
                2,
                "meeting",
                "ai_queued",
                "Other",
                "Office planning 👩‍💻",
            ),
            (
                "queued",
                3,
                "link",
                "source_queued",
                "Other",
                "Office source",
            ),
            (
                "draft",
                4,
                "normal",
                "draft",
                "Office draft",
                "Office secret",
            ),
        ] {
            writer.execute(
                "INSERT INTO notes (id, short_id, type, status, title, content, project_id) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'project-1')",
                params![id, short_id, note_type, status, title, content],
            ).unwrap();
        }
        let hits = search(&writer, &["Office".into()], Some("lab"), 10).unwrap();
        assert_eq!(
            hits.iter().map(|hit| hit.short_id).collect::<Vec<_>>(),
            vec![Some(1), Some(3), Some(2)]
        );
        assert_eq!(
            hits.iter()
                .map(|hit| hit.note_type.as_str())
                .collect::<Vec<_>>(),
            vec!["normal", "link", "meeting"]
        );
        assert_eq!(
            hits.iter().map(|hit| hit.content_bytes).collect::<Vec<_>>(),
            vec![
                "é".len() as u64,
                "Office source".len() as u64,
                "Office planning 👩‍💻".len() as u64
            ]
        );
        assert!(
            hits.iter()
                .all(|hit| !hit.draft && hit.project_id.as_deref() == Some("project-1"))
        );
        assert!(hits.iter().all(|hit| {
            hit.snippet
                .segments
                .iter()
                .any(|segment| segment.highlighted)
        }));
        assert_eq!(
            search_ids(&writer, &["Office".into()], Some("lab"), 10)
                .unwrap()
                .len(),
            3
        );
    }

    #[tokio::test]
    async fn startup_recovers_missing_and_incomplete_index() {
        let (_directory, db) = test_powersync_db().await;
        let service = FtsSearchService::new(db.clone());
        service.prepare().await.unwrap();
        {
            let writer = db.writer().await.unwrap();
            writer
                .execute(
                    "INSERT INTO notes (id, type, status, title, content) VALUES ('recover', 'normal', 'ready', 'Office', 'planning')",
                    [],
                )
                .unwrap();
            writer.execute("DELETE FROM note_search_fts", []).unwrap();
        }
        service.prepare().await.unwrap();
        let input = NoteFindInput {
            keywords: vec!["offi".into()],
            extractions: Vec::new(),
            project: None,
            created_after: None,
            created_before: None,
            human: false,
            archived: false,
            limit: 10,
        };
        assert_eq!(service.find(&input).await.unwrap().len(), 1);
        {
            let writer = db.writer().await.unwrap();
            writer.execute_batch("DROP TABLE note_search_fts").unwrap();
        }
        service.prepare().await.unwrap();
        assert_eq!(service.find(&input).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn schema_version_mismatch_discards_and_rebuilds_index() {
        let (_directory, db) = test_powersync_db().await;
        let service = FtsSearchService::new(db.clone());
        service.prepare().await.unwrap();
        {
            let writer = db.writer().await.unwrap();
            writer
                .execute(
                    "INSERT INTO notes (id, type, status, title) VALUES ('versioned', 'normal', 'ready', 'Current title')",
                    [],
                )
                .unwrap();
            writer
                .execute(
                    "UPDATE note_search_fts SET title='Stale title' WHERE uuid='versioned'",
                    [],
                )
                .unwrap();
            writer
                .execute(
                    "UPDATE note_search_meta SET schema_version=0 WHERE id=1",
                    [],
                )
                .unwrap();
        }
        service.prepare().await.unwrap();
        let reader = db.reader().await.unwrap();
        let version: i64 = reader
            .query_row("SELECT schema_version FROM note_search_meta", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(version, FTS_SCHEMA_VERSION);
        assert_eq!(
            search(&reader, &["Current".into()], None, 10)
                .unwrap()
                .len(),
            1
        );
        assert!(
            search(&reader, &["Stale".into()], None, 10)
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn missing_trigger_rebuilds_stale_index_even_when_row_count_matches() {
        let (_directory, db) = test_powersync_db().await;
        let service = FtsSearchService::new(db.clone());
        service.prepare().await.unwrap();
        {
            let writer = db.writer().await.unwrap();
            writer
                .execute(
                    "INSERT INTO notes (id, type, status, title) VALUES ('triggered', 'normal', 'ready', 'Old title')",
                    [],
                )
                .unwrap();
            writer
                .execute_batch("DROP TRIGGER note_search_update")
                .unwrap();
            writer
                .execute(
                    "UPDATE notes SET title='New title' WHERE id='triggered'",
                    [],
                )
                .unwrap();
            let counts: (i64, i64) = writer
                .query_row(
                    "SELECT (SELECT count(*) FROM ps_data__notes), (SELECT count(*) FROM note_search_fts)",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .unwrap();
            assert_eq!(counts.0, counts.1);
        }
        service.prepare().await.unwrap();
        let reader = db.reader().await.unwrap();
        assert_eq!(search(&reader, &["New".into()], None, 10).unwrap().len(), 1);
        assert!(
            search(&reader, &["Old".into()], None, 10)
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    #[allow(clippy::too_many_lines)] // End-to-end local, physical, and reopen assertions share one fixture.
    async fn triggers_cover_view_writes_physical_writes_readers_and_reopen() {
        let (directory, db) = test_powersync_db().await;
        let path = directory.path().join("test.db");
        {
            let writer = db.writer().await.unwrap();
            install(&writer).unwrap();
            install(&writer).unwrap();
            writer
                .execute(
                    "INSERT INTO projects (id, name) VALUES ('project-1', 'lab')",
                    [],
                )
                .unwrap();
            writer
                .execute(
                    "INSERT INTO notes (id, type, status, title, content, project_id) VALUES (?1, 'normal', 'ready', ?2, ?3, 'project-1')",
                    params!["local", "bird", "👩‍💻 a\u{301} 中文 note"],
                )
                .unwrap();
        }
        for _ in 0..3 {
            let reader = db.reader().await.unwrap();
            let count: i64 = reader
                .query_row(
                    "SELECT count(*) FROM note_search_fts WHERE note_search_fts MATCH '中'",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(count, 1);
            let hits = search(&reader, &["中".into()], None, 10).unwrap();
            assert_eq!(hits[0].title.as_deref(), Some("bird"));
            assert_eq!(
                search(&reader, &["中".into()], Some("lab"), 10)
                    .unwrap()
                    .len(),
                1
            );
            assert!(
                search(&reader, &["中".into()], Some("other"), 10)
                    .unwrap()
                    .is_empty()
            );
            assert!(
                hits[0]
                    .snippet
                    .segments
                    .iter()
                    .any(|segment| segment.highlighted && segment.text == "中")
            );
            assert!(
                hits[0]
                    .snippet
                    .segments
                    .iter()
                    .any(|segment| segment.text.contains("👩‍💻 a\u{301}"))
            );
        }
        {
            let writer = db.writer().await.unwrap();
            writer.execute("DELETE FROM note_search_fts", []).unwrap();
            rebuild(&writer).unwrap();
            let count: i64 = writer
                .query_row("SELECT count(*) FROM note_search_fts", [], |r| r.get(0))
                .unwrap();
            assert_eq!(count, 1);
            writer.execute_batch("DROP TABLE note_search_fts").unwrap();
            install(&writer).unwrap();
            let count: i64 = writer
                .query_row("SELECT count(*) FROM note_search_fts", [], |r| r.get(0))
                .unwrap();
            assert_eq!(count, 1);
        }
        {
            let writer = db.writer().await.unwrap();
            // A downloaded row bypasses the notes view and changes this same
            // PowerSync-owned physical table. This characterizes the trigger
            // boundary without requiring a production sync service.
            writer
                .execute(
                    "INSERT INTO ps_data__notes (id, data) VALUES (?1, ?2)",
                    params!["remote", r#"{"type":"normal", "status":"ready", "title":"remote", "content":"青い鳥"}"#],
                )
                .unwrap();
            let count: i64 = writer
                .query_row(
                    "SELECT count(*) FROM note_search_fts WHERE note_search_fts MATCH '青'",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(count, 1);
            writer
                .execute(
                    "UPDATE ps_data__notes SET data=?2 WHERE id=?1",
                    params!["remote", r#"{"type":"normal", "status":"ready", "title":"remote", "content":"赤い鳥"}"#],
                )
                .unwrap();
            let changed = search(&writer, &["赤".into()], None, 10).unwrap();
            assert_eq!(changed.len(), 1);
            assert_eq!(changed[0].title.as_deref(), Some("remote"));
            assert!(
                search(&writer, &["青".into()], None, 10)
                    .unwrap()
                    .is_empty()
            );
            writer
                .execute("UPDATE notes SET content='changed' WHERE id='local'", [])
                .unwrap();
            let count: i64 = writer
                .query_row(
                    "SELECT count(*) FROM note_search_fts WHERE note_search_fts MATCH '中'",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(count, 0);
            writer
                .execute("DELETE FROM ps_data__notes WHERE id='remote'", [])
                .unwrap();
            let count: i64 = writer
                .query_row(
                    "SELECT count(*) FROM note_search_fts WHERE note_search_fts MATCH '赤'",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(count, 0);
            writer
                .execute(
                    "UPDATE notes SET deleted_at='2026-01-01' WHERE id='local'",
                    [],
                )
                .unwrap();
            let count: i64 = writer
                .query_row("SELECT count(*) FROM note_search_fts", [], |r| r.get(0))
                .unwrap();
            assert_eq!(count, 0);
        }
        drop(db);
        let reopened = test_powersync_db_at(&path, app_schema());
        let writer = reopened.writer().await.unwrap();
        install(&writer).unwrap();
        let count: i64 = writer
            .query_row("SELECT count(*) FROM note_search_fts", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }

    #[tokio::test]
    async fn remote_download_updates_the_fts_trigger() {
        PowerSyncEnvironment::powersync_auto_extension().unwrap();
        flicknote_core::sqlite_extension::register_better_trigram().unwrap();
        let directory = tempfile::tempdir().unwrap();
        let service = Arc::new(MockSyncService::new());
        let pool = ConnectionPool::open(directory.path().join("sync.db")).unwrap();
        let environment = PowerSyncEnvironment::custom(
            Arc::clone(&service).client(),
            pool,
            PowerSyncEnvironment::tokio_timer(),
        );
        let db = PowerSyncDatabase::new(environment, app_schema());
        let writer = db.writer().await.unwrap();
        install(&writer).unwrap();
        drop(writer);
        let tasks = db.async_tasks().spawn_with(tokio::spawn);
        db.connect(SyncOptions::new(TestConnector)).await;
        let request = tokio::time::timeout(Duration::from_secs(5), service.next_request())
            .await
            .unwrap();
        send_checkpoint(&request, 1).await;
        send_op(
            &request,
            1,
            "PUT",
            Some(r#"{"short_id":9,"title":"遠端鳥","content":"青い鳥"}"#),
        )
        .await;
        send_complete(&request, 1).await;
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let reader = db.reader().await.unwrap();
                let count: i64 = reader
                    .query_row(
                        "SELECT count(*) FROM note_search_fts WHERE note_search_fts MATCH '青'",
                        [],
                        |row| row.get(0),
                    )
                    .unwrap();
                if count == 1 {
                    break;
                }
                drop(reader);
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        send_checkpoint(&request, 2).await;
        send_op(&request, 2, "REMOVE", None).await;
        send_complete(&request, 2).await;
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let reader = db.reader().await.unwrap();
                let count: i64 = reader
                    .query_row(
                        "SELECT count(*) FROM note_search_fts WHERE note_search_fts MATCH '青'",
                        [],
                        |row| row.get(0),
                    )
                    .unwrap();
                if count == 0 {
                    break;
                }
                drop(reader);
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        drop(tasks);
    }

    mod mock_sync {
        use async_trait::async_trait;
        use bytes::Bytes;
        use futures_lite::{StreamExt, stream};
        use powersync::{
            BackendConnector, PowerSyncCredentials,
            error::PowerSyncError,
            http::{HttpClient, Request, Response, ResponseBody},
        };
        use serde_json::{Value, json};
        use tokio::sync::{Mutex, mpsc};

        type Lines = mpsc::Sender<Value>;

        pub(super) struct MockSyncService {
            client: MockClient,
            requests: Mutex<mpsc::UnboundedReceiver<Lines>>,
        }

        impl MockSyncService {
            pub(super) fn new() -> Self {
                let (send, receive) = mpsc::unbounded_channel();
                Self {
                    client: MockClient { requests: send },
                    requests: Mutex::new(receive),
                }
            }

            pub(super) fn client(&self) -> MockClient {
                self.client.clone()
            }

            pub(super) async fn next_request(&self) -> Lines {
                self.requests.lock().await.recv().await.unwrap()
            }
        }

        #[derive(Clone, Debug)]
        pub(super) struct MockClient {
            requests: mpsc::UnboundedSender<Lines>,
        }

        #[async_trait]
        impl HttpClient for MockClient {
            async fn send(&self, request: Request) -> Result<Response, PowerSyncError> {
                match request.url.path() {
                    "/sync/stream" => {
                        let (send, receive) = mpsc::channel(8);
                        self.requests.send(send).unwrap();
                        let reader = stream::unfold(receive, |mut receive| async move {
                            let line: Value = receive.recv().await?;
                            let bytes = Bytes::from(format!("{line}\n"));
                            Some((Ok(bytes), receive))
                        })
                        .boxed();
                        Ok(Response {
                            status: 200,
                            content_type: Some("application/json".into()),
                            body: ResponseBody {
                                reader,
                                length: None,
                            },
                        })
                    }
                    "/write-checkpoint2.json" => {
                        let bytes = Bytes::from_static(b"{\"data\":{\"write_checkpoint\":\"10\"}}");
                        Ok(Response {
                            status: 200,
                            content_type: Some("application/json".into()),
                            body: ResponseBody {
                                reader: stream::once(Ok(bytes)).boxed(),
                                length: None,
                            },
                        })
                    }
                    path => panic!("unexpected sync request: {path}"),
                }
            }
        }

        pub(super) struct TestConnector;

        #[async_trait]
        impl BackendConnector for TestConnector {
            async fn fetch_credentials(&self) -> Result<PowerSyncCredentials, PowerSyncError> {
                Ok(PowerSyncCredentials {
                    endpoint: "https://sync.test/".into(),
                    token: "test".into(),
                })
            }

            async fn upload_data(&self) -> Result<(), PowerSyncError> {
                Ok(())
            }
        }

        pub(super) async fn send_checkpoint(lines: &Lines, id: i64) {
            lines
                .send(json!({"checkpoint":{"last_op_id":id.to_string(),
                "streams":[{"name":"default_stream","is_default":true,"errors":[]}],
                "buckets":[{"bucket":"default_stream","checksum":0,"count":id,
                    "subscriptions":[{"default":0}]}]}}))
                .await
                .unwrap();
        }

        pub(super) async fn send_op(lines: &Lines, id: i64, op: &str, data: Option<&str>) {
            lines
                .send(json!({"data":{"bucket":"default_stream","data":[{
                    "checksum":0,"op_id":id.to_string(),"op":op,"object_id":"remote-note",
                    "object_type":"notes","subkey":null,"data":data
                }]}}))
                .await
                .unwrap();
        }

        pub(super) async fn send_complete(lines: &Lines, id: i64) {
            lines
                .send(json!({"checkpoint_complete":{"last_op_id":id.to_string()}}))
                .await
                .unwrap();
        }
    }
}
