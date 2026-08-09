use super::*;
use crate::test_support::*;

#[tokio::test]
async fn remote_committed_note_is_fully_visible_before_return() {
    let (_directory, db) = test_powersync_db().await;
    let inserted = commit_remote_note(&db, &remote_note("note-full", "Remote title"))
        .await
        .unwrap();

    assert!(inserted);
    let reader = db.reader().await.unwrap();
    let row = reader
        .query_row(
            "SELECT short_id, title, summary, metadata, source FROM notes WHERE id = ?",
            params!["note-full"],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                ))
            },
        )
        .unwrap();
    assert_eq!(row.0, 42);
    assert_eq!(row.1, "Remote title");
    assert_eq!(row.2, "Canonical summary");
    assert_eq!(row.3, r#"{"source":"remote"}"#);
    assert_eq!(row.4, r#"{"kind":"plain"}"#);

    let transaction = db.next_crud_transaction().await.unwrap().unwrap();
    assert_eq!(
        transaction.crud[0].metadata.as_deref(),
        Some(REMOTE_COMMITTED_INSERT_METADATA)
    );
}

#[tokio::test]
async fn remote_committed_note_does_not_replace_row_downloaded_first() {
    let (_directory, db) = test_powersync_db().await;
    {
        let writer = db.writer().await.unwrap();
        writer
            .execute(
                "INSERT INTO notes (id, short_id, user_id, type, status, title) VALUES (?, ?, ?, ?, ?, ?)",
                params!["note-race", 42, "user-1", "normal", "ready", "Newer title"],
            )
            .unwrap();
        writer.execute("DELETE FROM ps_crud", []).unwrap();
    }

    let inserted = commit_remote_note(&db, &remote_note("note-race", "Older title"))
        .await
        .unwrap();

    assert!(!inserted);
    let reader = db.reader().await.unwrap();
    let title: String = reader
        .query_row(
            "SELECT title FROM notes WHERE id = ?",
            params!["note-race"],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(title, "Newer title");
    assert!(db.next_crud_transaction().await.unwrap().is_none());
}

#[tokio::test]
async fn remote_committed_extractions_are_visible_before_return() {
    let (_directory, db) = test_powersync_db().await;
    let rows = vec![RemoteExtractionRow {
        id: "extraction-1".to_string(),
        note_id: "note-1".to_string(),
        user_id: "user-1".to_string(),
        key: TOPIC_EXTRACTION_KEY.to_string(),
        value: "rust".to_string(),
    }];

    let inserted = commit_remote_extractions(&db, &rows).await.unwrap();

    assert_eq!(inserted, 1);
    let reader = db.reader().await.unwrap();
    let value: String = reader
        .query_row(
            "SELECT value FROM note_extractions WHERE id = ?",
            params!["extraction-1"],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(value, "rust");
    let transaction = db.next_crud_transaction().await.unwrap().unwrap();
    assert_eq!(
        transaction.crud[0].metadata.as_deref(),
        Some(REMOTE_COMMITTED_INSERT_METADATA)
    );
}

#[test]
fn partial_remote_create_maps_to_non_retryable_structured_service_error() {
    let error = remote_create_service_error(DaemonError::PartialCreate {
        message: "note created; topics pending".to_string(),
        note_id: "note-partial".to_string(),
        short_id: Some(80),
        confirmed_extraction_ids: vec!["extraction-confirmed".to_string()],
        pending_extraction_ids: vec!["extraction-1".to_string()],
    });

    assert_eq!(error.code(), "note_create_partial");
    assert!(!error.retryable());
    let flicknote_core::services::error::ServiceError::Remote { details, .. } = error else {
        panic!("expected remote service error")
    };
    let details = details.unwrap();
    assert_eq!(details["short_id"], 80);
    assert_eq!(
        details["confirmed_extraction_ids"],
        serde_json::json!(["extraction-confirmed"])
    );
}

#[tokio::test]
async fn remote_create_returns_after_canonical_note_is_committed_locally() {
    let body = r#"[{"id":"note-create","short_id":77,"user_id":"user-1","type":"normal","status":"ai_queued","title":"Remote title","content":"Body","summary":null,"is_flagged":false,"project_id":null,"metadata":null,"source":null,"created_at":"2026-08-09T00:00:00Z","updated_at":"2026-08-09T00:00:00Z","deleted_at":null}]"#;
    let (origin, server) = spawn_server(vec![("201 Created", body)]);
    let mut config = test_config(String::new());
    config.supabase_url = origin;
    config.supabase_anon_key = "anon-key".to_string();
    let (_directory, db) = test_powersync_db().await;
    let request = CreateNoteRequest {
        id: "note-create".to_string(),
        note_type: "normal".to_string(),
        status: "ai_queued".to_string(),
        title: Some("Requested title".to_string()),
        content: Some("Body".to_string()),
        metadata: None,
        project_id: None,
        now: "2026-08-09T00:00:00Z".to_string(),
        topics: Vec::new(),
        attachment_path: None,
    };

    let created = create_note_with_token(
        &db,
        &reqwest::Client::new(),
        &config,
        "access-token",
        "user-1",
        request,
    )
    .await
    .unwrap();

    assert_eq!(created.uuid, "note-create");
    assert_eq!(created.short_id, 77);
    let reader = db.reader().await.unwrap();
    let title: String = reader
        .query_row(
            "SELECT title FROM notes WHERE id = ?",
            params!["note-create"],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(title, "Remote title");
    assert_eq!(
        server.join().unwrap(),
        ["POST /rest/v1/notes?on_conflict=id HTTP/1.1"]
    );
}

#[tokio::test]
async fn remote_create_reports_typed_partial_success_after_note_commit() {
    let note = r#"[{"id":"note-partial","short_id":80,"user_id":"user-1","type":"normal","status":"ai_queued","title":"Remote title","content":"Body","summary":null,"is_flagged":false,"project_id":null,"metadata":null,"source":null,"created_at":"2026-08-09T00:00:00Z","updated_at":"2026-08-09T00:00:00Z","deleted_at":null}]"#;
    let (origin, server) = spawn_server(vec![
        ("201 Created", note),
        (
            "500 Internal Server Error",
            r#"{"message":"topic failure"}"#,
        ),
    ]);
    let mut config = test_config(String::new());
    config.supabase_url = origin;
    config.supabase_anon_key = "anon-key".to_string();
    let (_directory, db) = test_powersync_db().await;

    let error = create_note_with_token(
        &db,
        &reqwest::Client::new(),
        &config,
        "access-token",
        "user-1",
        CreateNoteRequest {
            id: "note-partial".to_string(),
            note_type: "normal".to_string(),
            status: "ai_queued".to_string(),
            title: Some("Requested title".to_string()),
            content: Some("Body".to_string()),
            metadata: None,
            project_id: None,
            now: "2026-08-09T00:00:00Z".to_string(),
            topics: vec!["rust".to_string()],
            attachment_path: None,
        },
    )
    .await
    .unwrap_err();

    let DaemonError::PartialCreate {
        note_id,
        short_id,
        pending_extraction_ids,
        ..
    } = error
    else {
        panic!("expected partial create error")
    };
    assert_eq!(note_id, "note-partial");
    assert_eq!(short_id, Some(80));
    assert_eq!(pending_extraction_ids.len(), 1);
    let reader = db.reader().await.unwrap();
    let count: i64 = reader
        .query_row(
            "SELECT COUNT(*) FROM notes WHERE id = ?",
            params!["note-partial"],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count, 1);
    assert_eq!(server.join().unwrap().len(), 2);
}

#[tokio::test]
async fn remote_create_recovers_empty_idempotent_response_by_stable_uuid() {
    let body = r#"[{"id":"note-retry","short_id":78,"user_id":"user-1","type":"normal","status":"ai_queued","title":"Recovered","content":"Body","summary":null,"is_flagged":false,"project_id":null,"metadata":null,"source":null,"created_at":"2026-08-09T00:00:00Z","updated_at":"2026-08-09T00:00:00Z","deleted_at":null}]"#;
    let (origin, server) = spawn_server(vec![("200 OK", "[]"), ("200 OK", body)]);
    let mut config = test_config(String::new());
    config.supabase_url = origin;
    config.supabase_anon_key = "anon-key".to_string();
    let (_directory, db) = test_powersync_db().await;
    let request = CreateNoteRequest {
        id: "note-retry".to_string(),
        note_type: "normal".to_string(),
        status: "ai_queued".to_string(),
        title: Some("Requested".to_string()),
        content: Some("Body".to_string()),
        metadata: None,
        project_id: None,
        now: "2026-08-09T00:00:00Z".to_string(),
        topics: Vec::new(),
        attachment_path: None,
    };

    let created = create_note_with_token(
        &db,
        &reqwest::Client::new(),
        &config,
        "access-token",
        "user-1",
        request,
    )
    .await
    .unwrap();

    assert_eq!(created.short_id, 78);
    assert_eq!(
        server.join().unwrap(),
        [
            "POST /rest/v1/notes?on_conflict=id HTTP/1.1",
            "GET /rest/v1/notes?id=eq.note-retry&select=* HTTP/1.1",
        ]
    );
}

#[tokio::test]
async fn remote_create_recovers_malformed_success_response_by_stable_uuid() {
    let body = r#"[{"id":"note-malformed","short_id":81,"user_id":"user-1","type":"normal","status":"ai_queued","title":"Recovered","content":"Body","summary":null,"is_flagged":false,"project_id":null,"metadata":null,"source":null,"created_at":"2026-08-09T00:00:00Z","updated_at":"2026-08-09T00:00:00Z","deleted_at":null}]"#;
    let (origin, server) = spawn_server(vec![("201 Created", "{"), ("200 OK", body)]);
    let mut config = test_config(String::new());
    config.supabase_url = origin;
    config.supabase_anon_key = "anon-key".to_string();
    let (_directory, db) = test_powersync_db().await;

    let created = create_note_with_token(
        &db,
        &reqwest::Client::new(),
        &config,
        "access-token",
        "user-1",
        CreateNoteRequest {
            id: "note-malformed".to_string(),
            note_type: "normal".to_string(),
            status: "ai_queued".to_string(),
            title: Some("Requested".to_string()),
            content: Some("Body".to_string()),
            metadata: None,
            project_id: None,
            now: "2026-08-09T00:00:00Z".to_string(),
            topics: Vec::new(),
            attachment_path: None,
        },
    )
    .await
    .unwrap();

    assert_eq!(created.short_id, 81);
    assert_eq!(
        server.join().unwrap(),
        [
            "POST /rest/v1/notes?on_conflict=id HTTP/1.1",
            "GET /rest/v1/notes?id=eq.note-malformed&select=* HTTP/1.1",
        ]
    );
}

#[tokio::test]
async fn malformed_success_with_failed_reconciliation_reports_confirmed_create() {
    let (origin, server) = spawn_server(vec![
        ("201 Created", "{"),
        ("503 Service Unavailable", r#"{"message":"try later"}"#),
    ]);
    let mut config = test_config(String::new());
    config.supabase_url = origin;
    config.supabase_anon_key = "anon-key".to_string();
    let (_directory, db) = test_powersync_db().await;

    let error = create_note_with_token(
        &db,
        &reqwest::Client::new(),
        &config,
        "access-token",
        "user-1",
        CreateNoteRequest {
            id: "note-confirmed".to_string(),
            note_type: "normal".to_string(),
            status: "ai_queued".to_string(),
            title: Some("Requested".to_string()),
            content: Some("Body".to_string()),
            metadata: None,
            project_id: None,
            now: "2026-08-09T00:00:00Z".to_string(),
            topics: Vec::new(),
            attachment_path: None,
        },
    )
    .await
    .unwrap_err();
    let service_error = remote_create_service_error(error);

    assert_eq!(service_error.code(), "note_create_partial");
    let flicknote_core::services::error::ServiceError::Remote { details, .. } = service_error
    else {
        panic!("expected structured remote error")
    };
    let details = details.unwrap();
    assert_eq!(details["created"], true);
    assert_eq!(details["note_id"], "note-confirmed");
    assert!(details["short_id"].is_null());
    assert_eq!(server.join().unwrap().len(), 2);
}

#[tokio::test]
async fn local_commit_failure_after_remote_create_reports_partial_success() {
    let note = r#"[{"id":"note-local-failure","short_id":82,"user_id":"user-1","type":"normal","status":"ai_queued","title":"Remote title","content":"Body","summary":null,"is_flagged":false,"project_id":null,"metadata":null,"source":null,"created_at":"2026-08-09T00:00:00Z","updated_at":"2026-08-09T00:00:00Z","deleted_at":null}]"#;
    let (origin, server) = spawn_server(vec![("201 Created", note)]);
    let mut config = test_config(String::new());
    config.supabase_url = origin;
    config.supabase_anon_key = "anon-key".to_string();
    let (_directory, db) = test_powersync_db().await;
    db.writer()
        .await
        .unwrap()
        .execute("DROP VIEW notes", [])
        .unwrap();

    let error = create_note_with_token(
        &db,
        &reqwest::Client::new(),
        &config,
        "access-token",
        "user-1",
        CreateNoteRequest {
            id: "note-local-failure".to_string(),
            note_type: "normal".to_string(),
            status: "ai_queued".to_string(),
            title: Some("Requested".to_string()),
            content: Some("Body".to_string()),
            metadata: None,
            project_id: None,
            now: "2026-08-09T00:00:00Z".to_string(),
            topics: Vec::new(),
            attachment_path: None,
        },
    )
    .await
    .unwrap_err();
    let service_error = remote_create_service_error(error);

    assert_eq!(service_error.code(), "note_create_partial");
    let flicknote_core::services::error::ServiceError::Remote { details, .. } = service_error
    else {
        panic!("expected structured remote error")
    };
    let details = details.unwrap();
    assert_eq!(details["created"], true);
    assert_eq!(details["note_id"], "note-local-failure");
    assert_eq!(details["short_id"], 82);
    assert_eq!(server.join().unwrap().len(), 1);
}

#[tokio::test]
async fn remote_create_recovers_lost_response_by_stable_uuid() {
    let body = r#"[{"id":"note-lost","short_id":79,"user_id":"user-1","type":"normal","status":"ai_queued","title":"Recovered","content":"Body","summary":null,"is_flagged":false,"project_id":null,"metadata":null,"source":null,"created_at":"2026-08-09T00:00:00Z","updated_at":"2026-08-09T00:00:00Z","deleted_at":null}]"#;
    let (origin, server) = spawn_disconnected_response_then_server("200 OK", body);
    let mut config = test_config(String::new());
    config.supabase_url = origin;
    config.supabase_anon_key = "anon-key".to_string();
    let (_directory, db) = test_powersync_db().await;
    let request = CreateNoteRequest {
        id: "note-lost".to_string(),
        note_type: "normal".to_string(),
        status: "ai_queued".to_string(),
        title: Some("Requested".to_string()),
        content: Some("Body".to_string()),
        metadata: None,
        project_id: None,
        now: "2026-08-09T00:00:00Z".to_string(),
        topics: Vec::new(),
        attachment_path: None,
    };

    let created = create_note_with_token(
        &db,
        &reqwest::Client::new(),
        &config,
        "access-token",
        "user-1",
        request,
    )
    .await
    .unwrap();

    assert_eq!(created.short_id, 79);
    assert_eq!(server.join().unwrap().len(), 2);
}

#[tokio::test]
async fn ambiguous_transport_failure_reports_stable_unknown_outcome() {
    let (origin, server) = spawn_disconnected_response_then_server(
        "503 Service Unavailable",
        r#"{"message":"try later"}"#,
    );
    let mut config = test_config(String::new());
    config.supabase_url = origin;
    config.supabase_anon_key = "anon-key".to_string();
    let (_directory, db) = test_powersync_db().await;

    let error = create_note_with_token(
        &db,
        &reqwest::Client::new(),
        &config,
        "access-token",
        "user-1",
        CreateNoteRequest {
            id: "note-unknown".to_string(),
            note_type: "normal".to_string(),
            status: "ai_queued".to_string(),
            title: Some("Requested".to_string()),
            content: Some("Body".to_string()),
            metadata: None,
            project_id: None,
            now: "2026-08-09T00:00:00Z".to_string(),
            topics: Vec::new(),
            attachment_path: None,
        },
    )
    .await
    .unwrap_err();
    let service_error = remote_create_service_error(error);

    assert_eq!(service_error.code(), "note_create_unknown");
    assert!(!service_error.retryable());
    let flicknote_core::services::error::ServiceError::Remote { details, .. } = service_error
    else {
        panic!("expected structured remote error")
    };
    let details = details.unwrap();
    assert!(details["created"].is_null());
    assert_eq!(details["note_id"], "note-unknown");
    assert_eq!(server.join().unwrap().len(), 2);
}

#[tokio::test]
async fn ambiguous_transport_failure_retries_create_with_the_same_stable_uuid() {
    let body = r#"[{"id":"note-recovered-after-retry","short_id":83,"user_id":"user-1","type":"normal","status":"ai_queued","title":"Recovered","content":"Body","summary":null,"is_flagged":false,"project_id":null,"metadata":null,"source":null,"created_at":"2026-08-09T00:00:00Z","updated_at":"2026-08-09T00:00:00Z","deleted_at":null}]"#;
    let (origin, server) = spawn_disconnected_then_retry_responses(vec![("201 Created", body)]);
    let mut config = test_config(String::new());
    config.supabase_url = origin;
    config.supabase_anon_key = "anon-key".to_string();
    let (_directory, db) = test_powersync_db().await;

    let result = create_note_with_token(
        &db,
        &reqwest::Client::new(),
        &config,
        "access-token",
        "user-1",
        CreateNoteRequest {
            id: "note-recovered-after-retry".to_string(),
            note_type: "normal".to_string(),
            status: "ai_queued".to_string(),
            title: Some("Requested".to_string()),
            content: Some("Body".to_string()),
            metadata: None,
            project_id: None,
            now: "2026-08-09T00:00:00Z".to_string(),
            topics: Vec::new(),
            attachment_path: None,
        },
    )
    .await;
    let requests = server.join().unwrap();

    let created = result.unwrap();
    assert_eq!(created.short_id, 83);
    assert_eq!(requests.len(), 2);
    assert!(requests[0].starts_with("POST /rest/v1/notes"));
    assert!(requests[1].starts_with("POST /rest/v1/notes"));
}

#[tokio::test]
async fn retryable_status_retries_create_with_the_same_stable_uuid() {
    let body = r#"[{"id":"note-retryable-status","short_id":84,"user_id":"user-1","type":"normal","status":"ai_queued","title":"Recovered","content":"Body","summary":null,"is_flagged":false,"project_id":null,"metadata":null,"source":null,"created_at":"2026-08-09T00:00:00Z","updated_at":"2026-08-09T00:00:00Z","deleted_at":null}]"#;
    let (origin, server) = spawn_server(vec![
        ("503 Service Unavailable", r#"{"message":"try later"}"#),
        ("201 Created", body),
    ]);
    let mut config = test_config(String::new());
    config.supabase_url = origin;
    config.supabase_anon_key = "anon-key".to_string();
    let (_directory, db) = test_powersync_db().await;

    let created = create_note_with_token(
        &db,
        &reqwest::Client::new(),
        &config,
        "access-token",
        "user-1",
        CreateNoteRequest {
            id: "note-retryable-status".to_string(),
            note_type: "normal".to_string(),
            status: "ai_queued".to_string(),
            title: Some("Requested".to_string()),
            content: Some("Body".to_string()),
            metadata: None,
            project_id: None,
            now: "2026-08-09T00:00:00Z".to_string(),
            topics: Vec::new(),
            attachment_path: None,
        },
    )
    .await
    .unwrap();
    let requests = server.join().unwrap();

    assert_eq!(created.short_id, 84);
    assert_eq!(requests.len(), 2);
    assert!(
        requests
            .iter()
            .all(|request| request.starts_with("POST /rest/v1/notes"))
    );
}

#[tokio::test]
async fn remote_extraction_create_commits_confirmed_rows_locally() {
    let body = r#"[{"id":"extraction-create","note_id":"note-create","user_id":"user-1","key":"::topic","value":"rust"}]"#;
    let (origin, server) = spawn_server(vec![("201 Created", body)]);
    let mut config = test_config(String::new());
    config.supabase_url = origin;
    config.supabase_anon_key = "anon-key".to_string();
    let (_directory, db) = test_powersync_db().await;
    let requested = vec![RemoteExtractionRow {
        id: "extraction-create".to_string(),
        note_id: "note-create".to_string(),
        user_id: "user-1".to_string(),
        key: TOPIC_EXTRACTION_KEY.to_string(),
        value: "rust".to_string(),
    }];

    let outcome = create_extractions_with_token(
        &db,
        &reqwest::Client::new(),
        &config,
        "access-token",
        &requested,
    )
    .await;

    assert_eq!(outcome.confirmed_ids, ["extraction-create"]);
    assert!(outcome.pending_ids.is_empty());
    let reader = db.reader().await.unwrap();
    let count: i64 = reader
        .query_row(
            "SELECT COUNT(*) FROM note_extractions WHERE id = ?",
            params!["extraction-create"],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count, 1);
    assert_eq!(
        server.join().unwrap(),
        ["POST /rest/v1/note_extractions?on_conflict=id HTTP/1.1"]
    );
}

#[tokio::test]
async fn remote_extraction_create_recovers_by_stable_uuid() {
    let body = r#"[{"id":"extraction-retry","note_id":"note-create","user_id":"user-1","key":"::topic","value":"rust"}]"#;
    let (origin, server) = spawn_server(vec![("200 OK", "[]"), ("200 OK", body)]);
    let mut config = test_config(String::new());
    config.supabase_url = origin;
    config.supabase_anon_key = "anon-key".to_string();
    let (_directory, db) = test_powersync_db().await;
    let requested = vec![RemoteExtractionRow {
        id: "extraction-retry".to_string(),
        note_id: "note-create".to_string(),
        user_id: "user-1".to_string(),
        key: TOPIC_EXTRACTION_KEY.to_string(),
        value: "rust".to_string(),
    }];

    let outcome = create_extractions_with_token(
        &db,
        &reqwest::Client::new(),
        &config,
        "access-token",
        &requested,
    )
    .await;

    assert_eq!(outcome.confirmed_ids, ["extraction-retry"]);
    assert!(outcome.pending_ids.is_empty());
    assert_eq!(
        server.join().unwrap(),
        [
            "POST /rest/v1/note_extractions?on_conflict=id HTTP/1.1",
            "GET /rest/v1/note_extractions?id=eq.extraction-retry&select=* HTTP/1.1",
        ]
    );
}

#[tokio::test]
async fn remote_extraction_create_commits_confirmed_subset_and_reports_exact_pending_ids() {
    let body = r#"[{"id":"extraction-confirmed","note_id":"note-create","user_id":"user-1","key":"::topic","value":"rust"}]"#;
    let (origin, server) = spawn_server(vec![("201 Created", body), ("200 OK", "[]")]);
    let mut config = test_config(String::new());
    config.supabase_url = origin;
    config.supabase_anon_key = "anon-key".to_string();
    let (_directory, db) = test_powersync_db().await;
    let requested = vec![
        RemoteExtractionRow {
            id: "extraction-confirmed".to_string(),
            note_id: "note-create".to_string(),
            user_id: "user-1".to_string(),
            key: TOPIC_EXTRACTION_KEY.to_string(),
            value: "rust".to_string(),
        },
        RemoteExtractionRow {
            id: "extraction-pending".to_string(),
            note_id: "note-create".to_string(),
            user_id: "user-1".to_string(),
            key: TOPIC_EXTRACTION_KEY.to_string(),
            value: "sqlite".to_string(),
        },
    ];

    let outcome = create_extractions_with_token(
        &db,
        &reqwest::Client::new(),
        &config,
        "access-token",
        &requested,
    )
    .await;

    assert_eq!(outcome.confirmed_ids, ["extraction-confirmed"]);
    assert_eq!(outcome.pending_ids, ["extraction-pending"]);
    let reader = db.reader().await.unwrap();
    let count: i64 = reader
        .query_row(
            "SELECT COUNT(*) FROM note_extractions WHERE id = ?",
            params!["extraction-confirmed"],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count, 1);
    assert_eq!(server.join().unwrap().len(), 2);
}
