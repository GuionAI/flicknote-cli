//! Run only on an isolated PowerSync SQLite snapshot. The output DB is new.
use std::io;
use std::time::Instant;

use flicknote_sync::fts_search;
use rusqlite::Connection;
use serde_json::{Value, json};

#[allow(clippy::print_stdout)]
#[allow(clippy::too_many_lines)] // One-shot benchmark setup, query loop, and measurements.
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().collect();
    if args.len() != 5 {
        return Err("usage: fts_spike_bench SNAPSHOT FIXTURE NEW_DB RESULTS_JSON".into());
    }
    flicknote_core::sqlite_extension::register_better_trigram()?;
    let mut source = std::fs::File::open(&args[1])?;
    let mut output = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&args[3])?;
    io::copy(&mut source, &mut output)?;
    drop(output);
    let before = std::fs::metadata(&args[3])?.len();
    let db = Connection::open(&args[3])?;
    let build = Instant::now();
    fts_search::install(&db)?;
    let build_ms = build.elapsed().as_secs_f64() * 1000.0;
    let after = std::fs::metadata(&args[3])?.len();
    let indexed_count: i64 =
        db.query_row("SELECT count(*) FROM note_search_fts", [], |r| r.get(0))?;
    drop(db);
    let reopen = Instant::now();
    let mut db = Connection::open(&args[3])?;
    let _: i64 = db.query_row("SELECT count(*) FROM note_search_fts", [], |r| r.get(0))?;
    let reopen_ms = reopen.elapsed().as_secs_f64() * 1000.0;
    let fixture: Value = serde_json::from_str(&std::fs::read_to_string(&args[2])?)?;
    let mut results = Vec::new();
    for entry in fixture["queries"]
        .as_array()
        .ok_or("queries array missing")?
    {
        let terms = entry["terms"]
            .as_array()
            .ok_or("terms missing")?
            .iter()
            .map(|x| x.as_str().unwrap_or("").to_owned())
            .collect::<Vec<_>>();
        let project = entry["project"].as_str();
        let query_started = Instant::now();
        let ids = fts_search::search_ids(&db, &terms, project, 100);
        let query_ms = query_started.elapsed().as_secs_f64() * 1000.0;
        let started = Instant::now();
        let result = fts_search::search(&db, &terms, project, 100);
        let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
        let relevant = entry["relevantIds"]
            .as_array()
            .ok_or("relevantIds missing")?;
        match (ids, result) {
            (Ok(_), Ok(hits)) => {
                let rank = hits
                    .iter()
                    .position(|hit| relevant.iter().any(|id| id.as_i64() == hit.short_id));
                results.push(json!({"id":entry["id"],"rank":rank.map(|x|x+1),
                    "queryMs":query_ms,"elapsedMs":elapsed_ms,"hits":hits.len(),"topIds":hits.iter().take(10).map(|h|h.short_id).collect::<Vec<_>>() }));
            }
            (Err(error), _) | (_, Err(error)) => results
                .push(json!({"id":entry["id"],"error":error.to_string(),"queryMs":query_ms,"elapsedMs":elapsed_ms})),
        }
    }
    let updated = Instant::now();
    db.execute(
        "INSERT INTO ps_data__notes(id,data) VALUES (?1,?2)",
        rusqlite::params![
            "fts-spike-single",
            r#"{"short_id":-1,"title":"spike","content":"violet visibility"}"#
        ],
    )?;
    db.execute(
        "UPDATE ps_data__notes SET data=?2 WHERE id=?1",
        rusqlite::params![
            "fts-spike-single",
            r#"{"short_id":-1,"title":"spike","content":"amber visibility"}"#
        ],
    )?;
    let visible: i64 = db.query_row(
        "SELECT count(*) FROM note_search_fts WHERE note_search_fts MATCH 'amber' AND uuid='fts-spike-single'",
        [],
        |r| r.get(0),
    )?;
    if visible != 1 {
        let content: String = db.query_row(
            "SELECT content FROM note_search_fts WHERE uuid='fts-spike-single'",
            [],
            |row| row.get(0),
        )?;
        return Err(format!("updated note not visible: {content}").into());
    }
    let update_ms = updated.elapsed().as_secs_f64() * 1000.0;
    let archived = Instant::now();
    db.execute("UPDATE ps_data__notes SET data=?2 WHERE id=?1",
        rusqlite::params!["fts-spike-single", r#"{"short_id":-1,"title":"spike","content":"amber visibility","deleted_at":"2026-01-01"}"#])?;
    let visible: i64 = db.query_row(
        "SELECT count(*) FROM note_search_fts WHERE note_search_fts MATCH 'amber' AND uuid='fts-spike-single'",
        [],
        |r| r.get(0),
    )?;
    if visible != 0 {
        return Err("archived note remains visible".into());
    }
    let archive_ms = archived.elapsed().as_secs_f64() * 1000.0;
    let burst100_ms = burst(&mut db, 100)?;
    let burst1000_ms = burst(&mut db, 1000)?;
    let report = json!({"engine":"fts5-better-trigram-coverage", "snapshotBytes":before,
        "indexedBytes":after,"growthBytes":after-before,"indexedNotes":indexed_count,
        "buildMs":build_ms,"reopenMs":reopen_ms,"updateMs":update_ms,
        "archiveMs":archive_ms,"burst100Ms":burst100_ms,"burst1000Ms":burst1000_ms,
        "results":results});
    std::fs::write(&args[4], serde_json::to_vec_pretty(&report)?)?;
    println!(
        "notes={indexed_count} build_ms={build_ms:.1} reopen_ms={reopen_ms:.1} growth_mib={:.1} queries={}",
        (after - before) as f64 / 1048576.0,
        results.len()
    );
    Ok(())
}

fn burst(db: &mut Connection, count: usize) -> Result<f64, Box<dyn std::error::Error>> {
    let started = Instant::now();
    let transaction = db.transaction()?;
    {
        let mut insert =
            transaction.prepare("INSERT INTO ps_data__notes(id,data) VALUES (?1,?2)")?;
        for index in 0..count {
            insert.execute(rusqlite::params![
                format!("fts-spike-burst-{count}-{index}"),
                r#"{"title":"burst fixture","content":"synthetic searchable text"}"#
            ])?;
        }
    }
    transaction.commit()?;
    Ok(started.elapsed().as_secs_f64() * 1000.0)
}
