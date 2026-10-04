//! Run only through scripts/test-private-pg.sh, which creates its own container.
use std::str::FromStr;
use std::sync::Arc;

use deadpool_postgres::{Manager, Pool};
use flicknote_client::dto::{NoteAddInput, NoteFindInput, ProjectAddInput};
use flicknote_core::backend::NoteDb;
use flicknote_core::services::note::NoteService;
use flicknote_core::services::project::ProjectService;
use flicknote_sync::pg::{PgNoteCreator, PgRequestDb, PgSearch};
use flicknote_sync::search::NoteSearch;
use tokio_postgres::NoTls;
use uuid::Uuid;

async fn tool_call(
    client: &reqwest::Client,
    resource: &str,
    token: &str,
    name: &str,
    arguments: serde_json::Value,
) -> serde_json::Value {
    let response = client.post(resource).bearer_auth(token).header("Accept", "application/json, text/event-stream").header("MCP-Protocol-Version", "2025-03-26")
        .json(&serde_json::json!({"jsonrpc":"2.0","id":100,"method":"tools/call","params":{"name":name,"arguments":arguments}})).send().await.unwrap();
    let status = response.status();
    let body = response.text().await.unwrap();
    assert_eq!(status, reqwest::StatusCode::OK, "{name}: {body}");
    let value: serde_json::Value =
        serde_json::from_str(&body).unwrap_or_else(|_| panic!("{name}: {body}"));
    assert_ne!(value["result"]["isError"], true, "{name}: {value}");
    value["result"]["structuredContent"].clone()
}

fn pool() -> Pool {
    pool_with_size(4)
}
fn pool_with_size(size: usize) -> Pool {
    let port: u16 = std::env::var("FLICKNOTE_TEST_PG_PORT")
        .expect("run through test-private-pg.sh")
        .parse()
        .unwrap();
    let url = format!("postgresql://flicknote_mcp@127.0.0.1:{port}/supabase?sslmode=disable");
    Pool::builder(Manager::new(
        tokio_postgres::Config::from_str(&url).unwrap(),
        NoTls,
    ))
    .max_size(size)
    .build()
    .unwrap()
}
async fn db(pool: &Pool, user: &str) -> Arc<PgRequestDb> {
    PgRequestDb::begin(pool, Uuid::parse_str(user).unwrap())
        .await
        .unwrap()
}
fn add_input(content: &str, draft: bool) -> NoteAddInput {
    NoteAddInput {
        content: content.into(),
        project: None,
        interpret_as_url: true,
        draft,
        topics: Vec::new(),
        created_by_ai: true,
        created_at: None,
    }
}

#[tokio::test]
#[ignore = "requires the test-owned PGroonga container started by scripts/test-private-pg.sh"]
async fn human_filters_use_json_boolean_across_pagination_count_and_search() {
    use flicknote_client::dto::{ExtractionFilterDto, NoteCountInput, NoteListInput};
    let pool = pool();
    let a = db(&pool, "11111111-1111-4111-8111-111111111111").await;
    let project = ProjectService::new(a.as_ref())
        .add(ProjectAddInput {
            name: "Creation filters".into(),
            color: None,
        })
        .await
        .unwrap();
    let mut expected = seed_creation_filters(&a, &project.id).await;
    let service = NoteService::new(a.as_ref());
    assert_eq!(
        service
            .count(NoteCountInput {
                project: Some(project.name.clone()),
                note_type: None,
                archived: false,
                human: true
            })
            .await
            .unwrap(),
        5
    );
    expected.reverse();
    let mut paged = Vec::new();
    loop {
        let rows = service
            .list(NoteListInput {
                project: Some(project.name.clone()),
                no_project: false,
                note_type: None,
                status: None,
                human: true,
                archived: false,
                shared: false,
                created_after: None,
                created_before: None,
                limit: 1,
                cursor: paged.last().copied(),
            })
            .await
            .unwrap();
        if rows.is_empty() {
            break;
        }
        paged.push(rows[0].id.unwrap());
    }
    assert_eq!(paged, expected);
    let mut input = NoteFindInput {
        keywords: vec!["Classification".into()],
        extractions: Vec::new(),
        project: Some(project.name),
        created_after: None,
        created_before: None,
        human: true,
        archived: false,
        limit: 10,
    };
    let mut lexical_ids: Vec<_> = PgSearch(a.clone())
        .find(&input)
        .await
        .unwrap()
        .iter()
        .map(|hit| hit.short_id.unwrap())
        .collect();
    lexical_ids.sort_unstable();
    expected.sort_unstable();
    assert_eq!(lexical_ids, expected);
    input.keywords.clear();
    input.extractions = vec![ExtractionFilterDto {
        key: "::topic".into(),
        value: "Classification".into(),
    }];
    let mut extracted_ids: Vec<_> = service
        .find(input)
        .await
        .unwrap()
        .iter()
        .map(|hit| hit.id.unwrap())
        .collect();
    extracted_ids.sort_unstable();
    assert_eq!(extracted_ids, expected);
    a.finish(true).await.unwrap();
}

async fn seed_creation_filters(a: &Arc<PgRequestDb>, project_id: &str) -> Vec<i64> {
    let mut expected = Vec::new();
    for (index, metadata) in [
        None,
        Some(r#"{"created_by_ai":false,"link":{"url":"https://example.test"}}"#),
        Some(r#"{"created_by_ai":true}"#),
        Some(r#"{"created_by_ai":"true"}"#),
        Some(r#"{"created_by_ai":1}"#),
        Some(r#"{"created_by_ai":null}"#),
    ]
    .into_iter()
    .enumerate()
    {
        let id = Uuid::new_v4().to_string();
        let inserted = a
            .insert_note(&flicknote_core::backend::InsertNoteReq {
                id: &id,
                note_type: "normal",
                status: "draft",
                title: Some("Classification"),
                content: Some("Classification"),
                metadata,
                project_id: Some(project_id),
                now: "2026-09-24T00:00:00Z",
            })
            .await
            .unwrap();
        a.set_note_extractions(&id, "::topic", &["Classification".into()])
            .await
            .unwrap();
        let service = NoteService::new(a.as_ref());
        service.append(&id, " preserved").await.unwrap();
        service.submit(&id).await.unwrap();
        service.archive(&id).await.unwrap();
        service.restore(&id).await.unwrap();
        let stored = a.find_note(&id).await.unwrap().metadata;
        assert_eq!(
            stored.map(|v| serde_json::from_str::<serde_json::Value>(&v).unwrap()),
            metadata.map(|v| serde_json::from_str::<serde_json::Value>(v).unwrap())
        );
        if index != 2 {
            expected.push(inserted.short_id.unwrap());
        }
    }
    expected
}

#[tokio::test]
#[ignore = "requires the test-owned PGroonga container started by scripts/test-private-pg.sh"]
async fn canonical_extractions_replace_filter_and_isolate_owners() {
    use flicknote_client::dto::ExtractionFilterDto;
    let pool = pool();
    let a = db(&pool, "11111111-1111-4111-8111-111111111111").await;
    let note = NoteService::new(a.as_ref())
        .add(
            &PgNoteCreator(a.clone()),
            add_input("Extraction contract", true),
        )
        .await
        .unwrap();
    let id = &note.uuid;
    a.set_note_extractions(id, "::topic", &["old".into()])
        .await
        .unwrap();
    a.set_note_extractions(id, "::person", &["Ada".into()])
        .await
        .unwrap();
    a.set_note_extractions(id, "::topic", &["Rust".into(), "Rust".into()])
        .await
        .unwrap();
    let rows = a
        .list_note_extractions(&[id], &["::topic", "::person"])
        .await
        .unwrap();
    assert_eq!(
        rows[id],
        vec![
            ("::person".into(), "Ada".into()),
            ("::topic".into(), "Rust".into())
        ]
    );
    assert_eq!(a.list_note_topics(&[id]).await.unwrap()[id], vec!["Rust"]);
    assert_eq!(
        a.list_extraction_values(&["::person"], false)
            .await
            .unwrap(),
        vec!["Ada"]
    );
    assert!(
        a.list_note_extractions(&[id], &["topic", "person"])
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(a.find_note(id).await.unwrap().status, "draft");
    let search = NoteFindInput {
        extractions: [("::topic", "Rust"), ("::person", "Ada")]
            .into_iter()
            .map(|(key, value)| ExtractionFilterDto {
                key: key.into(),
                value: value.into(),
            })
            .collect(),
        keywords: Vec::new(),
        project: None,
        created_after: None,
        created_before: None,
        human: false,
        archived: false,
        limit: 10,
    };
    assert!(
        NoteService::new(a.as_ref())
            .find(search.clone())
            .await
            .unwrap()
            .is_empty()
    );
    NoteService::new(a.as_ref())
        .submit(&note.short_id.unwrap().to_string())
        .await
        .unwrap();
    assert_eq!(
        NoteService::new(a.as_ref())
            .find(search.clone())
            .await
            .unwrap()
            .len(),
        1
    );
    a.set_note_extractions(id, "::topic", &[]).await.unwrap();
    assert!(a.list_note_topics(&[id]).await.unwrap().is_empty());
    assert!(
        NoteService::new(a.as_ref())
            .find(search.clone())
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        a.list_extraction_values(&["::person"], false)
            .await
            .unwrap(),
        vec!["Ada"]
    );
    assert_eq!(a.find_note(id).await.unwrap().status, "ai_queued");
    a.finish(true).await.unwrap();
    assert_extraction_owner_isolation(&pool, id, &search).await;
}

async fn assert_extraction_owner_isolation(pool: &Pool, id: &str, search: &NoteFindInput) {
    let mut search = search.clone();
    search.extractions.retain(|filter| filter.key == "::person");
    let b = db(pool, "22222222-2222-4222-8222-222222222222").await;
    assert!(
        b.list_note_extractions(&[id], &["::person"])
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        b.list_extraction_values(&["::person"], false)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        NoteService::new(b.as_ref())
            .find(search.clone())
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        b.set_note_extractions(id, "::person", &["Mallory".into()])
            .await
            .is_err()
    );
    b.finish(false).await.unwrap();
    let a = db(pool, "11111111-1111-4111-8111-111111111111").await;
    assert_eq!(
        NoteService::new(a.as_ref())
            .find(search)
            .await
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        a.list_note_extractions(&[id], &["::person"]).await.unwrap()[id],
        vec![("::person".into(), "Ada".into())]
    );
    a.finish(true).await.unwrap();
}

#[tokio::test]
#[ignore = "requires the test-owned PGroonga container started by scripts/test-private-pg.sh"]
async fn private_notes_and_search_use_real_pgroonga() {
    let pool = pool();
    let alice = "11111111-1111-4111-8111-111111111111";
    let bob = "22222222-2222-4222-8222-222222222222";
    let a = db(&pool, alice).await;
    let project = ProjectService::new(a.as_ref())
        .add(ProjectAddInput {
            name: "Private project".into(),
            color: None,
        })
        .await
        .unwrap();
    let creator = PgNoteCreator(a.clone());
    let notes = NoteService::new(a.as_ref());
    let mut text_input = add_input("# Orchard\n\n中文 苹果 and English apple", false);
    text_input.project = Some("Private project".into());
    let text = notes.add(&creator, text_input).await.unwrap();
    let id = text.short_id.unwrap().to_string();
    assert_eq!(a.find_note(&text.uuid).await.unwrap().status, "ai_queued");
    assert!(text.uuid.parse::<Uuid>().is_ok());
    let draft = notes
        .add(&creator, add_input("# Draft\n\nworking copy", true))
        .await
        .unwrap();
    assert_eq!(a.find_note(&draft.uuid).await.unwrap().status, "draft");
    let link = notes
        .add(&creator, add_input("https://example.test/article", false))
        .await
        .unwrap();
    assert_eq!(
        a.find_note(&link.uuid).await.unwrap().status,
        "source_queued"
    );
    a.finish(true).await.unwrap();

    let b = db(&pool, bob).await;
    assert!(b.resolve_note_id(&id).await.is_err());
    assert!(b.resolve_note_id(&text.uuid).await.is_err());
    assert!(b.resolve_project_id(&project.id).await.is_err());
    let attempted_id = Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();
    assert!(
        b.insert_note(&flicknote_core::backend::InsertNoteReq {
            id: &attempted_id,
            note_type: "normal",
            status: "draft",
            title: None,
            content: Some("cross-user link"),
            metadata: None,
            project_id: Some(&project.id),
            now: &now,
        })
        .await
        .is_err()
    );
    assert!(
        b.list_note_extractions(&[&text.uuid], &["::topic"])
            .await
            .unwrap()
            .is_empty()
    );
    b.finish(false).await.unwrap();

    let a = db(&pool, alice).await;
    let hits = PgSearch(a.clone())
        .find(&NoteFindInput {
            keywords: vec!["苹果".into(), "apple".into()],
            extractions: Vec::new(),
            project: Some("Private project".into()),
            created_after: None,
            created_before: None,
            human: false,
            archived: false,
            limit: 10,
        })
        .await
        .unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].short_id, text.short_id);
    assert!(!hits[0].snippet.segments.is_empty());
    let notes = NoteService::new(a.as_ref());
    notes.append(&id, "\n\n香蕉 banana").await.unwrap();
    assert_eq!(a.find_note(&text.uuid).await.unwrap().status, "ai_queued");
    notes.archive(&id).await.unwrap();
    a.finish(true).await.unwrap();

    let a = db(&pool, alice).await;
    let hits = PgSearch(a.clone())
        .find(&NoteFindInput {
            keywords: vec!["香蕉".into()],
            extractions: Vec::new(),
            project: Some("Private project".into()),
            created_after: None,
            created_before: None,
            human: false,
            archived: false,
            limit: 10,
        })
        .await
        .unwrap();
    assert!(hits.is_empty());
    NoteService::new(a.as_ref()).restore(&id).await.unwrap();
    a.finish(true).await.unwrap();
}

#[tokio::test]
#[ignore = "requires the test-owned PGroonga container started by scripts/test-private-pg.sh"]
#[expect(
    clippy::too_many_lines,
    reason = "one HTTP journey covers the complete remote tool subset"
)]
async fn private_http_mcp_advertises_and_runs_only_supported_tools() {
    use axum::{
        Json, Router,
        http::{HeaderMap, StatusCode, header},
        response::IntoResponse,
        routing::post,
    };
    use flicknote_sync::private_mcp::{PrivateMcpConfig, serve};
    use serde_json::{Value, json};
    use std::sync::atomic::{AtomicUsize, Ordering};

    let calls = Arc::new(AtomicUsize::new(0));
    let mcp_probe = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mcp_port = mcp_probe.local_addr().unwrap().port();
    drop(mcp_probe);
    let verifier_probe = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let verifier_port = verifier_probe.local_addr().unwrap().port();
    let resource = format!("http://127.0.0.1:{mcp_port}/mcp");
    let verifier_resource = resource.clone();
    let verifier_counter = calls.clone();
    let verifier = Router::new().route("/verify", post(move |headers: HeaderMap| {
        let resource = verifier_resource.clone();
        let counter = verifier_counter.clone();
        async move {
            counter.fetch_add(1, Ordering::SeqCst);
            match headers.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok()) {
                Some("Bearer full") => Json(json!({"ok":true,"userId":"11111111-1111-4111-8111-111111111111","scope":["flicknote:full"],"resource":resource})).into_response(),
                Some("Bearer bob") => Json(json!({"ok":true,"userId":"22222222-2222-4222-8222-222222222222","scope":["flicknote:full"],"resource":resource})).into_response(),
                _ => StatusCode::UNAUTHORIZED.into_response(),
            }
        }
    }));
    let verifier_task = tokio::spawn(async move {
        axum::serve(verifier_probe, verifier).await.unwrap();
    });
    let port = std::env::var("FLICKNOTE_TEST_PG_PORT").unwrap();
    let config = PrivateMcpConfig {
        database_url: format!(
            "postgresql://flicknote_mcp@127.0.0.1:{port}/supabase?sslmode=disable"
        ),
        listen: format!("127.0.0.1:{mcp_port}").parse().unwrap(),
        resource: resource.clone(),
        issuer: format!("http://127.0.0.1:{verifier_port}/oauth"),
        verifier: format!("http://127.0.0.1:{verifier_port}/verify"),
        allowed_hosts: vec![format!("127.0.0.1:{mcp_port}")],
        allowed_origins: vec![format!("http://127.0.0.1:{mcp_port}")],
    };
    let mcp_task = tokio::spawn(async move {
        serve(config).await.unwrap();
    });
    let client = reqwest::Client::new();
    for _ in 0..30 {
        if client
            .get(format!(
                "http://127.0.0.1:{mcp_port}/.well-known/oauth-protected-resource/mcp"
            ))
            .send()
            .await
            .is_ok()
        {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    let send = |body: Value, token: &'static str| {
        client
            .post(&resource)
            .bearer_auth(token)
            .header("Accept", "application/json, text/event-stream")
            .header("MCP-Protocol-Version", "2025-03-26")
            .json(&body)
            .send()
    };
    let init = send(json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"fixture","version":"1"}}}), "full").await.unwrap();
    assert_eq!(
        init.status(),
        StatusCode::OK,
        "{}",
        init.text().await.unwrap()
    );
    let listed = send(
        json!({"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}),
        "full",
    )
    .await
    .unwrap();
    assert_eq!(
        listed.status(),
        StatusCode::OK,
        "{}",
        listed.text().await.unwrap()
    );
    let body = listed.text().await.unwrap();
    let list: Value = serde_json::from_str(&body).unwrap_or_else(|_| panic!("tools/list: {body}"));
    let tools = list["result"]["tools"]
        .as_array()
        .unwrap_or_else(|| panic!("tools/list: {list}"));
    let names = tools
        .iter()
        .map(|tool| tool["name"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(names.len(), 23);
    fn strict_schema(value: &Value) {
        match value {
            Value::Object(fields) => {
                assert!(!fields.contains_key("format"));
                for (key, child) in fields {
                    if matches!(key.as_str(), "properties" | "$defs" | "patternProperties") {
                        for schema in child.as_object().unwrap().values() {
                            strict_schema(schema);
                        }
                    } else if matches!(
                        key.as_str(),
                        "items"
                            | "additionalProperties"
                            | "unevaluatedProperties"
                            | "not"
                            | "contains"
                    ) {
                        strict_schema(child);
                    } else if matches!(key.as_str(), "allOf" | "anyOf" | "oneOf" | "prefixItems") {
                        for schema in child.as_array().unwrap() {
                            strict_schema(schema);
                        }
                    }
                }
            }
            Value::Bool(_) => panic!("bare boolean schema term"),
            _ => {}
        }
    }
    fn no_format(value: &Value) {
        match value {
            Value::Object(fields) => {
                assert!(!fields.contains_key("format"));
                for child in fields.values() {
                    no_format(child);
                }
            }
            Value::Array(items) => {
                for child in items {
                    no_format(child);
                }
            }
            _ => {}
        }
    }
    for tool in tools {
        assert_eq!(tool["outputSchema"]["type"], "object", "{}", tool["name"]);
        strict_schema(&tool["outputSchema"]);
        no_format(&tool["inputSchema"]);
    }
    assert!(!names.contains(&"note_source"));
    assert!(!names.contains(&"note_share"));
    let add_tool = tools
        .iter()
        .find(|tool| tool["name"] == "note_add")
        .unwrap();
    assert!(add_tool["inputSchema"]["properties"].get("draft").is_none());
    assert_eq!(add_tool["inputSchema"]["additionalProperties"], false);
    let before = tool_call(&client, &resource, "full", "note_count", json!({})).await;
    for draft in [true, false] {
        let rejected = send(json!({"jsonrpc":"2.0","id":34,"method":"tools/call","params":{
            "name":"note_add","arguments":{"content":"Rejected stale draft argument","draft":draft}
        }}), "full").await.unwrap();
        let rejected: Value = rejected.json().await.unwrap();
        assert_eq!(rejected["result"]["isError"], true, "{rejected}");
        assert!(
            rejected["result"]["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("draft")
        );
    }
    let after = tool_call(&client, &resource, "full", "note_count", json!({})).await;
    assert_eq!(before, after);
    let added = send(json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"note_add","arguments":{"content":"# HTTP note\n\n中文 苹果"}}}), "full").await.unwrap();
    assert_eq!(
        added.status(),
        StatusCode::OK,
        "{}",
        added.text().await.unwrap()
    );
    let add_body = added.text().await.unwrap();
    let add_json: Value =
        serde_json::from_str(&add_body).unwrap_or_else(|_| panic!("note_add: {add_body}"));
    assert!(
        add_json["result"]["structuredContent"]["id"].is_number(),
        "note_add: {add_json}"
    );
    let id = add_json["result"]["structuredContent"]["id"]
        .as_i64()
        .unwrap();
    let pg_pool = pool();
    let seed = db(&pg_pool, "11111111-1111-4111-8111-111111111111").await;
    let full_id = seed.resolve_note_id(&id.to_string()).await.unwrap();
    seed.set_note_extractions(&full_id, "::topic", &["HTTP topic".into()])
        .await
        .unwrap();
    seed.set_note_extractions(&full_id, "::company", &["HTTP company".into()])
        .await
        .unwrap();
    seed.finish(true).await.unwrap();
    let detail = tool_call(&client, &resource, "full", "note_get", json!({"id":id})).await;
    assert_eq!(detail["content"], "中文 苹果");
    assert_eq!(detail["metadata"]["created_by_ai"], true);
    assert_eq!(detail["draft"], false);
    assert_eq!(detail["extractions"].as_array().unwrap().len(), 2);
    tool_call(
        &client,
        &resource,
        "full",
        "note_write",
        json!({"id":id,"content":"# Intro\n中文 苹果\n\n## Child\nbeta"}),
    )
    .await;
    let detail = tool_call(&client, &resource, "full", "note_get", json!({"id":id})).await;
    let section = detail["sections"][0]["id"].as_str().unwrap().to_string();
    let section_body = tool_call(
        &client,
        &resource,
        "full",
        "note_get_section",
        json!({"id":id,"section":section}),
    )
    .await;
    assert!(section_body["content"].as_str().unwrap().contains("Intro"));
    tool_call(
        &client,
        &resource,
        "full",
        "note_insert",
        json!({"id":id,"section":section,"position":"after","content":"# Tail\ntail text"}),
    )
    .await;
    tool_call(
        &client,
        &resource,
        "full",
        "note_rename_section",
        json!({"id":id,"section":section,"name":"Renamed"}),
    )
    .await;
    let after_rename = tool_call(&client, &resource, "full", "note_get", json!({"id":id})).await;
    let renamed = after_rename["sections"][0]["id"].as_str().unwrap();
    tool_call(
        &client,
        &resource,
        "full",
        "note_replace_section",
        json!({"id":id,"section":renamed,"content":"# Renamed\nreplacement"}),
    )
    .await;
    let after_replace = tool_call(&client, &resource, "full", "note_get", json!({"id":id})).await;
    let tail = after_replace["sections"][1]["id"].as_str().unwrap();
    tool_call(
        &client,
        &resource,
        "full",
        "note_delete_section",
        json!({"id":id,"section":tail}),
    )
    .await;
    tool_call(&client, &resource, "full", "note_modify", json!({"id":id,"before":"replacement","after":"edited","summary":"Private summary","flagged":true})).await;
    let flagged = tool_call(&client, &resource, "full", "note_get", json!({"id":id})).await;
    assert_eq!(flagged["flagged"], true);
    tool_call(
        &client,
        &resource,
        "full",
        "note_append",
        json!({"id":id,"content":"\nmore"}),
    )
    .await;
    tool_call(
        &client,
        &resource,
        "full",
        "note_write",
        json!({"id":id,"content":"# Search\n中文 香蕉 and English banana"}),
    )
    .await;
    let search = tool_call(
        &client,
        &resource,
        "full",
        "note_find",
        json!({"keywords":["香蕉","banana"]}),
    )
    .await;
    assert!(
        search["hits"]
            .as_array()
            .unwrap()
            .iter()
            .any(|hit| hit["short_id"] == id)
    );
    let future = tool_call(
        &client,
        &resource,
        "full",
        "note_find",
        json!({"keywords":["香蕉"],"created_after":"2099-01-01T00:00:00Z"}),
    )
    .await;
    assert!(future["hits"].as_array().unwrap().is_empty());
    let human = tool_call(
        &client,
        &resource,
        "full",
        "note_find",
        json!({"keywords":["香蕉"],"human":true}),
    )
    .await;
    assert!(human["hits"].as_array().unwrap().is_empty());
    let count = tool_call(&client, &resource, "full", "note_count", json!({})).await;
    assert!(count["count"].as_u64().unwrap() >= 1);
    let listed_notes = tool_call(&client, &resource, "full", "note_list", json!({})).await;
    assert!(
        listed_notes["notes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|note| note["id"] == id)
    );
    tool_call(&client, &resource, "full", "note_archive", json!({"id":id})).await;
    let archived = tool_call(
        &client,
        &resource,
        "full",
        "note_find",
        json!({"keywords":["香蕉"]}),
    )
    .await;
    assert!(
        !archived["hits"]
            .as_array()
            .unwrap()
            .iter()
            .any(|hit| hit["short_id"] == id)
    );
    tool_call(&client, &resource, "full", "note_restore", json!({"id":id})).await;
    let project = tool_call(
        &client,
        &resource,
        "full",
        "project_add",
        json!({"name":"HTTP project","color":"blue"}),
    )
    .await;
    assert_eq!(project["name"], "HTTP project");
    let changed = tool_call(
        &client,
        &resource,
        "full",
        "project_modify",
        json!({"project":"HTTP project","summary":"For HTTP"}),
    )
    .await;
    assert_eq!(changed["summary"], "For HTTP");
    tool_call(
        &client,
        &resource,
        "full",
        "project_get",
        json!({"project":"HTTP project"}),
    )
    .await;
    let projects = tool_call(&client, &resource, "full", "project_list", json!({})).await;
    assert!(
        projects["projects"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p["name"] == "HTTP project")
    );
    tool_call(
        &client,
        &resource,
        "full",
        "project_archive",
        json!({"project":"HTTP project"}),
    )
    .await;
    let seed = db(&pg_pool, "11111111-1111-4111-8111-111111111111").await;
    let draft = NoteService::new(seed.as_ref())
        .add(
            &PgNoteCreator(seed.clone()),
            add_input("# HTTP draft\nwork", true),
        )
        .await
        .unwrap();
    let draft_id = draft.short_id.unwrap();
    seed.finish(true).await.unwrap();
    let link = tool_call(
        &client,
        &resource,
        "full",
        "note_add",
        json!({"content":"https://example.com/article"}),
    )
    .await;
    let link_id = link["id"].as_i64().unwrap();
    let link_detail = tool_call(
        &client,
        &resource,
        "full",
        "note_get",
        json!({"id":link_id}),
    )
    .await;
    assert_eq!(link_detail["type"], "link");
    assert_eq!(link_detail["draft"], false);
    let check = db(&pg_pool, "11111111-1111-4111-8111-111111111111").await;
    assert_eq!(check.find_note(&full_id).await.unwrap().status, "ai_queued");
    let link_uuid = check.resolve_note_id(&link_id.to_string()).await.unwrap();
    assert_eq!(
        check.find_note(&link_uuid).await.unwrap().status,
        "source_queued"
    );
    check.finish(true).await.unwrap();
    let draft_detail = tool_call(
        &client,
        &resource,
        "full",
        "note_get",
        json!({"id":draft_id}),
    )
    .await;
    assert_eq!(draft_detail["draft"], true);
    assert_eq!(draft_detail["content"], "work");
    let drafts = tool_call(
        &client,
        &resource,
        "full",
        "note_list",
        json!({"status":"draft","limit":1}),
    )
    .await;
    assert_eq!(drafts["notes"][0]["id"], draft_id);
    tool_call(
        &client,
        &resource,
        "full",
        "note_modify",
        json!({"id":draft_id,"flagged":true}),
    )
    .await;
    tool_call(
        &client,
        &resource,
        "full",
        "note_write",
        json!({"id":draft_id,"content":"draft updated"}),
    )
    .await;
    let draft_detail = tool_call(
        &client,
        &resource,
        "full",
        "note_get",
        json!({"id":draft_id}),
    )
    .await;
    assert_eq!(draft_detail["draft"], true);
    assert_eq!(draft_detail["flagged"], true);
    assert_eq!(draft_detail["content"], "draft updated");
    tool_call(
        &client,
        &resource,
        "full",
        "note_modify",
        json!({"id":draft_id,"flagged":false}),
    )
    .await;
    let draft_detail = tool_call(
        &client,
        &resource,
        "full",
        "note_get",
        json!({"id":draft_id}),
    )
    .await;
    assert_eq!(draft_detail["flagged"], false);
    tool_call(
        &client,
        &resource,
        "full",
        "note_submit",
        json!({"id":draft_id}),
    )
    .await;
    let entities = tool_call(&client, &resource, "full", "entity_list", json!({})).await;
    assert!(
        entities["entities"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["value"] == "HTTP company")
    );
    let topics = tool_call(&client, &resource, "full", "topic_list", json!({})).await;
    assert!(
        topics["topics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|topic| topic == "HTTP topic")
    );
    let extracted = tool_call(
        &client,
        &resource,
        "full",
        "note_find",
        json!({"extractions":[{"key":"::topic","value":"HTTP topic"}]}),
    )
    .await;
    assert!(
        extracted["hits"]
            .as_array()
            .unwrap()
            .iter()
            .any(|hit| hit["short_id"] == id)
    );
    let extracted_human = tool_call(
        &client,
        &resource,
        "full",
        "note_find",
        json!({"extractions":[{"key":"::topic","value":"HTTP topic"}],"human":true}),
    )
    .await;
    assert!(extracted_human["hits"].as_array().unwrap().is_empty());
    let bob_list = tool_call(&client, &resource, "bob", "note_list", json!({})).await;
    assert!(bob_list["notes"].as_array().unwrap().is_empty());
    let bob_get = send(json!({"jsonrpc":"2.0","id":31,"method":"tools/call","params":{"name":"note_get","arguments":{"id":id}}}), "bob").await.unwrap();
    let bob_get: Value = bob_get.json().await.unwrap();
    assert_eq!(bob_get["result"]["isError"], true);
    let bob_count = tool_call(&client, &resource, "bob", "note_count", json!({})).await;
    assert_eq!(bob_count["count"], 0);
    let bob_topics = tool_call(&client, &resource, "bob", "topic_list", json!({})).await;
    assert!(bob_topics["topics"].as_array().unwrap().is_empty());
    let bob_search = tool_call(
        &client,
        &resource,
        "bob",
        "note_find",
        json!({"keywords":["香蕉"]}),
    )
    .await;
    assert!(bob_search["hits"].as_array().unwrap().is_empty());
    let unsupported = send(json!({"jsonrpc":"2.0","id":32,"method":"tools/call","params":{"name":"note_share","arguments":{"id":id}}}), "full").await.unwrap();
    let unsupported: Value = unsupported.json().await.unwrap();
    assert!(unsupported.get("error").is_some() || unsupported["result"]["isError"] == true);
    let unauthorized = send(
        json!({"jsonrpc":"2.0","id":4,"method":"tools/list","params":{}}),
        "partial",
    )
    .await
    .unwrap();
    assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);
    assert!(calls.load(Ordering::SeqCst) >= 4);

    // Permit INSERT/RETURNING but fail the canonical read in this test-owned DB.
    let (admin, connection) = tokio_postgres::connect(
        &format!("host=127.0.0.1 port={port} user=postgres dbname=supabase"),
        NoTls,
    )
    .await
    .unwrap();
    let admin_task = tokio::spawn(connection);
    admin.batch_execute("REVOKE SELECT ON notes FROM authenticated; GRANT SELECT (id,short_id,user_id) ON notes TO authenticated").await.unwrap();
    let failed = send(json!({"jsonrpc":"2.0","id":33,"method":"tools/call","params":{"name":"note_add","arguments":{"content":"canonical-read-fault"}}}), "full").await.unwrap();
    let failed: Value = failed.json().await.unwrap();
    admin.batch_execute("GRANT SELECT ON notes TO authenticated; REVOKE SELECT (id,short_id,user_id) ON notes FROM authenticated").await.unwrap();
    let row = admin
        .query_one(
            "SELECT count(*) FROM notes WHERE content=$1",
            &[&"canonical-read-fault"],
        )
        .await
        .unwrap();
    assert_eq!(row.get::<_, i64>(0), 0, "failed create must be rolled back");
    assert_eq!(failed["result"]["isError"], true);
    let error = &failed["result"]["structuredContent"];
    assert_eq!(error["details"]["created"], false, "{failed}");
    assert_eq!(error["details"]["rolled_back"], true);
    assert_eq!(error["code"], "note_create_failed");
    assert!(
        !error["message"]
            .as_str()
            .unwrap()
            .contains("Do not create it again")
    );
    admin_task.abort();
    mcp_task.abort();
    verifier_task.abort();
}

#[tokio::test]
#[ignore = "requires the test-owned PGroonga container started by scripts/test-private-pg.sh"]
async fn concurrent_appends_and_cancelled_transactions_preserve_ownership() {
    use flicknote_core::backend::InsertNoteReq;
    let pool = pool();
    let alice = "11111111-1111-4111-8111-111111111111";
    let bob = "22222222-2222-4222-8222-222222222222";
    let a = db(&pool, alice).await;
    let created = NoteService::new(a.as_ref())
        .add(
            &PgNoteCreator(a.clone()),
            add_input("# Concurrent\ninitial", false),
        )
        .await
        .unwrap();
    let id = created.short_id.unwrap().to_string();
    a.finish(true).await.unwrap();
    let mut tasks = Vec::new();
    for suffix in ["one", "two"] {
        let pool = pool.clone();
        let id = id.clone();
        tasks.push(tokio::spawn(async move {
            let db = db(&pool, alice).await;
            NoteService::new(db.as_ref())
                .append(&id, suffix)
                .await
                .unwrap();
            db.finish(true).await.unwrap();
        }));
    }
    for task in tasks {
        task.await.unwrap();
    }
    let a = db(&pool, alice).await;
    let content = a.find_note_content(&created.uuid).await.unwrap().unwrap();
    assert!(content.contains("one") && content.contains("two"));
    a.finish(true).await.unwrap();

    let cancelled_id = Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();
    let isolated_pool = pool_with_size(1);
    let a = db(&isolated_pool, alice).await;
    a.insert_note(&InsertNoteReq {
        id: &cancelled_id,
        note_type: "normal",
        status: "ai_queued",
        title: None,
        content: Some("rolled back"),
        metadata: None,
        project_id: None,
        now: &now,
    })
    .await
    .unwrap();
    drop(a); // Drops and disconnects the unfinished transaction; it cannot enter the pool.
    let b = db(&isolated_pool, bob).await;
    assert!(b.resolve_note_id(&cancelled_id).await.is_err());
    assert!(b.resolve_note_id(&created.uuid).await.is_err());
    b.finish(true).await.unwrap();
    let a = db(&isolated_pool, alice).await;
    assert!(a.resolve_note_id(&cancelled_id).await.is_err());
    assert!(a.resolve_note_id(&created.uuid).await.is_ok());
    a.finish(true).await.unwrap();
}
