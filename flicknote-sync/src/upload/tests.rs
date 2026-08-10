use std::sync::atomic::{AtomicUsize, Ordering};

use flicknote_core::{REMOTE_COMMITTED_INSERT_METADATA, schema::app_schema};
use rusqlite::params;

use super::*;
use crate::test_support::*;

#[tokio::test]
async fn failed_upload_is_retried_without_a_second_write_trigger() {
    let attempts = Arc::new(AtomicUsize::new(0));
    let attempt_counter = Arc::clone(&attempts);

    retry_with_backoff(
        move || {
            let attempt_counter = Arc::clone(&attempt_counter);
            async move { attempt_counter.fetch_add(1, Ordering::SeqCst) > 0 }
        },
        std::time::Duration::from_millis(1),
        std::time::Duration::from_millis(2),
    )
    .await;

    assert_eq!(attempts.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn remote_committed_insert_records_marker_in_crud() {
    let (_directory, db) = test_powersync_db().await;
    insert_marked_note(&db).await;

    let transaction = db.next_crud_transaction().await.unwrap().unwrap();
    assert_eq!(transaction.crud.len(), 1);
    assert_eq!(transaction.crud[0].table, "notes");
    assert!(matches!(
        transaction.crud.first().map(|entry| &entry.update_type),
        Some(UpdateType::Put)
    ));
    assert_eq!(
        transaction.crud[0].metadata.as_deref(),
        Some(r#"{"flicknote":"remote_committed_insert_v1"}"#)
    );
}

#[tokio::test]
async fn existing_database_upgrades_to_metadata_tracking_without_losing_rows() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("upgrade.db");
    let mut legacy_schema = app_schema();
    for table in &mut legacy_schema.tables {
        if matches!(table.name.as_ref(), "notes" | "note_extractions") {
            table.options.track_metadata = false;
        }
    }
    {
        let legacy_db = test_powersync_db_at(&path, legacy_schema);
        let writer = legacy_db.writer().await.unwrap();
        writer
            .execute(
                "INSERT INTO notes (id, user_id, type, status, title) VALUES (?, ?, ?, ?, ?)",
                params!["existing-note", "user-1", "normal", "ready", "Preserved"],
            )
            .unwrap();
        writer.execute("DELETE FROM ps_crud", []).unwrap();
    }

    let upgraded_db = test_powersync_db_at(&path, app_schema());
    {
        let writer = upgraded_db.writer().await.unwrap();
        let title: String = writer
            .query_row(
                "SELECT title FROM notes WHERE id = ?",
                params!["existing-note"],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(title, "Preserved");
        writer
            .execute(
                "INSERT INTO notes (id, user_id, type, status, title, _metadata) VALUES (?, ?, ?, ?, ?, ?)",
                params![
                    "marked-after-upgrade",
                    "user-1",
                    "normal",
                    "ready",
                    "Marked",
                    REMOTE_COMMITTED_INSERT_METADATA,
                ],
            )
            .unwrap();
    }

    let transaction = upgraded_db.next_crud_transaction().await.unwrap().unwrap();
    assert_eq!(transaction.crud.len(), 1);
    assert_eq!(transaction.crud[0].id, "marked-after-upgrade");
    assert_eq!(
        transaction.crud[0].metadata.as_deref(),
        Some(REMOTE_COMMITTED_INSERT_METADATA)
    );
}

fn schema_with_retired_keyterms() -> powersync::schema::Schema {
    let mut schema = app_schema();
    let projects = schema
        .tables
        .iter_mut()
        .find(|table| table.name.as_ref() == "projects")
        .unwrap();
    projects
        .columns
        .push(powersync::schema::Column::text("keyterm_id"));
    schema.tables.push(powersync::schema::Table::create(
        "keyterms",
        vec![
            powersync::schema::Column::text("user_id"),
            powersync::schema::Column::text("name"),
            powersync::schema::Column::text("description"),
            powersync::schema::Column::text("content"),
            powersync::schema::Column::text("created_at"),
            powersync::schema::Column::text("updated_at"),
        ],
        |_| {},
    ));
    schema
}

#[tokio::test]
async fn existing_database_retires_keyterm_schema_without_losing_projects() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("keyterm-retirement.db");

    {
        let legacy_db = test_powersync_db_at(&path, schema_with_retired_keyterms());
        let writer = legacy_db.writer().await.unwrap();
        writer
            .execute(
                "INSERT INTO keyterms (id, user_id, name) VALUES (?, ?, ?)",
                params!["retired-keyterm", "user-1", "Retired"],
            )
            .unwrap();
        writer
            .execute(
                "INSERT INTO projects (id, user_id, name, keyterm_id) VALUES (?, ?, ?, ?)",
                params![
                    "preserved-project",
                    "user-1",
                    "Preserved",
                    "retired-keyterm"
                ],
            )
            .unwrap();
    }

    let upgraded_db = test_powersync_db_at(&path, app_schema());
    {
        let writer = upgraded_db.writer().await.unwrap();
        let project_name: String = writer
            .query_row(
                "SELECT name FROM projects WHERE id = ?",
                params!["preserved-project"],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(project_name, "Preserved");
        let retired_view_count: i64 = writer
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'view' AND name = 'keyterms'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(retired_view_count, 0);
        let retired_column_count: i64 = writer
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('projects') WHERE name = 'keyterm_id'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(retired_column_count, 0);
    }

    let (server_url, server) = spawn_capture_server(1);
    assert!(
        run_upload(
            &upgraded_db,
            &reqwest::Client::new(),
            "token",
            &server_url,
            "anon-key",
        )
        .await
        .unwrap()
    );
    assert!(upgraded_db.next_crud_transaction().await.unwrap().is_none());
    let requests = server.join().unwrap();
    assert_eq!(requests.len(), 1);
    assert!(requests[0].starts_with("POST /rest/v1/projects "));
    let (_, body) = requests[0].split_once("\r\n\r\n").unwrap();
    let payload: serde_json::Value = serde_json::from_str(body).unwrap();
    assert_eq!(payload["name"], "Preserved");
    assert!(payload.get("keyterm_id").is_none());
}

#[tokio::test]
async fn remote_committed_put_completes_without_http_request() {
    let (_directory, db) = test_powersync_db().await;
    insert_marked_note(&db).await;

    let uploaded = run_upload(
        &db,
        &reqwest::Client::new(),
        "token",
        "http://127.0.0.1:1",
        "anon-key",
    )
    .await
    .unwrap();

    assert!(uploaded);
    assert!(db.next_crud_transaction().await.unwrap().is_none());
}

#[tokio::test]
async fn remote_committed_marker_is_matched_as_json_not_raw_text() {
    let (_directory, db) = test_powersync_db().await;
    insert_note_with_metadata(&db, r#"{ "flicknote" : "remote_committed_insert_v1" }"#).await;

    run_upload(
        &db,
        &reqwest::Client::new(),
        "token",
        "http://127.0.0.1:1",
        "anon-key",
    )
    .await
    .unwrap();

    assert!(db.next_crud_transaction().await.unwrap().is_none());
}

#[tokio::test]
async fn remote_committed_marker_rejects_extra_metadata_fields() {
    let (_directory, db) = test_powersync_db().await;
    insert_note_with_metadata(
        &db,
        r#"{"flicknote":"remote_committed_insert_v1","other":true}"#,
    )
    .await;

    let error = run_upload(
        &db,
        &reqwest::Client::new(),
        "token",
        "http://127.0.0.1:1",
        "anon",
    )
    .await
    .unwrap_err();

    assert!(
        error
            .to_string()
            .contains("invalid FlickNote CRUD metadata")
    );
    assert!(db.crud_transactions().try_next().await.unwrap().is_some());
}

#[tokio::test]
async fn unsupported_flicknote_marker_is_rejected_and_retained() {
    let (_directory, db) = test_powersync_db().await;
    insert_note_with_metadata(&db, r#"{"flicknote":"remote_committed_insert_v2"}"#).await;

    let error = run_upload(
        &db,
        &reqwest::Client::new(),
        "token",
        "http://127.0.0.1:1",
        "anon-key",
    )
    .await
    .unwrap_err();

    assert!(
        error
            .to_string()
            .contains("unsupported FlickNote CRUD marker")
    );
    assert!(db.next_crud_transaction().await.unwrap().is_some());
}

#[tokio::test]
async fn malformed_crud_metadata_is_rejected_and_retained() {
    let (_directory, db) = test_powersync_db().await;
    insert_note_with_metadata(&db, r#"{"flicknote":"remote_committed_insert_v1""#).await;

    let error = run_upload(
        &db,
        &reqwest::Client::new(),
        "token",
        "http://127.0.0.1:1",
        "anon-key",
    )
    .await
    .unwrap_err();

    assert!(error.to_string().contains("invalid CRUD metadata"));
    assert!(db.next_crud_transaction().await.unwrap().is_some());
}

#[tokio::test]
async fn remote_committed_marker_on_patch_is_rejected_and_retained() {
    let (_directory, db) = test_powersync_db().await;
    insert_marked_note(&db).await;
    db.next_crud_transaction()
        .await
        .unwrap()
        .unwrap()
        .complete()
        .await
        .unwrap();
    {
        let writer = db.writer().await.unwrap();
        writer
            .execute(
                "UPDATE notes SET title = ?, _metadata = ? WHERE id = ?",
                params!["Changed", REMOTE_COMMITTED_INSERT_METADATA, "note-1"],
            )
            .unwrap();
    }

    let error = run_upload(
        &db,
        &reqwest::Client::new(),
        "token",
        "http://127.0.0.1:1",
        "anon-key",
    )
    .await
    .unwrap_err();

    assert!(
        error
            .to_string()
            .contains("invalid remote-committed marker")
    );
    assert!(db.next_crud_transaction().await.unwrap().is_some());
}

#[test]
fn test_extract_fatal_code_fk_violation() {
    let body = r#"{"code":"23503","details":"Key is not present in table \"projects\".","hint":null,"message":"insert or update on table \"notes\" violates foreign key constraint"}"#;
    assert_eq!(extract_fatal_code(body), Some("23503".to_string()));
}

#[test]
fn test_extract_fatal_code_rls_violation() {
    let body = r#"{"code":"42501","message":"new row violates row-level security policy"}"#;
    assert_eq!(extract_fatal_code(body), Some("42501".to_string()));
}

#[test]
fn test_extract_fatal_code_transient() {
    let body = r#"{"code":"08006","message":"connection failure"}"#;
    assert_eq!(extract_fatal_code(body), None);
}

#[test]
fn test_extract_fatal_code_not_json() {
    assert_eq!(extract_fatal_code("Internal Server Error"), None);
}

#[test]
fn test_extract_fatal_code_postgrest() {
    let body = r#"{"code":"PGRST204","message":"column not found"}"#;
    assert_eq!(extract_fatal_code(body), Some("PGRST204".to_string()));
}

#[test]
fn test_extract_fatal_code_class22_data_exception() {
    let body = r#"{"code":"22001","message":"value too long for type character varying(255)"}"#;
    assert_eq!(extract_fatal_code(body), Some("22001".to_string()));
}

#[test]
fn test_extract_fatal_code_missing_code_field() {
    // Supabase auth-layer errors omit "code" — should be treated as unknown (transient)
    let body = r#"{"error":"invalid_grant","error_description":"Refresh Token Not Found"}"#;
    assert_eq!(extract_fatal_code(body), None);
}

#[test]
fn test_unwrap_json_strings() {
    let mut data = serde_json::Map::new();
    data.insert("title".into(), serde_json::Value::String("Hello".into()));
    data.insert(
        "metadata".into(),
        serde_json::Value::String(r#"{"file":{"name":"photo.jpg"}}"#.into()),
    );
    data.insert(
        "tags".into(),
        serde_json::Value::String(r#"["rust","cli"]"#.into()),
    );
    // Primitive JSON values ("42", "true") must stay as strings — guard is is_object()||is_array().
    data.insert("count".into(), serde_json::Value::String("42".into()));
    data.insert("flag".into(), serde_json::Value::String("true".into()));
    data.insert("source".into(), serde_json::Value::Null);
    unwrap_json_strings(&mut data);
    assert_eq!(data["title"], serde_json::Value::String("Hello".into())); // plain string unchanged
    assert!(data["metadata"].is_object()); // JSON object string → Value::Object
    assert!(data["tags"].is_array()); // JSON array string → Value::Array
    assert_eq!(data["count"], serde_json::Value::String("42".into())); // primitive JSON unchanged
    assert_eq!(data["flag"], serde_json::Value::String("true".into())); // primitive JSON unchanged
    assert!(data["source"].is_null()); // null unchanged
}
