use super::*;
use crate::TOPIC_EXTRACTION_KEY;
use rusqlite::params;

async fn make_powersync_backend() -> (
    tempfile::TempDir,
    powersync::PowerSyncDatabase,
    LocalPowerSyncBackend,
) {
    struct NoHttp;

    #[async_trait::async_trait]
    impl powersync::http::HttpClient for NoHttp {
        async fn send(
            &self,
            _request: powersync::http::Request,
        ) -> Result<powersync::http::Response, powersync::error::PowerSyncError> {
            panic!("local backend tests must not make HTTP requests")
        }
    }

    use powersync::{ConnectionPool, PowerSyncDatabase, env::PowerSyncEnvironment};

    PowerSyncEnvironment::powersync_auto_extension().unwrap();
    let directory = tempfile::tempdir().unwrap();
    let pool = ConnectionPool::open(directory.path().join("test-powersync.db")).unwrap();
    let environment =
        PowerSyncEnvironment::custom(NoHttp, pool, PowerSyncEnvironment::tokio_timer());
    let db = PowerSyncDatabase::new(environment, crate::schema::app_schema());
    let backend = LocalPowerSyncBackend::new(db.clone(), "test-user-id".to_string());
    (directory, db, backend)
}

#[tokio::test]
async fn powersync_backend_writes_to_the_supplied_database() {
    let (_directory, db, backend) = make_powersync_backend().await;
    let id = uuid::Uuid::new_v4().to_string();

    backend
        .insert_note(&InsertNoteReq {
            id: &id,
            note_type: "normal",
            status: "ai_queued",
            title: Some("Shared database"),
            content: Some("visible immediately"),
            metadata: None,
            project_id: None,
            now: "2026-08-10T00:00:00Z",
        })
        .await
        .unwrap();

    assert_eq!(
        backend.find_note_content(&id).await.unwrap().as_deref(),
        Some("visible immediately")
    );
    let transaction = db.next_crud_transaction().await.unwrap().unwrap();
    assert_eq!(transaction.crud.len(), 1);
    assert_eq!(transaction.crud[0].table, "notes");
    assert_eq!(transaction.crud[0].id, id);
}

#[tokio::test]
async fn replacing_extractions_rolls_back_on_insert_failure() {
    let (_directory, _db, backend) = make_powersync_backend().await;
    let id = uuid::Uuid::new_v4().to_string();
    backend
        .insert_note(&InsertNoteReq {
            id: &id,
            note_type: "normal",
            status: "ai_queued",
            title: Some("Atomic extractions"),
            content: Some("body"),
            metadata: None,
            project_id: None,
            now: "2026-08-10T00:00:00Z",
        })
        .await
        .unwrap();
    backend
        .set_note_extractions(&id, TOPIC_EXTRACTION_KEY, &["old".to_string()])
        .await
        .unwrap();

    let writer = backend.database().writer().await.unwrap();
    writer
        .execute_batch(
            r#"
            CREATE TRIGGER fail_test_extraction
            INSTEAD OF INSERT ON note_extractions
            WHEN NEW.value = 'fail'
            BEGIN
                SELECT RAISE(ABORT, 'forced extraction failure');
            END;
            "#,
        )
        .unwrap();
    drop(writer);

    assert!(
        backend
            .set_note_extractions(
                &id,
                TOPIC_EXTRACTION_KEY,
                &["new".to_string(), "fail".to_string()],
            )
            .await
            .is_err()
    );
    assert_eq!(
        backend.list_note_topics(&[&id]).await.unwrap().get(&id),
        Some(&vec!["old".to_string()])
    );
}

async fn make_backend() -> LocalPowerSyncBackend {
    let (directory, _db, backend) = make_powersync_backend().await;
    std::mem::forget(directory);
    backend
}

#[tokio::test]
async fn local_backend_insert_and_find() {
    let backend = make_backend().await;
    let id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();

    let inserted = backend
        .insert_note(&InsertNoteReq {
            id: &id,
            note_type: "normal",
            status: "ai_queued",
            title: Some("Hello world"),
            content: Some("# Hello world\n\nContent here."),
            metadata: None,
            project_id: None,
            now: &now,
        })
        .await
        .unwrap();

    assert_eq!(inserted.uuid, id);
    assert_eq!(inserted.short_id, None);

    // Find by full id
    let note = backend.find_note(&id).await.unwrap();
    assert_eq!(note.id, id);
    assert_eq!(note.title, Some("Hello world".to_string()));

    // Find by full UUID compatibility path
    let resolved = backend.resolve_note_id(&id).await.unwrap();
    assert_eq!(resolved, id);

    let writer = backend.database().writer().await.unwrap();
    writer
        .execute(
            "UPDATE notes SET short_id = ? WHERE id = ?",
            params![42, id],
        )
        .unwrap();
    drop(writer);
    let resolved = backend.resolve_note_id("42").await.unwrap();
    assert_eq!(resolved, id);

    // UUID prefixes are not accepted for notes; use short IDs or full UUIDs.
    let prefix = &id[..8];
    let err = backend.resolve_note_id(prefix).await.unwrap_err();
    assert!(matches!(err, CliError::NoteNotFound { .. }));

    // Find content
    let content = backend.find_note_content(&id).await.unwrap();
    assert_eq!(content, Some("# Hello world\n\nContent here.".to_string()));
}

#[tokio::test]
async fn test_numeric_short_id_ref_does_not_fallback_to_short_uuid_prefix() {
    let backend = make_backend().await;
    let id = "42000000-e29b-41d4-a716-446655440000".to_string();
    let now = chrono::Utc::now().to_rfc3339();

    backend
        .insert_note(&InsertNoteReq {
            id: &id,
            note_type: "normal",
            status: "ai_queued",
            title: Some("Numeric prefix note"),
            content: Some("content"),
            metadata: None,
            project_id: None,
            now: &now,
        })
        .await
        .unwrap();

    let err = backend.resolve_note_id("42").await.unwrap_err();
    assert!(matches!(err, CliError::NoteNotFound { .. }));
}

#[tokio::test]
async fn test_eight_digit_uuid_prefix_does_not_resolve_note() {
    let backend = make_backend().await;
    let id = "12345678-e29b-41d4-a716-446655440000".to_string();
    let now = chrono::Utc::now().to_rfc3339();

    backend
        .insert_note(&InsertNoteReq {
            id: &id,
            note_type: "normal",
            status: "ai_queued",
            title: Some("Eight digit prefix note"),
            content: Some("content"),
            metadata: None,
            project_id: None,
            now: &now,
        })
        .await
        .unwrap();

    let err = backend.resolve_note_id("12345678").await.unwrap_err();
    assert!(matches!(err, CliError::NoteNotFound { .. }));
}

#[tokio::test]
async fn test_resolved_note_id_can_update_content_and_extractions() {
    let backend = make_backend().await;
    let id = "11fa49a2-6ac4-421e-94bf-240ee4197bb7".to_string();
    let now = chrono::Utc::now().to_rfc3339();

    backend
        .insert_note(&InsertNoteReq {
            id: &id,
            note_type: "normal",
            status: "ai_queued",
            title: Some("Editable note"),
            content: Some("hello"),
            metadata: None,
            project_id: None,
            now: &now,
        })
        .await
        .unwrap();
    let writer = backend.database().writer().await.unwrap();
    writer
        .execute(
            "UPDATE notes SET short_id = ? WHERE id = ?",
            params![1172, id],
        )
        .unwrap();
    drop(writer);

    let from_uuid = backend.resolve_note_id(&id).await.unwrap();
    backend
        .update_note_content(&from_uuid, "hi from uuid", true)
        .await
        .unwrap();
    assert_eq!(
        backend.find_note_content(&id).await.unwrap(),
        Some("hi from uuid".to_string())
    );

    let from_short_id = backend.resolve_note_id("1172").await.unwrap();
    backend
        .update_note_content(&from_short_id, "hi from short id", true)
        .await
        .unwrap();
    assert_eq!(
        backend.find_note_content(&id).await.unwrap(),
        Some("hi from short id".to_string())
    );

    backend
        .set_note_extractions(
            &from_short_id,
            TOPIC_EXTRACTION_KEY,
            &["orientation".to_string(), "cli".to_string()],
        )
        .await
        .unwrap();
    let extractions = backend
        .list_note_extractions(&[&id], &[TOPIC_EXTRACTION_KEY])
        .await
        .unwrap();
    assert_eq!(
        extractions.get(&id),
        Some(&vec![
            (TOPIC_EXTRACTION_KEY.to_string(), "cli".to_string()),
            (TOPIC_EXTRACTION_KEY.to_string(), "orientation".to_string())
        ])
    );
}

#[tokio::test]
async fn local_backend_list_filter() {
    let backend = make_backend().await;
    let now = chrono::Utc::now().to_rfc3339();

    // Create two projects
    let proj_a = backend.create_project("Project A").await.unwrap();
    let proj_b = backend.create_project("Project B").await.unwrap();

    // Insert notes in different projects
    let id_a = uuid::Uuid::new_v4().to_string();
    backend
        .insert_note(&InsertNoteReq {
            id: &id_a,
            note_type: "normal",
            status: "ai_queued",
            title: Some("Note A"),
            content: Some("content a"),
            metadata: None,
            project_id: Some(&proj_a),
            now: &now,
        })
        .await
        .unwrap();

    let id_b = uuid::Uuid::new_v4().to_string();
    backend
        .insert_note(&InsertNoteReq {
            id: &id_b,
            note_type: "normal",
            status: "ai_queued",
            title: Some("Note B"),
            content: Some("content b"),
            metadata: None,
            project_id: Some(&proj_b),
            now: &now,
        })
        .await
        .unwrap();

    // List by project A
    let notes = backend
        .list_notes(&NoteFilter {
            project_id: Some(&proj_a),
            note_type: None,
            archived: false,
            limit: 20,
        })
        .await
        .unwrap();
    assert_eq!(notes.len(), 1);
    assert_eq!(notes[0].id, id_a);
}

#[tokio::test]
async fn local_backend_search_notes() {
    let backend = make_backend().await;
    let now = chrono::Utc::now().to_rfc3339();

    let id = uuid::Uuid::new_v4().to_string();
    backend
        .insert_note(&InsertNoteReq {
            id: &id,
            note_type: "normal",
            status: "ai_queued",
            title: Some("Unique searchable title"),
            content: Some("some body text"),
            metadata: None,
            project_id: None,
            now: &now,
        })
        .await
        .unwrap();

    let results = backend
        .search_notes(
            &["Unique".to_string()],
            &NoteFilter {
                project_id: None,
                note_type: None,
                archived: false,
                limit: 20,
            },
        )
        .await
        .unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].id, id);

    // Empty keywords should return Err
    let err = backend
        .search_notes(
            &[],
            &NoteFilter {
                project_id: None,
                note_type: None,
                archived: false,
                limit: 20,
            },
        )
        .await;
    assert!(err.is_err());
}

#[tokio::test]
async fn local_backend_search_notes_matches_all_extraction_filters() {
    let backend = make_backend().await;
    let now = chrono::Utc::now().to_rfc3339();

    let matching_id = uuid::Uuid::new_v4().to_string();
    backend
        .insert_note(&InsertNoteReq {
            id: &matching_id,
            note_type: "normal",
            status: "ai_queued",
            title: Some("Whisper pipeline"),
            content: Some("ASR notes"),
            metadata: None,
            project_id: None,
            now: &now,
        })
        .await
        .unwrap();
    backend
        .set_note_extractions(
            &matching_id,
            TOPIC_EXTRACTION_KEY,
            &["ASR".to_string(), "AI".to_string()],
        )
        .await
        .unwrap();
    backend
        .set_note_extractions(&matching_id, "::person", &["瓜子".to_string()])
        .await
        .unwrap();

    let topic_only_id = uuid::Uuid::new_v4().to_string();
    backend
        .insert_note(&InsertNoteReq {
            id: &topic_only_id,
            note_type: "normal",
            status: "ai_queued",
            title: Some("Whisper without person"),
            content: Some("ASR notes"),
            metadata: None,
            project_id: None,
            now: &now,
        })
        .await
        .unwrap();
    backend
        .set_note_extractions(&topic_only_id, TOPIC_EXTRACTION_KEY, &["ASR".to_string()])
        .await
        .unwrap();

    let results = backend
        .search_notes_structured(
            &NoteSearch {
                keywords: vec!["Whisper".to_string()],
                extractions: vec![
                    MetadataFilter {
                        key: TOPIC_EXTRACTION_KEY.to_string(),
                        value: "ASR".to_string(),
                    },
                    MetadataFilter {
                        key: "::person".to_string(),
                        value: "瓜子".to_string(),
                    },
                ],
            },
            &NoteFilter {
                project_id: None,
                note_type: None,
                archived: false,
                limit: 20,
            },
        )
        .await
        .unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].id, matching_id);
}

#[tokio::test]
async fn local_backend_search_notes_accepts_structured_only_query() {
    let backend = make_backend().await;
    let now = chrono::Utc::now().to_rfc3339();
    let id = uuid::Uuid::new_v4().to_string();

    backend
        .insert_note(&InsertNoteReq {
            id: &id,
            note_type: "normal",
            status: "ai_queued",
            title: Some("No keyword match here"),
            content: Some("body"),
            metadata: None,
            project_id: None,
            now: &now,
        })
        .await
        .unwrap();
    backend
        .set_note_extractions(&id, "::company", &["OpenAI".to_string()])
        .await
        .unwrap();

    let results = backend
        .search_notes_structured(
            &NoteSearch {
                keywords: Vec::new(),
                extractions: vec![MetadataFilter {
                    key: "::company".to_string(),
                    value: "OpenAI".to_string(),
                }],
            },
            &NoteFilter {
                project_id: None,
                note_type: None,
                archived: false,
                limit: 20,
            },
        )
        .await
        .unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].id, id);
}

#[tokio::test]
async fn local_backend_list_extraction_values_dedupes_and_sorts() {
    let backend = make_backend().await;
    let now = chrono::Utc::now().to_rfc3339();

    for value in ["ASR", "AI", "ASR"] {
        let id = uuid::Uuid::new_v4().to_string();
        backend
            .insert_note(&InsertNoteReq {
                id: &id,
                note_type: "normal",
                status: "ai_queued",
                title: Some(value),
                content: Some("body"),
                metadata: None,
                project_id: None,
                now: &now,
            })
            .await
            .unwrap();
        backend
            .set_note_extractions(&id, TOPIC_EXTRACTION_KEY, &[value.to_string()])
            .await
            .unwrap();
    }

    let values = backend
        .list_extraction_values(&[TOPIC_EXTRACTION_KEY], false)
        .await
        .unwrap();

    assert_eq!(values, vec!["AI".to_string(), "ASR".to_string()]);
}

#[tokio::test]
async fn local_backend_search_respects_type_filter() {
    let backend = make_backend().await;
    let now = chrono::Utc::now().to_rfc3339();

    let normal_id = uuid::Uuid::new_v4().to_string();
    backend
        .insert_note(&InsertNoteReq {
            id: &normal_id,
            note_type: "normal",
            status: "ai_queued",
            title: Some("Shared searchable title"),
            content: Some("normal body"),
            metadata: None,
            project_id: None,
            now: &now,
        })
        .await
        .unwrap();

    let link_id = uuid::Uuid::new_v4().to_string();
    backend
        .insert_note(&InsertNoteReq {
            id: &link_id,
            note_type: "link",
            status: "ai_queued",
            title: Some("Shared searchable title"),
            content: Some("link body"),
            metadata: None,
            project_id: None,
            now: &now,
        })
        .await
        .unwrap();

    let results = backend
        .search_notes(
            &["Shared".to_string()],
            &NoteFilter {
                project_id: None,
                note_type: Some("link"),
                archived: false,
                limit: 20,
            },
        )
        .await
        .unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].id, link_id);
}

#[tokio::test]
async fn local_backend_archive() {
    let backend = make_backend().await;
    let id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();

    backend
        .insert_note(&InsertNoteReq {
            id: &id,
            note_type: "normal",
            status: "ai_queued",
            title: Some("To archive"),
            content: Some("content"),
            metadata: None,
            project_id: None,
            now: &now,
        })
        .await
        .unwrap();

    // Verify it appears in active list
    let active = backend
        .list_notes(&NoteFilter {
            project_id: None,
            note_type: None,
            archived: false,
            limit: 20,
        })
        .await
        .unwrap();
    assert!(active.iter().any(|n| n.id == id));

    // Archive it
    backend
        .set_note_deleted_at(&id, Some(&now), &now)
        .await
        .unwrap();

    // Should be gone from active
    let active_after = backend
        .list_notes(&NoteFilter {
            project_id: None,
            note_type: None,
            archived: false,
            limit: 20,
        })
        .await
        .unwrap();
    assert!(!active_after.iter().any(|n| n.id == id));

    // Should appear in archived
    let archived = backend
        .list_notes(&NoteFilter {
            project_id: None,
            note_type: None,
            archived: true,
            limit: 20,
        })
        .await
        .unwrap();
    assert!(archived.iter().any(|n| n.id == id));

    // Unarchive
    backend.set_note_deleted_at(&id, None, &now).await.unwrap();
    let active_restored = backend
        .list_notes(&NoteFilter {
            project_id: None,
            note_type: None,
            archived: false,
            limit: 20,
        })
        .await
        .unwrap();
    assert!(active_restored.iter().any(|n| n.id == id));
}

#[tokio::test]
async fn test_find_archived_note() {
    let backend = make_backend().await;
    let id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();

    backend
        .insert_note(&InsertNoteReq {
            id: &id,
            note_type: "normal",
            status: "ai_queued",
            title: Some("Archived note"),
            content: Some("content"),
            metadata: None,
            project_id: None,
            now: &now,
        })
        .await
        .unwrap();

    // Not findable via find_archived_note before archiving
    assert!(backend.find_archived_note(&id).await.is_err());

    // Archive it
    backend
        .set_note_deleted_at(&id, Some(&now), &now)
        .await
        .unwrap();

    // Now findable via find_archived_note
    let note = backend.find_archived_note(&id).await.unwrap();
    assert_eq!(note.id, id);
    assert_eq!(note.title, Some("Archived note".to_string()));
    assert!(note.deleted_at.is_some());

    // No longer findable via find_note (active-only)
    assert!(backend.find_note(&id).await.is_err());
}

// ─── Fix: PowerSync view-UPDATE zero affected rows ────────────────────

#[tokio::test]
async fn test_move_note_to_project_ok() {
    let backend = make_backend().await;
    let now = chrono::Utc::now().to_rfc3339();
    let note_id = uuid::Uuid::new_v4().to_string();
    let proj_a = backend.create_project("Proj-A").await.unwrap();
    let proj_b = backend.create_project("Proj-B").await.unwrap();

    backend
        .insert_note(&InsertNoteReq {
            id: &note_id,
            note_type: "normal",
            status: "ai_queued",
            title: Some("Test note"),
            content: Some("body"),
            metadata: None,
            project_id: Some(&proj_a),
            now: &now,
        })
        .await
        .unwrap();

    // Move to proj_b — should succeed (not return NoteNotFound)
    let result = backend
        .move_note_to_project(&note_id, &proj_b, Some(&proj_a))
        .await
        .unwrap();
    // This note was the only one in proj_a, so proj_a gets deleted
    assert_eq!(result.as_deref(), Some("Proj-A"));

    // Verify the note is now in proj_b
    let note = backend.find_note(&note_id).await.unwrap();
    assert_eq!(note.project_id.as_deref(), Some(proj_b.as_str()));
}

#[tokio::test]
async fn test_move_note_to_project_missing_returns_err() {
    let backend = make_backend().await;
    let fake_id = uuid::Uuid::new_v4().to_string();
    let proj_a = backend.create_project("Proj-A").await.unwrap();
    let proj_b = backend.create_project("Proj-B").await.unwrap();

    let err = backend
        .move_note_to_project(&fake_id, &proj_b, Some(&proj_a))
        .await
        .unwrap_err();
    match err {
        CliError::NoteNotFound { id } => assert_eq!(id, fake_id),
        _ => panic!("expected NoteNotFound, got {:?}", err),
    }
}

#[tokio::test]
async fn test_move_note_to_project_same_project_noop() {
    let backend = make_backend().await;
    let now = chrono::Utc::now().to_rfc3339();
    let note_id = uuid::Uuid::new_v4().to_string();
    let proj_x = backend.create_project("Proj-X").await.unwrap();

    backend
        .insert_note(&InsertNoteReq {
            id: &note_id,
            note_type: "normal",
            status: "ai_queued",
            title: Some("Same-project note"),
            content: Some("body"),
            metadata: None,
            project_id: Some(&proj_x),
            now: &now,
        })
        .await
        .unwrap();

    // Same source and target — should be idempotent, return Ok(None),
    // not delete the project (it still holds the note).
    let result = backend
        .move_note_to_project(&note_id, &proj_x, Some(&proj_x))
        .await
        .unwrap();
    assert_eq!(result, None, "same-project move should not delete project");

    // Verify project still exists and note is still in it
    let note = backend.find_note(&note_id).await.unwrap();
    assert_eq!(note.project_id.as_deref(), Some(proj_x.as_str()));
    let active = backend.list_projects(false).await.unwrap();
    assert!(
        active.iter().any(|p| p.id == proj_x),
        "project should still exist"
    );
}

#[tokio::test]
async fn test_update_note_title_ok() {
    let backend = make_backend().await;
    let now = chrono::Utc::now().to_rfc3339();
    let note_id = uuid::Uuid::new_v4().to_string();

    backend
        .insert_note(&InsertNoteReq {
            id: &note_id,
            note_type: "normal",
            status: "ai_queued",
            title: Some("Old title"),
            content: Some("body"),
            metadata: None,
            project_id: None,
            now: &now,
        })
        .await
        .unwrap();

    backend
        .update_note_title(&note_id, "New title")
        .await
        .unwrap();
    let note = backend.find_note(&note_id).await.unwrap();
    assert_eq!(note.title, Some("New title".to_string()));
}

#[tokio::test]
async fn test_update_note_flagged_ok() {
    let backend = make_backend().await;
    let now = chrono::Utc::now().to_rfc3339();
    let note_id = uuid::Uuid::new_v4().to_string();

    backend
        .insert_note(&InsertNoteReq {
            id: &note_id,
            note_type: "normal",
            status: "ai_queued",
            title: Some("Flag me"),
            content: Some("body"),
            metadata: None,
            project_id: None,
            now: &now,
        })
        .await
        .unwrap();

    backend.update_note_flagged(&note_id, true).await.unwrap();
    let note = backend.find_note(&note_id).await.unwrap();
    assert_eq!(note.is_flagged, Some(1));

    backend.update_note_flagged(&note_id, false).await.unwrap();
    let note = backend.find_note(&note_id).await.unwrap();
    assert_eq!(note.is_flagged, Some(0));
}

#[tokio::test]
async fn test_delete_project_archives() {
    let backend = make_backend().await;
    let proj_id = backend.create_project("ToDelete").await.unwrap();

    // Verify project exists
    let proj = backend.find_project(&proj_id).await.unwrap();
    assert_eq!(proj.name, "ToDelete");

    backend.delete_project(&proj_id).await.unwrap();

    // After archive, project should not appear in active list
    let active = backend.list_projects(false).await.unwrap();
    assert!(
        !active.iter().any(|p| p.id == proj_id),
        "deleted project should not appear in active list"
    );

    // Archived list should contain it
    let archived = backend.list_projects(true).await.unwrap();
    assert!(
        archived.iter().any(|p| p.id == proj_id),
        "deleted project should appear in archived list"
    );
}

#[tokio::test]
async fn test_delete_project_missing_returns_err() {
    let backend = make_backend().await;
    let fake_id = uuid::Uuid::new_v4().to_string();

    let err = backend.delete_project(&fake_id).await.unwrap_err();
    match err {
        CliError::Other(msg) => assert!(msg.contains("not found"), "got: {msg}"),
        _ => panic!("expected Other error, got {:?}", err),
    }
}

#[tokio::test]
async fn test_project_resolver_rejects_uuid_prefixes() {
    let backend = make_backend().await;

    let project_id = backend.create_project("Exact Project").await.unwrap();

    assert_eq!(
        backend.resolve_project_id(&project_id).await.unwrap(),
        project_id
    );

    let project_prefix = &project_id[..8];

    assert!(backend.resolve_project_id(project_prefix).await.is_err());
}
