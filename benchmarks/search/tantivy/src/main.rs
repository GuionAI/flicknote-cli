use std::collections::HashSet;
use std::error::Error;
use std::fs;
use std::path::Path;
use std::process::Command;
use std::time::Instant;

use rusqlite::{Connection, OpenFlags};
use serde::{Deserialize, Serialize};
use tantivy::collector::TopDocs;
use tantivy::query::{BooleanQuery, BoostQuery, Occur, Query, TermQuery};
use tantivy::schema::{
    FAST, Field, IndexRecordOption, STORED, STRING, Schema, TextFieldIndexing, TextOptions, Value,
};
use tantivy::snippet::SnippetGenerator;
use tantivy::tokenizer::{LowerCaser, NgramTokenizer, TextAnalyzer};
use tantivy::{Index, TantivyDocument, Term};

use flicknote_tantivy::{coverage, prefix};
mod probe;

#[derive(Clone, Copy, Default, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
enum SearchMode {
    #[default]
    Ngram,
    HybridPrefix,
    Coverage,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SearchQuery {
    id: String,
    audience: String,
    group: String,
    project: Option<String>,
    terms: Vec<String>,
    relevant_ids: Vec<u64>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Fixture {
    queries: Vec<SearchQuery>,
    candidate_limit: Option<usize>,
    #[serde(default)]
    mode: SearchMode,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct QueryResult {
    id: String,
    audience: String,
    group: String,
    elapsed_ms: f64,
    with_snippet_ms: f64,
    hits: Vec<u64>,
    snippets: Vec<Option<String>>,
    target_rank: Option<usize>,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct ProbeReport {
    resident_after_open_bytes: Option<u64>,
    resident_after_queries_bytes: Option<u64>,
    median_query_ms: f64,
    median_with_snippet_ms: f64,
    results: Vec<QueryResult>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Report {
    mode: SearchMode,
    note_count: usize,
    index_bytes: u64,
    build_seconds: f64,
    median_query_ms: f64,
    median_with_snippet_ms: f64,
    resident_after_open_bytes: Option<u64>,
    resident_after_queries_bytes: Option<u64>,
    results: Vec<QueryResult>,
}

fn grams(value: &str) -> Vec<String> {
    let chars: Vec<char> = value.to_lowercase().chars().collect();
    let mut unique = HashSet::new();
    for size in 2..=3 {
        for window in chars.windows(size) {
            unique.insert(window.iter().collect::<String>());
        }
    }
    unique.into_iter().collect()
}

fn query_for(terms: &[String], fields: [Field; 3]) -> Box<dyn Query> {
    let term_clauses = terms
        .iter()
        .filter_map(|text| {
            let gram_tokens = grams(text);
            if gram_tokens.is_empty() {
                return None;
            }
            let field_clauses = fields
                .into_iter()
                .zip([3.0, 2.0, 1.0])
                .map(|(field, weight)| {
                    let all_grams = gram_tokens
                        .iter()
                        .map(|gram| {
                            (
                                Occur::Must,
                                Box::new(TermQuery::new(
                                    Term::from_field_text(field, gram),
                                    IndexRecordOption::WithFreqs,
                                )) as Box<dyn Query>,
                            )
                        })
                        .collect();
                    (
                        Occur::Should,
                        Box::new(BoostQuery::new(
                            Box::new(BooleanQuery::new(all_grams)),
                            weight,
                        )) as Box<dyn Query>,
                    )
                })
                .collect();
            Some((
                Occur::Should,
                Box::new(BooleanQuery::new(field_clauses)) as Box<dyn Query>,
            ))
        })
        .collect();
    Box::new(BooleanQuery::new(term_clauses))
}

fn directory_bytes(path: &Path) -> Result<u64, Box<dyn Error>> {
    let mut total = 0;
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let metadata = entry.metadata()?;
        total += if metadata.is_dir() {
            directory_bytes(&entry.path())?
        } else {
            metadata.len()
        };
    }
    Ok(total)
}

fn median(values: &[f64]) -> f64 {
    let mut ordered = values.to_vec();
    ordered.sort_by(f64::total_cmp);
    let middle = ordered.len() / 2;
    if ordered.len().is_multiple_of(2) {
        (ordered[middle - 1] + ordered[middle]) / 2.0
    } else {
        ordered[middle]
    }
}

#[cfg(unix)]
fn resident_bytes() -> Option<u64> {
    let output = Command::new("ps")
        .args(["-o", "rss=", "-p", &std::process::id().to_string()])
        .output()
        .ok()?;
    let kib: u64 = String::from_utf8(output.stdout).ok()?.trim().parse().ok()?;
    kib.checked_mul(1024)
}

#[cfg(not(unix))]
fn resident_bytes() -> Option<u64> {
    None
}

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() == 4 && args[1] == "--probe" {
        let report = probe::run(Path::new(&args[2]), Path::new(&args[3]))?;
        println!("{}", serde_json::to_string(&report)?);
        return Ok(());
    }
    if args.len() != 4 {
        return Err("Usage: tantivy-bench SNAPSHOT_DB QUERIES_JSON OUTPUT_JSON".into());
    }
    let snapshot = Path::new(&args[1]);
    let fixture: Fixture = serde_json::from_slice(&fs::read(&args[2])?)?;
    if fixture.queries.is_empty() {
        return Err("Query fixture must not be empty".into());
    }
    let output = Path::new(&args[3]);
    let local_dir = output.parent().ok_or("Output needs a parent directory")?;
    fs::create_dir_all(local_dir)?;
    let index_dir = tempfile::Builder::new()
        .prefix("tantivy-ngram-")
        .tempdir_in(local_dir)?;

    let mut schema = Schema::builder();
    let short_id = schema.add_u64_field("short_id", STORED | FAST);
    let project = schema.add_text_field("project", STRING | STORED);
    let updated = schema.add_text_field("updated_at", STORED);
    let updated_sort = schema.add_i64_field("updated_sort", FAST);
    let ngram = TextOptions::default()
        .set_indexing_options(
            TextFieldIndexing::default()
                .set_tokenizer("ngram_2_3")
                .set_index_option(IndexRecordOption::WithFreqsAndPositions),
        )
        .set_stored();
    let title = schema.add_text_field("title", ngram.clone());
    let summary = schema.add_text_field("summary", ngram.clone());
    let content = schema.add_text_field("content", ngram);
    let word_fields = if fixture.mode != SearchMode::Ngram {
        let options = TextOptions::default().set_indexing_options(
            TextFieldIndexing::default()
                .set_tokenizer("words")
                .set_index_option(IndexRecordOption::WithFreqs),
        );
        Some([
            schema.add_text_field("title_words", options.clone()),
            schema.add_text_field("summary_words", options.clone()),
            schema.add_text_field("content_words", options),
        ])
    } else {
        None
    };
    let index = Index::create_in_dir(index_dir.path(), schema.build())?;
    index.tokenizers().register(
        "ngram_2_3",
        TextAnalyzer::builder(NgramTokenizer::new(2, 3, false)?)
            .filter(LowerCaser)
            .build(),
    );
    index.tokenizers().register("words", prefix::analyzer());

    let database = Connection::open_with_flags(snapshot, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let mut select = database.prepare(
        "SELECT notes.short_id, projects.name, notes.title, notes.summary, notes.content, notes.updated_at, COALESCE(CAST(unixepoch(notes.updated_at, 'subsec') * 1000 AS INTEGER), 0)
         FROM notes LEFT JOIN projects ON projects.id = notes.project_id
         WHERE notes.deleted_at IS NULL",
    )?;
    let mut rows = select.query([])?;
    let mut writer = index.writer(50_000_000)?;
    let started = Instant::now();
    let mut note_count = 0;
    while let Some(row) = rows.next()? {
        let id: Option<i64> = row.get(0)?;
        let Some(id) = id else { continue };
        let id = u64::try_from(id)?;
        let mut document = TantivyDocument::default();
        document.add_u64(short_id, id);
        document.add_text(
            project,
            row.get::<_, Option<String>>(1)?.unwrap_or_default(),
        );
        for (column, field) in [(2, title), (3, summary), (4, content)] {
            if let Some(text) = row.get::<_, Option<String>>(column)? {
                document.add_text(field, &text);
                if let Some(words) = word_fields {
                    document.add_text(words[column - 2], &text);
                }
            }
        }
        document.add_text(
            updated,
            row.get::<_, Option<String>>(5)?.unwrap_or_default(),
        );
        document.add_i64(updated_sort, row.get(6)?);
        writer.add_document(document)?;
        note_count += 1;
    }
    writer.commit()?;
    writer.wait_merging_threads()?;
    let build_seconds = started.elapsed().as_secs_f64();
    let index_bytes = directory_bytes(index_dir.path())?;
    drop(rows);
    drop(select);
    drop(database);
    drop(index);
    let child = Command::new(std::env::current_exe()?)
        .arg("--probe")
        .arg(index_dir.path())
        .arg(&args[2])
        .output()?;
    if !child.status.success() {
        return Err(format!("Probe failed: {}", String::from_utf8_lossy(&child.stderr)).into());
    }
    let probe: ProbeReport = serde_json::from_slice(&child.stdout)?;
    let report = Report {
        mode: fixture.mode,
        note_count,
        index_bytes,
        build_seconds,
        median_query_ms: probe.median_query_ms,
        median_with_snippet_ms: probe.median_with_snippet_ms,
        resident_after_open_bytes: probe.resident_after_open_bytes,
        resident_after_queries_bytes: probe.resident_after_queries_bytes,
        results: probe.results,
    };
    fs::write(output, serde_json::to_vec_pretty(&report)?)?;
    println!(
        "notes={} index_mib={:.1} build_s={:.1} search_ms={:.1} with_snippet_ms={:.1} warm_rss_mib={:.1}",
        report.note_count,
        report.index_bytes as f64 / 1_048_576.0,
        report.build_seconds,
        report.median_query_ms,
        report.median_with_snippet_ms,
        report.resident_after_queries_bytes.unwrap_or(0) as f64 / 1_048_576.0,
    );
    Ok(())
}
