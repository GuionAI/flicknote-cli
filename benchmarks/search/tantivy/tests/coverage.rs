use rusqlite::Connection;
use serde_json::{Value, json};
use std::process::Command;

#[test]
fn coverage_prefers_fields_and_words_then_recency_and_rejects_scattered_grams() {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("snapshot.db");
    let fixture = temp.path().join("queries.json");
    let output = temp.path().join("results.json");
    Connection::open(&db)
        .unwrap()
        .execute_batch(
            "CREATE TABLE projects(id INTEGER, name TEXT);
         INSERT INTO projects VALUES(1, 'alpha'), (2, 'beta');
         CREATE TABLE notes(short_id INTEGER, title TEXT, summary TEXT, content TEXT,
                            project_id INTEGER, deleted_at TEXT, updated_at TEXT);
         INSERT INTO notes VALUES
           (1, 'red chair', '', '', 1, NULL, '2026-01-01'),
           (2, 'red', '', 'chair chair chair chair', 1, NULL, '2026-03-01'),
           (3, '', '', 'red chair red chair red chair', 1, NULL, '2026-04-01'),
           (4, 'red chair', '', '', 1, NULL, '2026-02-01'),
           (5, 'abc bcd', '', '', 1, NULL, '2026-05-01'),
           (6, 'abcd', '', '', 1, NULL, '2026-01-01'),
           (7, 'red chair', '', '', 2, NULL, '2026-06-01'),
           (8, '索引同步', '', '', 1, NULL, '2026-01-01');",
        )
        .unwrap();
    let connection = Connection::open(&db).unwrap();
    for id in 1000..1200 {
        connection
            .execute(
                "INSERT INTO notes VALUES (?1, 'window', '', '', 1, NULL, '2026-01-01')",
                [id],
            )
            .unwrap();
    }
    connection
        .execute_batch("INSERT INTO notes VALUES (999, 'window', '', '', 1, NULL, '2027-01-01')")
        .unwrap();
    drop(connection);
    let cases = [
        ("coverage", vec!["red", "chair"], vec![4, 1, 2, 3]),
        ("continuity", vec!["abcd"], vec![6]),
        ("or", vec!["red", "nonexistent"], vec![2, 4, 1, 3]),
        ("chinese", vec!["索引"], vec![8]),
        (
            "single-upper",
            vec!["W"],
            vec![999, 1199, 1198, 1197, 1196, 1195, 1194, 1193, 1192, 1191],
        ),
        (
            "bounded-recency",
            vec!["window"],
            vec![999, 1199, 1198, 1197, 1196, 1195, 1194, 1193, 1192, 1191],
        ),
    ];
    let queries: Vec<_> = cases
        .iter()
        .map(|(id, terms, expected)| {
            json!({
                "id":id,"terms":terms,"relevantIds":expected,"project":"alpha",
                "audience":"human","group":"synthetic"
            })
        })
        .collect();
    std::fs::write(
        &fixture,
        serde_json::to_vec(&json!({"mode":"coverage","candidateLimit":10,"queries":queries}))
            .unwrap(),
    )
    .unwrap();
    let child = Command::new(env!("CARGO_BIN_EXE_flicknote-search-tantivy-bench"))
        .args([&db, &fixture, &output])
        .output()
        .unwrap();
    assert!(
        child.status.success(),
        "{}",
        String::from_utf8_lossy(&child.stderr)
    );
    let report: Value = serde_json::from_slice(&std::fs::read(output).unwrap()).unwrap();
    for (row, (id, _, expected)) in report["results"].as_array().unwrap().iter().zip(cases) {
        assert_eq!(row["hits"], json!(expected), "{id}");
        assert!(
            row["snippets"]
                .as_array()
                .unwrap()
                .iter()
                .all(|s| s.as_str().is_some_and(|s| s.contains("<b>")))
        );
    }
}
