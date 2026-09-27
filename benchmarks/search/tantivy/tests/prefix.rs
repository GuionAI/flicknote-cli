use std::process::Command;

use rusqlite::Connection;
use serde_json::{Value, json};

#[test]
fn native_prefix_respects_words_filters_and_highlights() {
    let temp = tempfile::tempdir().unwrap();
    let snapshot = temp.path().join("snapshot.db");
    let fixture = temp.path().join("queries.json");
    let output = temp.path().join("results.json");
    let database = Connection::open(&snapshot).unwrap();
    database
        .execute_batch(
            "CREATE TABLE projects (id INTEGER, name TEXT);
         INSERT INTO projects VALUES (1, 'alpha'), (2, 'beta');
         CREATE TABLE notes (short_id INTEGER, title TEXT, summary TEXT, content TEXT,
                             project_id INTEGER, deleted_at TEXT);
         INSERT INTO notes VALUES
           (1, 'Meilisearch', 'search engine', '', 1, NULL),
           (2, 'xmeilisearch', '', '', 1, NULL),
           (3, 'meili', '', '', 1, NULL),
           (4, 'meilxsearch', '', '', 1, NULL),
           (5, 'search', '', '', 1, NULL),
           (6, '索引同步', '', '', 1, NULL),
           (7, 'Meilisearch', '', '', 2, NULL);",
        )
        .unwrap();
    database
        .execute_batch("ALTER TABLE notes ADD COLUMN updated_at TEXT")
        .unwrap();
    drop(database);
    let cases = [
        ("word", vec!["meilis"], None, vec![1, 7]),
        ("case", vec!["MEILIS"], None, vec![1, 7]),
        ("single", vec!["m"], None, vec![1, 3, 4, 7]),
        ("last-required", vec!["search", "meilis"], None, vec![1, 7]),
        ("project", vec!["meilis"], Some("alpha"), vec![1]),
        ("no-fuzzy", vec!["meilisa"], None, vec![]),
        ("cjk", vec!["索引"], None, vec![6]),
        ("cjk-single", vec!["索"], None, vec![]),
        ("punctuation", vec!["search-meilis"], None, vec![1]),
    ];
    let queries: Vec<Value> = cases
        .iter()
        .map(|(id, terms, project, ids)| {
            json!({
                "id": id, "terms": terms, "project": project, "relevantIds": ids,
                "audience": "human", "group": "synthetic"
            })
        })
        .collect();
    std::fs::write(
        &fixture,
        serde_json::to_vec(&json!({
            "queries": queries, "mode": "hybridPrefix"
        }))
        .unwrap(),
    )
    .unwrap();
    let child = Command::new(env!("CARGO_BIN_EXE_flicknote-search-tantivy-bench"))
        .args([&snapshot, &fixture, &output])
        .output()
        .unwrap();
    assert!(
        child.status.success(),
        "{}",
        String::from_utf8_lossy(&child.stderr)
    );
    let report: Value = serde_json::from_slice(&std::fs::read(output).unwrap()).unwrap();
    assert_eq!(report["mode"], "hybridPrefix");
    for (row, (id, _, _, expected)) in report["results"].as_array().unwrap().iter().zip(cases) {
        let mut actual: Vec<u64> = serde_json::from_value(row["hits"].clone()).unwrap();
        actual.sort_unstable();
        assert_eq!(actual, expected, "query {id}");
        for snippet in row["snippets"].as_array().unwrap() {
            assert!(
                snippet.as_str().unwrap().contains("<b>"),
                "query {id}: {snippet}"
            );
        }
    }
}
