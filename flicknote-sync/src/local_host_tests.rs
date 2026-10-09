#![allow(clippy::print_stderr)] // Test-owned latency evidence, never note bodies.
//! Production host and connector over test-owned storage and local HTTP only.
use crate::runtime::*;
use crate::today::{Snapshot, TodayWatch};
use axum::{
    Json, Router,
    body::Body,
    extract::State,
    response::{IntoResponse, Response},
    routing::post,
};
use flicknote_auth::{
    client::{AuthSession, AuthUser},
    session::save_session,
};
use flicknote_client::{AppRequest, AppResponse, DaemonClient, dto::NoteAddInput};
use futures_lite::{StreamExt, stream};
use serde_json::{Value, json};
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use tokio::sync::{Notify, mpsc};

#[derive(Clone)]
struct Fake {
    streams: mpsc::Sender<mpsc::Sender<String>>,
    gate: Arc<Notify>,
    mode: Arc<AtomicU8>,
    refresh_ok: Arc<AtomicBool>,
}
async fn sync(State(fake): State<Fake>) -> Response {
    let (send, receive) = mpsc::channel::<String>(8);
    fake.streams.send(send).await.unwrap();
    fake.gate.notified().await;
    let lines = stream::unfold(receive, |mut receive| async move {
        receive
            .recv()
            .await
            .map(|line| (Ok::<_, std::io::Error>(line), receive))
    });
    (
        [("content-type", "application/json")],
        Body::from_stream(lines),
    )
        .into_response()
}
async fn create(State(fake): State<Fake>, Json(mut note): Json<Value>) -> Response {
    match fake.mode.load(Ordering::SeqCst) {
        1 => return (axum::http::StatusCode::BAD_REQUEST, "rejected").into_response(),
        2 => return (axum::http::StatusCode::INTERNAL_SERVER_ERROR, "unavailable").into_response(),
        _ => {}
    }
    note["short_id"] = json!(if note["content"] == "partial" { 78 } else { 77 });
    note["is_flagged"] = json!(false);
    note["summary"] = Value::Null;
    note["source"] = Value::Null;
    note["deleted_at"] = Value::Null;
    (axum::http::StatusCode::CREATED, Json(json!([note]))).into_response()
}
fn session(config: &Config, expired: bool) {
    save_session(
        &config.paths.session_file,
        &AuthSession {
            access_token: "test-token".into(),
            refresh_token: "test-refresh".into(),
            expires_at: Some(if expired { 1 } else { u64::MAX }),
            user: AuthUser {
                id: "account-a".into(),
                email: None,
            },
        },
    )
    .unwrap();
}
async fn fixture() -> (
    tempfile::TempDir,
    Config,
    Fake,
    mpsc::Receiver<mpsc::Sender<String>>,
    JoinHandle<()>,
) {
    let root = tempfile::tempdir().unwrap();
    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let (streams, receive) = mpsc::channel(8);
    let fake = Fake {
        streams,
        gate: Arc::new(Notify::new()),
        mode: Arc::new(AtomicU8::new(0)),
        refresh_ok: Arc::new(AtomicBool::new(false)),
    };
    let router = Router::new()
        .route("/rest/v1/projects", axum::routing::patch(|| async { Json(json!([])) }))
        .route(
            "/write-checkpoint2.json",
            axum::routing::get(|| async { Json(json!({"data":{"write_checkpoint":"1"}})) }),
        )
        .route("/sync/stream", post(sync))
        .route(
            "/rest/v1/notes",
            post(create)
                .get(|| async { Json(json!([])) })
                .patch(|| async { Json(json!([])) }),
        )
        .route(
            "/rest/v1/note_extractions",
            post(|| async {
                (
                    axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                    "topic failed",
                )
            }),
        )
        .route("/auth/v1/token", post(|State(fake): State<Fake>| async move {
            if fake.refresh_ok.load(Ordering::SeqCst) {
                return Json(json!({"access_token":"refreshed-test-token","refresh_token":"refreshed-test-refresh","expires_at":u64::MAX,"user":{"id":"account-a"}})).into_response();
            }
            (axum::http::StatusCode::UNAUTHORIZED, Json(json!({"message":"expired"}))).into_response()
        }))
        .with_state(fake.clone());
    let server = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    let mut config = flicknote_core::profile::load(root.path()).unwrap();
    config.supabase_url.clone_from(&origin);
    config.supabase_anon_key = "test-anon".into();
    config.powersync_url.clone_from(&origin);
    config.api_url.clone_from(&origin);
    config.gateway_url = origin;
    session(&config, false);
    let db = open_powersync_database(&config).unwrap();
    let mut actors = spawn_powersync_actors(&db);
    {
        let writer = db.writer().await.unwrap();
        let now = chrono::Utc::now().to_rfc3339();
        for (id, user, short_id) in [("a", "account-a", 3), ("b", "account-b", 99)] {
            writer.execute("INSERT INTO notes(id,user_id,short_id,content,type,status,created_at) VALUES(?,?,?,'Cached body','link','ready',?)", rusqlite::params![id,user,short_id,now]).unwrap();
        }
        for (id, user, name) in [
            (
                "11111111-1111-4111-8111-111111111111",
                "account-a",
                "Current",
            ),
            ("22222222-2222-4222-8222-222222222222", "account-b", "Other"),
        ] {
            writer.execute("INSERT INTO projects(id,user_id,name,color,is_archived) VALUES(?,?,?,'123456',0)", rusqlite::params![id,user,name]).unwrap();
        }
        writer
            .execute(
                "UPDATE notes SET project_id='11111111-1111-4111-8111-111111111111' WHERE id='a'",
                [],
            )
            .unwrap();
        writer.execute("DELETE FROM ps_crud", []).unwrap();
    }
    db.disconnect().await;
    actors.abort_all();
    while actors.join_next().await.is_some() {}
    drop(db);
    (root, config, fake, receive, server)
}
async fn start(config: Config) -> LocalHost {
    let (_send, cancel) = watch::channel(false);
    // Keep cancellation sender alive through startup.
    tokio::time::timeout(
        Duration::from_secs(3),
        LocalHost::start(config, Some(0), cancel),
    )
    .await
    .unwrap()
    .unwrap()
}
async fn snapshot(
    watch: &mut TodayWatch,
    predicate: impl Fn(&Snapshot) -> bool + Send + Sync,
) -> Snapshot {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Some(Ok(value)) = watch.receiver.borrow_and_update().clone()
                && predicate(&value)
            {
                return value;
            }
            watch.receiver.changed().await.unwrap();
        }
    })
    .await
    .unwrap_or_else(|_| {
        panic!(
            "Watched snapshot deadline; last={:?}",
            watch.receiver.borrow()
        )
    })
}
async fn status(
    db: &PowerSyncDatabase,
    predicate: impl Fn(&powersync::SyncStatusData) -> bool + Send + Sync,
) {
    tokio::time::timeout(Duration::from_secs(8), async {
        let stream = db.watch_status();
        futures_lite::pin!(stream);
        while let Some(s) = stream.next().await {
            if predicate(&s) {
                return;
            }
        }
        panic!("status observer ended");
    })
    .await
    .unwrap_or_else(|_| panic!("Status deadline: {:?}", db.status()));
}
fn parse(body: &str) -> Value {
    serde_json::from_str(
        body.lines()
            .find_map(|line| {
                line.strip_prefix("data: ")
                    .filter(|data| data.starts_with('{'))
            })
            .unwrap_or(body),
    )
    .unwrap()
}
async fn mcp(port: u16, name: &str, args: Value) -> Value {
    let http = reqwest::Client::new();
    let url = format!("http://127.0.0.1:{port}/mcp");
    let initialized = http.post(&url).header("Accept", "application/json, text/event-stream").json(&json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"owned-test","version":"0"}}})).send().await.unwrap();
    let session = initialized.headers()["mcp-session-id"]
        .to_str()
        .unwrap()
        .to_string();
    let body = http.post(url).header("Accept", "application/json, text/event-stream").header("Mcp-Session-Id", session).json(&json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":name,"arguments":args}})).send().await.unwrap().text().await.unwrap();
    parse(&body)
}
fn add(content: &str, topics: Vec<String>) -> AppRequest {
    AppRequest::NoteAdd(NoteAddInput {
        content: content.into(),
        project: None,
        interpret_as_url: false,
        draft: false,
        topics,
        created_by_ai: false,
        created_at: None,
    })
}

#[tokio::test]
async fn cached_host_search_ipc_mcp_and_recall_survive_physical_replacement() {
    let (_root, config, _fake, _streams, server) = fixture().await;
    let host = start(config).await;
    {
        let writer = host.db.writer().await.unwrap();
        for content in ["oldword", "latestword 中文"] {
            writer.execute(
                "INSERT OR REPLACE INTO ps_data__notes(id, data) SELECT id, json_set(data, '$.content', ?1) FROM ps_data__notes WHERE id='a'",
                [content],
            ).unwrap();
        }
        // FTS internal identity is deliberately distinct from backing identity.
        writer
            .execute("UPDATE note_search_fts SET rowid=10001 WHERE uuid='a'", [])
            .unwrap();
        writer.execute("INSERT INTO note_extractions (id,note_id,user_id,key,value) VALUES('extraction','a','account-a','::topic','Replacement topic')", []).unwrap();
    }
    let client = DaemonClient::new(&host.socket);
    let input = flicknote_client::dto::NoteFindInput {
        keywords: vec!["latestword".into()],
        extractions: vec![],
        project: Some("Current".into()),
        created_after: None,
        created_before: None,
        human: true,
        archived: false,
        limit: 1,
    };
    let AppResponse::SearchHits(hits) = client.app(AppRequest::NoteFind(input)).await.unwrap()
    else {
        panic!("expected search hits");
    };
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].short_id, Some(3));
    assert_eq!(hits[0].note_type, "link");
    assert_eq!(hits[0].content_bytes, "latestword 中文".len() as u64);
    assert!(hits[0].snippet.segments.iter().any(|s| s.highlighted));
    let found = mcp(
        host.mcp_port,
        "note_find",
        json!({"keywords":["latestword"],"project":"Current","human":true,"limit":1}),
    )
    .await;
    let mcp_hits = &found["result"]["structuredContent"]["hits"];
    assert_eq!(mcp_hits, &serde_json::to_value(hits).unwrap());
    let old = mcp(host.mcp_port, "note_find", json!({"keywords":["oldword"]})).await;
    assert_eq!(old["result"]["structuredContent"]["hits"], json!([]));
    let AppResponse::NoteRecall(candidates) = client
        .app(AppRequest::NoteRecall {
            prompt: "Replacement topic".into(),
            project: Some("Current".into()),
        })
        .await
        .unwrap()
    else {
        panic!("expected recall candidates");
    };
    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].id, 3);
    host.db
        .writer()
        .await
        .unwrap()
        .execute(
            "UPDATE notes SET metadata='{\"created_by_ai\":true}' WHERE id='a'",
            [],
        )
        .unwrap();
    let hidden = mcp(
        host.mcp_port,
        "note_find",
        json!({"keywords":["latestword"],"human":true}),
    )
    .await;
    assert_eq!(hidden["result"]["structuredContent"]["hits"], json!([]));
    let AppResponse::NoteRecall(candidates) = client
        .app(AppRequest::NoteRecall {
            prompt: "Replacement topic".into(),
            project: None,
        })
        .await
        .unwrap()
    else {
        panic!("expected recall candidates");
    };
    assert!(candidates.is_empty());
    host.shutdown().await;
    server.abort();
}

#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn cached_host_production_creator_ipc_mcp_download_close_reopen_and_restart() {
    let (_root, config, fake, mut streams, server) = fixture().await;
    let began = Instant::now();
    let host = start(config.clone()).await;
    let ready_ms = began.elapsed().as_secs_f64() * 1000.0;
    let socket = host.socket.clone();
    let client = DaemonClient::new(&socket);
    let mut today = TodayWatch::start_for_user(host.db.clone(), host.user_id.clone());
    let initial = snapshot(&mut today, |s| s.rows.len() == 1).await;
    eprintln!(
        "MEASURE production_host_ready_ms={ready_ms:.3} cached_first_snapshot_ms={:.3} emission={}",
        began.elapsed().as_secs_f64() * 1000.0,
        initial.emission
    );
    assert_eq!(initial.rows[0].id, 3);
    assert_eq!(initial.rows[0].note_type, "link");
    assert_eq!(initial.rows[0].project_color.as_deref(), Some("123456"));
    assert_eq!(initial.projects.len(), 1);
    assert_eq!(initial.projects[0].name, "Current");
    assert!(client.health().await.is_ok());
    let send = tokio::time::timeout(Duration::from_secs(5), streams.recv())
        .await
        .unwrap()
        .unwrap();
    // The fake has not returned even HTTP headers: local cache/endpoints are already usable.
    assert!(
        mcp(host.mcp_port, "note_get", json!({"id":3})).await["result"]["structuredContent"]
            .is_object()
    );
    let (_cancel, receiver) = watch::channel(false);
    assert!(matches!(
        LocalHost::start(config.clone(), Some(0), receiver).await,
        Err(DaemonRunError::OwnershipConflict(_))
    ));
    let other = DataDirectoryLock::acquire(&_root.path().join("other-owner")).unwrap();
    let (_cancel, receiver) = watch::channel(false);
    assert!(matches!(
        LocalHost::start_owned(config.clone(), Some(0), receiver, other).await,
        Err(DaemonRunError::OwnershipConflict(_))
    ));
    assert!(client.health().await.is_ok());
    assert_eq!(
        flicknote_core::session::get_user_id(&config).unwrap(),
        "account-a"
    );
    client
        .app(AppRequest::NoteWrite {
            id: "3".into(),
            content: "CLI update".into(),
        })
        .await
        .unwrap();
    snapshot(&mut today, |s| s.rows[0].content == "CLI update").await;
    let project = mcp(
        host.mcp_port,
        "project_modify",
        json!({"project":"Current","color":"#654321"}),
    )
    .await;
    assert_eq!(project["result"]["structuredContent"]["color"], "#654321");
    snapshot(&mut today, |s| {
        s.rows[0].project_color.as_deref() == Some("#654321")
            && s.projects[0].color.as_deref() == Some("#654321")
    })
    .await;
    // Real remote-backed creator, canonical commit, and machine provenance.
    let created = mcp(
        host.mcp_port,
        "note_add",
        json!({"content":"Created through MCP"}),
    )
    .await;
    assert_eq!(created["result"]["structuredContent"]["id"], 77);
    snapshot(&mut today, |s| s.rows.first().is_some_and(|r| r.id == 77)).await;
    let row = host
        .db
        .reader()
        .await
        .unwrap()
        .query_row("SELECT metadata FROM notes WHERE short_id=77", [], |r| {
            r.get::<_, String>(0)
        })
        .unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&row).unwrap()["created_by_ai"],
        true
    );
    // Definite failure and production unknown/partial identity stay distinct.
    fake.mode.store(1, Ordering::SeqCst);
    let rejected = host.app.handle(add("rejected", vec![])).await.unwrap_err();
    assert_ne!(rejected.code, "note_create_unknown");
    fake.mode.store(2, Ordering::SeqCst);
    let unknown = host.app.handle(add("uncertain", vec![])).await.unwrap_err();
    assert_eq!(unknown.code, "note_create_unknown");
    assert!(!unknown.retryable);
    assert!(unknown.details.unwrap()["note_id"].is_string());
    fake.mode.store(0, Ordering::SeqCst);
    let partial = host
        .app
        .handle(add("partial", vec!["topic".into()]))
        .await
        .unwrap_err();
    assert_eq!(partial.code, "note_create_partial");
    assert_eq!(partial.details.unwrap()["short_id"], 78);
    fake.gate.notify_one();
    status(&host.db, powersync::SyncStatusData::is_connected).await;
    // Real downloader consumes a controlled wire checkpoint and canonical note.
    let row = json!({"short_id":88,"user_id":"account-a","type":"meeting","status":"ready","content":"Downloaded", "created_at":chrono::Utc::now().to_rfc3339()});
    send.send(format!("{}\n", json!({"checkpoint":{"last_op_id":"1","write_checkpoint":"1","buckets":[{"bucket":"owned","checksum":0,"count":1,"subscriptions":[{"default":0}]}],"streams":[{"name":"owned","is_default":true,"errors":[]}]}}))).await.unwrap();
    send.send(format!("{}\n", json!({"data":{"bucket":"owned","data":[{"op_id":"1","op":"PUT","object_id":"downloaded","object_type":"notes","checksum":0,"data":row.to_string()}]}}))).await.unwrap();
    send.send(format!(
        "{}\n",
        json!({"checkpoint_complete":{"last_op_id":"1"}})
    ))
    .await
    .unwrap();
    snapshot(&mut today, |s| {
        s.rows
            .first()
            .is_some_and(|r| r.id == 88 && r.content == "Downloaded")
    })
    .await;
    send.send("invalid-sync-line\n".into()).await.unwrap();
    drop(send);
    status(&host.db, |s| s.download_error().is_some()).await;
    let _reconnected = tokio::time::timeout(Duration::from_secs(8), streams.recv())
        .await
        .unwrap_or_else(|_| panic!("Reconnect deadline: {:?}", host.db.status()))
        .unwrap();
    fake.gate.notify_one();
    _reconnected.send(format!("{}\n", json!({"checkpoint":{"last_op_id":"1","write_checkpoint":"1","buckets":[{"bucket":"owned","checksum":0,"count":1,"subscriptions":[{"default":0}]}],"streams":[{"name":"owned","is_default":true,"errors":[]}]}}))).await.unwrap();
    _reconnected
        .send(format!(
            "{}\n",
            json!({"checkpoint_complete":{"last_op_id":"1"}})
        ))
        .await
        .unwrap();
    status(&host.db, |s| {
        s.is_connected() && s.download_error().is_none()
    })
    .await;
    drop(today); // Window close: host remains available.
    assert!(client.health().await.is_ok());
    assert!(mcp(host.mcp_port, "note_archive", json!({"id":88})).await["result"]["structuredContent"]["archived"].as_bool().unwrap());
    let mut reopened = TodayWatch::start_for_user(host.db.clone(), host.user_id.clone());
    snapshot(&mut reopened, |s| !s.rows.iter().any(|r| r.id == 88)).await;
    drop(reopened);
    host.shutdown().await;
    assert!(!socket.exists());
    let restarted = start(config).await;
    assert!(DaemonClient::new(&restarted.socket).health().await.is_ok());
    restarted.shutdown().await;
    server.abort();
}

#[tokio::test]
async fn authentication_failure_cancel_and_required_actor_failure_release_owner() {
    let (_root, config, fake, mut streams, server) = fixture().await;
    session(&config, true);
    let host = start(config.clone()).await;
    status(&host.db, |s| s.download_error().is_some()).await;
    assert!(DaemonClient::new(&host.socket).health().await.is_ok());
    let mut today = TodayWatch::start_for_user(host.db.clone(), host.user_id.clone());
    snapshot(&mut today, |s| s.rows.len() == 1).await;
    drop(today);
    fake.refresh_ok.store(true, Ordering::SeqCst);
    let _stream = tokio::time::timeout(Duration::from_secs(8), streams.recv())
        .await
        .unwrap_or_else(|_| panic!("Refresh deadline: {:?}", host.db.status()))
        .unwrap();
    fake.gate.notify_one();
    _stream.send(format!("{}\n", json!({"checkpoint":{"last_op_id":"0","write_checkpoint":"1","buckets":[{"bucket":"empty","checksum":0,"count":0,"subscriptions":[{"default":0}]}],"streams":[{"name":"empty","is_default":true,"errors":[]}]}}))).await.unwrap();
    _stream
        .send(format!(
            "{}\n",
            json!({"checkpoint_complete":{"last_op_id":"0"}})
        ))
        .await
        .unwrap();
    status(&host.db, |s| {
        s.is_connected() && s.download_error().is_none()
    })
    .await;
    assert_eq!(
        flicknote_auth::session::load_session(&config.paths.session_file)
            .unwrap()
            .access_token,
        "refreshed-test-token"
    );
    host.shutdown().await;
    session(&config, false);
    let host = start(config.clone()).await;
    let socket = host.socket.clone();
    host.actors.socket.abort();
    assert!(host.run_until(std::future::pending(), || {}).await.is_err());
    assert!(!socket.exists());
    let (cancel, receiver) = watch::channel(true);
    assert!(
        LocalHost::start(config.clone(), Some(0), receiver)
            .await
            .is_err()
    );
    drop(cancel);
    let (cancel, receiver) = watch::channel(false);
    let mut quitting = receiver.clone();
    {
        let startup = LocalHost::start(config.clone(), Some(0), receiver);
        futures_lite::pin!(startup);
        let ready = futures_lite::future::poll_once(&mut startup).await;
        cancel.send_replace(true);
        match ready {
            None => assert!(
                tokio::time::timeout(Duration::from_secs(3), startup)
                    .await
                    .unwrap()
                    .is_err()
            ),
            Some(Ok(host)) => host
                .run_until(
                    async {
                        if !*quitting.borrow() {
                            quitting.changed().await.unwrap();
                        }
                        Ok(())
                    },
                    || {},
                )
                .await
                .unwrap(),
            Some(Err(error)) => panic!("Startup failed before cancellation: {error}"),
        }
    }
    assert!(!socket.exists());
    let host = start(config.clone()).await;
    host.shutdown().await;
    std::fs::remove_file(&config.paths.session_file).unwrap();
    let (_cancel, receiver) = watch::channel(false);
    assert!(matches!(
        LocalHost::start(config, Some(0), receiver).await,
        Err(DaemonRunError::PermanentStartup(_))
    ));
    server.abort();
}

#[tokio::test]
async fn project_all_and_home_watch_account_membership_and_archival() {
    use crate::today::Destination;
    let (_root, config, _fake, _streams, server) = fixture().await;
    let host = start(config).await;
    let project = "11111111-1111-4111-8111-111111111111";
    let mut home = TodayWatch::start_for_user(host.db.clone(), host.user_id.clone());
    let mut all = TodayWatch::start_destination(
        host.db.clone(),
        host.user_id.clone(),
        Destination::Project(project.into()),
        false,
    );
    snapshot(&mut home, |s| s.rows.len() == 1).await;
    {
        let writer = host.db.writer().await.unwrap();
        for (id, user, short, assigned, date, deleted) in [
            (
                "old",
                "account-a",
                20,
                project,
                "2020-01-01T12:00:00Z",
                None,
            ),
            (
                "foreign",
                "account-b",
                90,
                project,
                "2020-01-01T12:00:00Z",
                None,
            ),
            (
                "unassigned",
                "account-a",
                30,
                "",
                "2020-01-01T12:00:00Z",
                None,
            ),
            (
                "deleted",
                "account-a",
                40,
                project,
                "2020-01-01T12:00:00Z",
                Some("2020-01-02T12:00:00Z"),
            ),
        ] {
            writer.execute("INSERT INTO notes(id,user_id,short_id,project_id,created_at,deleted_at,content,type) VALUES(?,?,?,?,?,?,'Old canonical body','normal')", rusqlite::params![id,user,short,assigned,date,deleted]).unwrap();
        }
        writer.execute("INSERT INTO projects(id,user_id,name,is_archived) VALUES('archived','account-a','Archived',1),('nullable','account-a','Nullable',NULL)", []).unwrap();
    }
    let notes = snapshot(&mut all, |s| s.rows.len() == 2 && s.projects.len() == 2).await;
    assert_eq!(notes.rows.iter().map(|r| r.id).collect::<Vec<_>>(), [20, 3]);
    assert_eq!(notes.rows[0].content, "Old canonical body");
    assert_eq!(notes.projects[0].id, project);
    let day = snapshot(&mut home, |s| s.projects.len() == 2).await;
    assert_eq!(day.rows.iter().map(|r| r.id).collect::<Vec<_>>(), [3]);
    host.db
        .writer()
        .await
        .unwrap()
        .execute("UPDATE projects SET is_archived=1 WHERE id=?", [project])
        .unwrap();
    snapshot(&mut all, |s| !s.projects.iter().any(|p| p.id == project)).await;
    host.db
        .writer()
        .await
        .unwrap()
        .execute("DELETE FROM projects WHERE id='nullable'", [])
        .unwrap();
    snapshot(&mut home, |s| s.projects.is_empty()).await;
    drop(all);
    drop(home);
    assert!(DaemonClient::new(&host.socket).health().await.is_ok());
    let count = mcp(host.mcp_port, "note_count", json!({})).await;
    assert!(!count["result"]["isError"].as_bool().unwrap_or(false));
    host.shutdown().await;
    server.abort();
}

#[tokio::test]
async fn desktop_previews_watch_thresholds_titles_and_canonical_content() {
    use crate::today::Destination;
    let (_root, config, _fake, _streams, server) = fixture().await;
    let host = start(config).await;
    let project = "11111111-1111-4111-8111-111111111111";
    let cases = [
        ("x".repeat(512), Some("ignored"), "x".repeat(512), "normal"),
        ("x".repeat(513), Some("  Long  title\tkept\n next  "), "Long  title\tkept next".into(), "link"),
        ("界".repeat(170) + "ab", Some("ignored"), "界".repeat(170) + "ab", "meeting"),
        ("界".repeat(171), None, "Untitled note".into(), "file"),
        ("x".repeat(513), Some(""), String::new(), "flash"),
        (String::new(), Some("ignored"), String::new(), "normal"),
        (" \t first  two\twords \r\n\n second\u{000b}third\u{000c}fourth\u{0085}fifth\u{2028}sixth\u{2029}last  ".into(), None, "first  two\twords second third fourth fifth sixth last".into(), "normal"),
        (" ".repeat(513), Some("byte count before folding"), "byte count before folding".into(), "normal"),
    ];
    {
        let writer = host.db.writer().await.unwrap();
        for (index, (content, title, _, kind)) in cases.iter().enumerate() {
            writer.execute("INSERT INTO notes(id,user_id,short_id,project_id,created_at,content,title,type) VALUES(?,'account-a',?,?,?, ?,?,?)", rusqlite::params![format!("preview-{index}"), 100 + index as i64, project, chrono::Utc::now().to_rfc3339(), content, title, kind]).unwrap();
        }
    }
    let mut home = TodayWatch::start_for_user(host.db.clone(), host.user_id.clone());
    let mut all = TodayWatch::start_destination(
        host.db.clone(),
        host.user_id.clone(),
        Destination::Project(project.into()),
        false,
    );
    for watch in [&mut home, &mut all] {
        let observed = snapshot(watch, |s| s.rows.len() == cases.len() + 1).await;
        assert_eq!(
            observed.rows.iter().map(|r| r.id).collect::<Vec<_>>(),
            (100..108).rev().chain([3]).collect::<Vec<_>>()
        );
        for (index, (content, title, preview, kind)) in cases.iter().enumerate() {
            let row = observed
                .rows
                .iter()
                .find(|r| r.id == 100 + index as i64)
                .unwrap();
            assert_eq!(&row.content, content);
            assert_eq!(&row.preview, preview);
            assert_eq!(&row.note_type, kind);
            assert_eq!(row.title.as_deref(), *title);
            assert_eq!(row.project_id.as_deref(), Some(project));
            assert_eq!(row.project_name.as_deref(), Some("Current"));
            assert_eq!(row.uuid, format!("preview-{index}"));
        }
    }
    for (content, title, preview) in [
        ("x".repeat(513), Some("processed title"), "processed title"),
        ("x".repeat(513), Some("retitled"), "retitled"),
        ("x".repeat(513), None, "Untitled note"),
        (
            " short  body\n second\tpart ".into(),
            Some("ignored"),
            "short  body second\tpart",
        ),
    ] {
        host.db
            .writer()
            .await
            .unwrap()
            .execute(
                "UPDATE notes SET content=?,title=? WHERE id='preview-0'",
                rusqlite::params![content, title],
            )
            .unwrap();
        for watch in [&mut home, &mut all] {
            let observed = snapshot(watch, |s| {
                s.rows.iter().any(|r| r.id == 100 && r.preview == preview)
            })
            .await;
            let row = observed.rows.iter().find(|r| r.id == 100).unwrap();
            assert_eq!(row.content, content);
            assert_eq!(row.title.as_deref(), title);
            assert_eq!(row.uuid, "preview-0");
            assert_eq!(observed.rows.len(), cases.len() + 1);
        }
    }
    drop(home);
    drop(all);
    host.shutdown().await;
    server.abort();
}

#[tokio::test]
async fn detail_metadata_watch_resolves_archived_projects_without_cross_owner_labels() {
    use crate::today::Destination;
    let (_root, config, _fake, _streams, server) = fixture().await;
    let host = start(config).await;
    let project = "11111111-1111-4111-8111-111111111111";
    let mut home = TodayWatch::start_for_user(host.db.clone(), host.user_id.clone());
    let mut all = TodayWatch::start_destination(
        host.db.clone(),
        host.user_id.clone(),
        Destination::Project(project.into()),
        false,
    );
    {
        let writer = host.db.writer().await.unwrap();
        writer
            .execute(
                "UPDATE projects SET name='Archived known',is_archived=1 WHERE id=?",
                [project],
            )
            .unwrap();
        writer
            .execute("UPDATE notes SET title='Real title' WHERE id='a'", [])
            .unwrap();
    }
    for watch in [&mut home, &mut all] {
        let s = snapshot(watch, |s| {
            s.rows[0].title.as_deref() == Some("Real title") && s.projects.is_empty()
        })
        .await;
        let row = &s.rows[0];
        assert_eq!(row.id, 3);
        assert_eq!(row.uuid, "a");
        assert_eq!(row.content, "Cached body");
        assert_eq!(row.project_id.as_deref(), Some(project));
        assert_eq!(row.project_name.as_deref(), Some("Archived known"));
        assert_eq!(row.project_color.as_deref(), Some("123456"));
    }
    host.db
        .writer()
        .await
        .unwrap()
        .execute(
            "UPDATE projects SET name='Renamed archived',color='abcdef' WHERE id=?",
            [project],
        )
        .unwrap();
    for watch in [&mut home, &mut all] {
        let s = snapshot(watch, |s| {
            s.rows[0].project_name.as_deref() == Some("Renamed archived")
        })
        .await;
        assert_eq!(s.rows[0].project_color.as_deref(), Some("abcdef"));
        assert_eq!(s.rows[0].content, "Cached body");
    }
    // Assignment to a foreign or dangling UUID never borrows another account's label.
    for assignment in ["22222222-2222-4222-8222-222222222222", "missing"] {
        host.db
            .writer()
            .await
            .unwrap()
            .execute(
                "UPDATE notes SET project_id=?,title=NULL WHERE id='a'",
                [assignment],
            )
            .unwrap();
        let s = snapshot(&mut home, |s| {
            s.rows[0].title.is_none() && s.rows[0].project_name.is_none()
        })
        .await;
        assert!(s.rows[0].project_id.is_none());
        assert!(s.rows[0].project_color.is_none());
        assert_eq!(s.rows[0].content, "Cached body");
        snapshot(&mut all, |s| s.rows.is_empty()).await;
    }
    drop(home);
    drop(all);
    host.shutdown().await;
    server.abort();
}

async fn seed_source_matrix(host: &LocalHost, project: &str) {
    let cases = [
        None,
        Some("{}"),
        Some("null"),
        Some("[]"),
        Some("true"),
        Some(r#"{"created_by_ai":null}"#),
        Some(r#"{"created_by_ai":false}"#),
        Some(r#"{"created_by_ai":"true"}"#),
        Some(r#"{"created_by_ai":1}"#),
        Some(r#"{"created_by_ai":[],"created_by":"ai"}"#),
        Some(r#"{"created_by_ai":true}"#),
    ];
    {
        let mut writer = host.db.writer().await.unwrap();
        let tx = writer.transaction().unwrap();
        tx.execute(
            "UPDATE projects SET metadata='{\"summary\":\"Keep context\"}' WHERE id=?",
            [project],
        )
        .unwrap();
        for (index, metadata) in cases.iter().enumerate() {
            tx.execute("INSERT INTO notes(id,user_id,short_id,project_id,created_at,content,metadata,status) VALUES(?,'account-a',?,?,?,'Canonical human body',?,'ready')", rusqlite::params![format!("source-{index}"), 100 + index as i64,project,chrono::Utc::now().to_rfc3339(),metadata]).unwrap();
        }
        for (id, owner, deleted, date) in [
            (
                "foreign-source",
                "account-b",
                None,
                chrono::Utc::now().to_rfc3339(),
            ),
            (
                "deleted-source",
                "account-a",
                Some("2026-01-01"),
                chrono::Utc::now().to_rfc3339(),
            ),
            (
                "old-source",
                "account-a",
                None,
                "2020-01-01T12:00:00Z".into(),
            ),
        ] {
            tx.execute("INSERT INTO notes(id,user_id,short_id,project_id,created_at,deleted_at,metadata) VALUES(?,?,900,?,?,?,NULL)", rusqlite::params![id,owner,project,date,deleted]).unwrap();
        }
        // More than LIMIT newer MCP notes must not crowd out the lower-ID humans.
        for index in 0..=crate::today::LIMIT {
            tx.execute("INSERT INTO notes(id,user_id,short_id,project_id,created_at,content,metadata,type,status,is_flagged) VALUES(?,'account-a',?,?,?,'MCP body','{\"created_by_ai\":true}','normal','ready',0)",rusqlite::params![format!("mcp-{index}"),1000+index as i64,project,chrono::Utc::now().to_rfc3339()]).unwrap();
        }
        tx.commit().unwrap();
    }
}

#[tokio::test]
async fn human_scope_marker_matrix_before_limit_and_direct_access() {
    use crate::today::Destination;
    let (_root, config, _fake, _streams, server) = fixture().await;
    let host = start(config).await;
    let project = "11111111-1111-4111-8111-111111111111";
    seed_source_matrix(&host, project).await;
    let mut home = TodayWatch::start_destination(
        host.db.clone(),
        host.user_id.clone(),
        Destination::Home,
        true,
    );
    let mut all = TodayWatch::start_destination(
        host.db.clone(),
        host.user_id.clone(),
        Destination::Project(project.into()),
        true,
    );
    let day = snapshot(&mut home, |s| s.rows.len() == 11).await;
    let history = snapshot(&mut all, |s| s.rows.len() == 12).await;
    assert_eq!(
        day.rows.iter().map(|r| r.id).collect::<Vec<_>>(),
        (100..110).rev().chain([3]).collect::<Vec<_>>()
    );
    assert_eq!(history.rows[0].uuid, "old-source");
    assert_eq!(day.projects.len(), 1);
    assert_eq!(day.projects[0].id, project);
    assert_eq!(day.projects[0].summary.as_deref(), Some("Keep context"));
    let mut unfiltered = TodayWatch::start_destination(
        host.db.clone(),
        host.user_id.clone(),
        Destination::Home,
        false,
    );
    let full = snapshot(&mut unfiltered, |s| s.rows.len() == crate::today::LIMIT).await;
    assert_eq!(full.projects[0].name, day.projects[0].name);
    // Both machine boundaries still resolve the hidden explicit ID.
    let direct = DaemonClient::new(&host.socket)
        .app(AppRequest::NoteGet {
            id: "1000".into(),
            archived: false,
        })
        .await
        .unwrap();
    assert!(matches!(direct, AppResponse::NoteDetail(_)));
    let response = mcp(host.mcp_port, "note_get", json!({"id":1000})).await;
    assert!(!response["result"]["isError"].as_bool().unwrap_or(false));
    host.db
        .writer()
        .await
        .unwrap()
        .execute(
            "UPDATE notes SET metadata='{\"created_by_ai\":true}' WHERE id='source-0'",
            [],
        )
        .unwrap();
    snapshot(&mut home, |s| {
        s.rows.len() == 10 && !s.rows.iter().any(|r| r.uuid == "source-0")
    })
    .await;
    host.db
        .writer()
        .await
        .unwrap()
        .execute(
            "UPDATE notes SET metadata='{\"created_by_ai\":false}' WHERE id='source-10'",
            [],
        )
        .unwrap();
    snapshot(&mut all, |s| {
        s.rows.len() == 12 && s.rows.iter().any(|r| r.uuid == "source-10")
    })
    .await;
    host.db
        .writer()
        .await
        .unwrap()
        .execute(
            "UPDATE notes SET metadata='{\"created_by_ai\":true}' WHERE user_id='account-a'",
            [],
        )
        .unwrap();
    let empty = snapshot(&mut all, |s| s.rows.is_empty()).await;
    assert_eq!(empty.projects[0].id, project);
    assert_eq!(empty.projects[0].summary, day.projects[0].summary);
    drop((home, all, unfiltered));
    host.shutdown().await;
    server.abort();
}

#[tokio::test]
#[allow(clippy::too_many_lines)] // One owned host verifies canonical mutation, readers and bounded watch membership.
async fn shared_archive_watch_use_canonical_expiry_owner_source_before_limit() {
    use crate::today::Destination;
    let (_root, config, _fake, _streams, server) = fixture().await;
    let events_path = config
        .paths
        .data_dir
        .join("events/project_assignment.jsonl");
    let host = start(config).await;
    seed_source_matrix(&host, "11111111-1111-4111-8111-111111111111").await;
    // This projection matrix also has intentionally sparse historical rows;
    // canonical lifecycle readers require their stored type to be populated.
    host.db
        .writer()
        .await
        .unwrap()
        .execute("UPDATE notes SET type=coalesce(type, 'normal')", [])
        .unwrap();
    // Exercise the internal UUID seam through the production Application, then its
    // existing IPC/MCP readers; no new wire contract or routing provenance.
    let project = "11111111-1111-4111-8111-111111111111";
    let other = "33333333-3333-4333-8333-333333333333";
    host.db
        .writer()
        .await
        .unwrap()
        .execute(
            "INSERT INTO projects(id,user_id,name,is_archived) VALUES(?,'account-a','Other',0)",
            [other],
        )
        .unwrap();
    let local =
        flicknote_core::backend::LocalPowerSyncBackend::new(host.db.clone(), host.user_id.clone());
    host.app
        .classify_local_note(&local, "a", 3, other)
        .await
        .unwrap();
    let response = DaemonClient::new(&host.socket)
        .app(AppRequest::NoteGet {
            id: "3".into(),
            archived: false,
        })
        .await
        .unwrap();
    assert!(
        matches!(response, AppResponse::NoteDetail(ref n) if n.note.project_id.as_deref() == Some(other))
    );
    let response = mcp(host.mcp_port, "note_get", json!({"id":3})).await;
    assert!(!response["result"]["isError"].as_bool().unwrap_or(false));
    host.app
        .classify_local_note(&local, "a", 3, other)
        .await
        .unwrap();
    host.app
        .classify_local_note(&local, "a", 3, project)
        .await
        .unwrap();
    let events = std::fs::read_to_string(events_path).unwrap();
    let events: Vec<serde_json::Value> = events
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(events.len(), 2, "same-project drop emits no event");
    assert!(
        events
            .iter()
            .all(|e| e["source"] == "manual" && e["probability"].is_null())
    );
    {
        let writer = host.db.writer().await.unwrap();
        writer.execute("INSERT INTO note_shares(id,user_id,token,created_at) SELECT id,user_id,'owned-token','2026-01-01T00:00:00Z' FROM notes", []).unwrap();
        writer
            .execute(
                "UPDATE projects SET is_archived=1 WHERE user_id='account-a'",
                [],
            )
            .unwrap();
    }
    let mut shared = TodayWatch::start_destination(
        host.db.clone(),
        host.user_id.clone(),
        Destination::Shared,
        true,
    );
    let observed = snapshot(&mut shared, |s| s.rows.len() == 12).await;
    assert!(observed.projects.is_empty());
    assert!(observed.rows.iter().all(|r| r.shared && !r.archived));
    assert!(observed.rows.windows(2).all(|r| r[0].id > r[1].id));
    assert_eq!(observed.rows.last().unwrap().uuid, "a");
    assert_eq!(observed.rows[0].project_name.as_deref(), Some("Current"));
    assert!(
        observed
            .rows
            .iter()
            .all(|r| r.project_color.as_deref() == Some("123456"))
    );
    {
        let writer = host.db.writer().await.unwrap();
        writer
            .execute(
                "UPDATE note_shares SET expires_at='2000-01-01T00:00:00Z' WHERE id='source-0'",
                [],
            )
            .unwrap();
        // The share row's owner must match the note, independent of other account rows.
        writer
            .execute(
                "UPDATE note_shares SET user_id='account-b' WHERE id='source-1'",
                [],
            )
            .unwrap();
    }
    snapshot(&mut shared, |s| s.rows.len() == 10).await;
    {
        let writer = host.db.writer().await.unwrap();
        writer.execute("UPDATE notes SET deleted_at='2026-01-02T00:00:00Z',status='draft' WHERE user_id='account-a'", []).unwrap();
    }
    snapshot(&mut shared, |s| s.rows.is_empty()).await;
    let mut archived = TodayWatch::start_destination(
        host.db.clone(),
        host.user_id.clone(),
        Destination::Archive,
        true,
    );
    let observed = snapshot(&mut archived, |s| s.rows.len() == 13).await;
    assert!(observed.rows.iter().all(|r| r.archived && r.draft));
    assert_eq!(observed.rows.last().unwrap().uuid, "a");
    let mut full = TodayWatch::start_destination(
        host.db.clone(),
        host.user_id.clone(),
        Destination::Archive,
        false,
    );
    snapshot(&mut full, |s| s.rows.len() == crate::today::LIMIT).await;
    host.app
        .handle(AppRequest::NoteRestore { id: "100".into() })
        .await
        .unwrap();
    snapshot(&mut archived, |s| {
        s.rows.len() == 12 && s.rows.iter().all(|r| r.uuid != "source-0")
    })
    .await;
    // Explicit archived reads via existing IPC/MCP remain unfiltered.
    let response = DaemonClient::new(&host.socket)
        .app(AppRequest::NoteGet {
            id: "1000".into(),
            archived: true,
        })
        .await
        .unwrap();
    assert!(matches!(response, AppResponse::NoteDetail(_)));
    let response = mcp(
        host.mcp_port,
        "note_get",
        json!({"id":1000,"archived":true}),
    )
    .await;
    assert!(!response["result"]["isError"].as_bool().unwrap_or(false));
    drop((shared, archived, full));
    host.shutdown().await;
    server.abort();
}

#[tokio::test]
#[allow(clippy::too_many_lines)] // One owned host verifies the calendar and SQL boundary together.
async fn historical_day_and_project_week_calendar_membership_before_limit() {
    use crate::today::{Destination, Period};
    use chrono::{Days, TimeZone};
    use chrono_tz::America::New_York;
    for (month, day, hours) in [(3, 7, 23), (10, 31, 25)] {
        let now = New_York
            .with_ymd_and_hms(2026, month, day, 12, 0, 0)
            .unwrap();
        let historical = Period::Day(Some(now.date_naive()));
        let (start, end) = historical.range(&now).unwrap().unwrap();
        assert_eq!((end - start).num_hours(), hours);
        let later = now + chrono::Duration::days(10);
        assert_eq!(historical.range(&later).unwrap(), Some((start, end)));
        assert_ne!(Period::Day(None).range(&later).unwrap(), Some((start, end)));
        assert!(!historical.follows_clock());
        assert!(Period::Day(None).follows_clock());
        let week = Period::Week(None).range(&now).unwrap().unwrap();
        assert_eq!((week.1 - week.0).num_hours(), 168 - (24 - hours));
    }
    let before = New_York.with_ymd_and_hms(2026, 1, 5, 3, 59, 59).unwrap();
    let after = before + chrono::Duration::seconds(1);
    assert_ne!(
        Period::Day(None).range(&before).unwrap(),
        Period::Day(None).range(&after).unwrap()
    );
    let sunday = New_York.with_ymd_and_hms(2026, 1, 4, 23, 59, 59).unwrap();
    let monday = sunday + chrono::Duration::seconds(1);
    assert_eq!(Period::Week(None).date(&sunday).to_string(), "2025-12-29");
    assert_eq!(Period::Week(None).date(&monday).to_string(), "2026-01-05");
    assert_eq!(Period::Day(None).date(&monday).to_string(), "2026-01-04");
    for period in [Period::Day(None), Period::Week(None), Period::All] {
        assert!(period.shifted(true, &monday).is_none());
    }
    let previous = Period::Week(None).shifted(false, &monday).unwrap();
    assert_eq!(previous.shifted(true, &monday), Some(Period::Week(None)));
    assert_eq!(
        previous.range(&before).unwrap(),
        previous.range(&monday).unwrap()
    );

    let (_root, config, _fake, _streams, server) = fixture().await;
    let host = start(config).await;
    let project = "11111111-1111-4111-8111-111111111111";
    let day = chrono::Local::now()
        .date_naive()
        .checked_sub_days(Days::new(30))
        .unwrap();
    let day_period = Period::Day(Some(day));
    let (day_start, day_end) = day_period.range(&chrono::Local::now()).unwrap().unwrap();
    let monday = day
        - chrono::Duration::days(i64::from(
            chrono::Datelike::weekday(&day).num_days_from_monday(),
        ));
    let week_period = Period::Week(Some(monday));
    let (week_start, week_end) = week_period.range(&chrono::Local::now()).unwrap().unwrap();
    {
        let mut writer = host.db.writer().await.unwrap();
        let tx = writer.transaction().unwrap();
        tx.execute("DELETE FROM notes", []).unwrap();
        for (id, date, owner, assigned) in [
            (
                1,
                day_start - chrono::Duration::seconds(1),
                "account-a",
                project,
            ),
            (2, day_start, "account-a", project),
            (
                3,
                day_end - chrono::Duration::seconds(1),
                "account-a",
                project,
            ),
            (4, day_end, "account-a", project),
            (5, week_start, "account-a", project),
            (6, week_end, "account-a", project),
            (7, day_start, "account-b", project),
            (8, day_start, "account-a", "other-project"),
        ] {
            tx.execute("INSERT INTO notes(id,short_id,user_id,project_id,created_at,content,status) VALUES(?,?,?,?,?,'Calendar body','ready')", rusqlite::params![format!("calendar-{id}"),id,owner,assigned,date.to_rfc3339()]).unwrap();
        }
        // Excluded high IDs must not exhaust the bound before calendar/source/owner filtering.
        for index in 0..=crate::today::LIMIT {
            let (kind, date, owner, metadata) = match index % 3 {
                0 => ("future", week_end, "account-a", "{}"),
                1 => (
                    "machine",
                    day_start,
                    "account-a",
                    "{\"created_by_ai\":true}",
                ),
                _ => ("foreign", day_start, "account-b", "{}"),
            };
            tx.execute("INSERT INTO notes(id,short_id,user_id,project_id,created_at,metadata,status) VALUES(?,?,?,?,?,?,'ready')", rusqlite::params![format!("{kind}-{index}"),100+index as i64,owner,project,date.to_rfc3339(),metadata]).unwrap();
        }
        tx.commit().unwrap();
    }
    let mut home = TodayWatch::start_period(
        host.db.clone(),
        host.user_id.clone(),
        Destination::Home,
        true,
        day_period,
    );
    let home_snapshot = snapshot(&mut home, |s| s.rows.len() >= 3).await;
    let mut expected = vec![8, 3, 2];
    if week_start >= day_start && week_start < day_end {
        expected.push(5);
    }
    expected.sort_unstable_by(|a, b| b.cmp(a));
    assert_eq!(
        home_snapshot.rows.iter().map(|r| r.id).collect::<Vec<_>>(),
        expected
    );
    assert_eq!(home_snapshot.range, Some((day_start, day_end)));
    let mut week = TodayWatch::start_period(
        host.db.clone(),
        host.user_id.clone(),
        Destination::Project(project.into()),
        true,
        week_period,
    );
    let week_snapshot = snapshot(&mut week, |s| !s.rows.is_empty()).await;
    assert_eq!(week_snapshot.range, Some((week_start, week_end)));
    assert!(
        week_snapshot
            .rows
            .iter()
            .all(|r| r.project_id.as_deref() == Some(project))
    );
    assert!(week_snapshot.rows.iter().any(|r| r.id == 2));
    assert!(week_snapshot.rows.iter().any(|r| r.id == 5));
    assert!(
        !week_snapshot
            .rows
            .iter()
            .any(|r| [6, 7, 8].contains(&r.id) || r.id >= 100)
    );
    let mut all = TodayWatch::start_period(
        host.db.clone(),
        host.user_id.clone(),
        Destination::Project(project.into()),
        true,
        Period::All,
    );
    let all_snapshot = snapshot(&mut all, |s| s.rows.len() > 3000).await;
    assert_eq!(all_snapshot.range, None);
    assert!(
        all_snapshot
            .rows
            .iter()
            .any(|r| r.uuid.starts_with("future-"))
    );
    host.db
        .writer()
        .await
        .unwrap()
        .execute(
            "UPDATE notes SET content='Anchored emission' WHERE id='calendar-2'",
            [],
        )
        .unwrap();
    let updated = snapshot(&mut home, |s| {
        s.rows
            .iter()
            .any(|r| r.id == 2 && r.content == "Anchored emission")
    })
    .await;
    assert_eq!(updated.range, Some((day_start, day_end)));
    drop((home, week, all));
    host.shutdown().await;
    server.abort();
}

#[tokio::test]
async fn calendar_timer_rolls_current_watch_but_keeps_historical_watch_fixed() {
    use crate::today::{Destination, Period};
    let (_root, config, _fake, _streams, server) = fixture().await;
    let host = start(config).await;
    for (destination, current) in [
        (Destination::Home, Period::Day(None)),
        (
            Destination::Project("11111111-1111-4111-8111-111111111111".into()),
            Period::Week(None),
        ),
    ] {
        let actual = chrono::Local::now();
        let old_range = current.range(&actual).unwrap().unwrap();
        let before =
            (old_range.1 - chrono::Duration::milliseconds(700)).with_timezone(&chrono::Local);
        let clock = Arc::new(std::sync::Mutex::new(before));
        let fixed = if matches!(current, Period::Day(_)) {
            Period::Day(Some(current.date(&before)))
        } else {
            Period::Week(Some(current.date(&before)))
        };
        {
            let writer = host.db.writer().await.unwrap();
            writer.execute("DELETE FROM notes", []).unwrap();
            for (id, date) in [
                (1, old_range.1 - chrono::Duration::seconds(1)),
                (2, old_range.1),
            ] {
                writer.execute("INSERT INTO notes(id,short_id,user_id,project_id,created_at,content,status) VALUES(?,?,'account-a','11111111-1111-4111-8111-111111111111',?,'Timer fixture','ready')",rusqlite::params![format!("timer-{id}"),id,date.to_rfc3339()]).unwrap();
            }
        }
        let moving_clock = clock.clone();
        let fixed_clock = clock.clone();
        let mut moving = TodayWatch::start_clock(
            host.db.clone(),
            host.user_id.clone(),
            destination.clone(),
            false,
            current.clone(),
            move || *moving_clock.lock().unwrap(),
        );
        let mut historical = TodayWatch::start_clock(
            host.db.clone(),
            host.user_id.clone(),
            destination,
            false,
            fixed,
            move || *fixed_clock.lock().unwrap(),
        );
        assert_eq!(
            snapshot(&mut moving, |s| s.rows.len() == 1).await.rows[0].id,
            1
        );
        assert_eq!(
            snapshot(&mut historical, |s| s.rows.len() == 1).await.rows[0].id,
            1
        );
        let after = old_range.1.with_timezone(&chrono::Local);
        *clock.lock().unwrap() = after;
        let updated = snapshot(&mut moving, |s| s.rows.len() == 1 && s.rows[0].id == 2).await;
        assert_eq!(updated.range, current.range(&after).unwrap());
        assert_ne!(updated.range, Some(old_range));
        host.db
            .writer()
            .await
            .unwrap()
            .execute(
                "UPDATE notes SET content='Fixed after boundary' WHERE short_id=1",
                [],
            )
            .unwrap();
        let anchored = snapshot(&mut historical, |s| {
            s.rows.iter().any(|r| r.content == "Fixed after boundary")
        })
        .await;
        assert_eq!(anchored.range, Some(old_range));
        assert_eq!(anchored.rows[0].id, 1);
        drop((moving, historical));
    }
    host.shutdown().await;
    server.abort();
}

async fn chart_snapshot(
    chart: &mut crate::creation_chart::ChartWatch,
    predicate: impl Fn(&crate::creation_chart::Snapshot) -> bool + Send + Sync,
) -> crate::creation_chart::Snapshot {
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            let result = chart.receiver.borrow_and_update().clone();
            if let Some(result) = result {
                let snapshot = result.expect("owned Chart query");
                if predicate(&snapshot) {
                    return snapshot;
                }
            }
            chart.receiver.changed().await.unwrap();
        }
    })
    .await
    .expect("Chart watch predicate")
}

#[tokio::test]
#[allow(clippy::too_many_lines)] // One owned host checks unbounded counts and canonical refresh.
async fn creation_chart_unbounded_owner_status_source_project_and_mutation_watch() {
    use crate::creation_chart::{ChartWatch, days};
    let (_root, config, _fake, _streams, server) = fixture().await;
    let host = start(config).await;
    let calendar = days(&chrono::Local::now()).unwrap();
    let start = calendar[0].range.0;
    let end = calendar[29].range.1;
    let stamp = calendar[29].range.0 + chrono::Duration::hours(1);
    {
        let writer = host.db.writer().await.unwrap();
        writer.execute("DELETE FROM notes", []).unwrap();
        writer.execute("DELETE FROM projects", []).unwrap();
        for (id, owner, name, color, archived) in [
            ("p1", "account-a", "Same", "#abcdef", 0),
            ("p2", "account-a", "Same", "invalid", 1),
            ("foreign", "account-b", "SECRET", "#ff0000", 0),
        ] {
            writer
                .execute(
                    "INSERT INTO projects(id,user_id,name,color,is_archived) VALUES(?,?,?,?,?)",
                    rusqlite::params![id, owner, name, color, archived],
                )
                .unwrap();
        }
        writer.execute_batch("BEGIN").unwrap();
        for i in 1..=10_017 {
            writer.execute("INSERT INTO notes(id,short_id,user_id,status,created_at,project_id,metadata) VALUES(?,?,'account-a',?,?,'p1','{}')",rusqlite::params![format!("chart-{i}"),i,if i%3==0 {"draft"} else if i%3==1 {"ai_queued"} else {"ready"},(stamp+chrono::Duration::milliseconds(i)).to_rfc3339()]).unwrap();
        }
        writer.execute_batch("COMMIT").unwrap();
        for (id, metadata) in [
            (20_001, "{}"),
            (20_002, "{\"created_by_ai\":false}"),
            (20_003, "{\"created_by_ai\":true}"),
            (20_004, "{\"created_by_ai\":\"true\"}"),
            (20_005, "{\"created_by_ai\":1}"),
            (20_006, "{\"created_by_ai\":null}"),
            (20_007, "{\"created_by_ai\":[]}"),
            (20_008, "{\"created_by\":\"ai\"}"),
        ] {
            writer.execute("INSERT INTO notes(id,short_id,user_id,status,created_at,metadata) VALUES(?,?,'account-a','draft',?,?)",rusqlite::params![format!("chart-{id}"),id,stamp.to_rfc3339(),metadata]).unwrap();
        }
        for (id, owner, project, date, deleted) in [
            (21_001, "account-a", Some("p2"), stamp, None),
            (21_002, "account-a", Some("missing"), stamp, None),
            (21_003, "account-a", Some("foreign"), stamp, None),
            (21_004, "account-b", Some("p1"), stamp, None),
            (21_005, "account-a", Some("p1"), stamp, Some("2026-01-01")),
            (21_006, "account-a", None, start, None),
            (
                21_007,
                "account-a",
                None,
                start - chrono::Duration::microseconds(1),
                None,
            ),
            (
                21_008,
                "account-a",
                None,
                end - chrono::Duration::microseconds(1),
                None,
            ),
            (21_009, "account-a", None, end, None),
        ] {
            writer.execute("INSERT INTO notes(id,short_id,user_id,status,project_id,created_at,deleted_at,metadata) VALUES(?,?,?,'ready',?,?,?,'{}')",rusqlite::params![format!("chart-{id}"),id,owner,project,date.to_rfc3339(),deleted]).unwrap();
        }
        writer.execute("INSERT INTO notes(id,user_id,created_at,metadata) VALUES('invalid-chart','account-a','nonsense','{}')",[]).unwrap();
    }
    let mut all = ChartWatch::start(host.db.clone(), host.user_id.clone(), false);
    let mut human = ChartWatch::start(host.db.clone(), host.user_id.clone(), true);
    let initial = chart_snapshot(&mut all, |s| s.total() == 10_030).await;
    let only = chart_snapshot(&mut human, |s| s.total() == 10_029).await;
    assert_eq!(initial.days.len(), 30);
    assert_eq!(initial.days[0].counts["unassigned"], 1);
    assert_eq!(
        initial.days.iter().filter(|d| d.counts.is_empty()).count(),
        28
    );
    let group = |key: &str| initial.groups.iter().find(|g| g.key == key).unwrap();
    assert_eq!(group("project:p1").total, 10_017);
    assert_eq!(group("project:p1").color, "#ABCDEF");
    assert_eq!(
        group("project:p2").name,
        "Same",
        "archived owned project label retained"
    );
    assert_eq!(group("project:foreign").name, "Unknown project");
    assert_eq!(group("project:missing").name, "Unknown project");
    assert_eq!(initial.projects.len(), 1);
    assert_eq!(
        only.groups
            .iter()
            .find(|g| g.key == "unassigned")
            .unwrap()
            .total,
        9
    );
    assert!(!format!("{initial:?}").contains("SECRET"));
    // Metadata/name/color changes and classification update the same UUID group, never creations.
    {
        let writer = host.db.writer().await.unwrap();
        writer
            .execute(
                "UPDATE projects SET name='Renamed',color='#123456' WHERE id='p1'",
                [],
            )
            .unwrap();
        writer
            .execute(
                "UPDATE notes SET project_id='p2',status='ready' WHERE short_id=1",
                [],
            )
            .unwrap();
    }
    let renamed = chart_snapshot(&mut all, |s| {
        s.groups
            .iter()
            .any(|g| g.name == "Renamed" && g.total == 10_016)
    })
    .await;
    assert_eq!(renamed.total(), initial.total());
    assert_eq!(
        renamed
            .groups
            .iter()
            .find(|g| g.key == "project:p1")
            .unwrap()
            .color,
        "#123456"
    );
    host.db
        .writer()
        .await
        .unwrap()
        .execute(
            "UPDATE notes SET type='normal',content='',is_flagged=0 WHERE short_id=1",
            [],
        )
        .unwrap();
    host.app
        .handle(AppRequest::NoteArchive { id: "1".into() })
        .await
        .unwrap();
    chart_snapshot(&mut all, |s| s.total() == 10_029).await;
    host.app
        .handle(AppRequest::NoteRestore { id: "1".into() })
        .await
        .unwrap();
    chart_snapshot(&mut all, |s| s.total() == 10_030).await;
    // A synced persisted creation is authoritative, including draft and absent short ID.
    host.db.writer().await.unwrap().execute("INSERT INTO notes(id,user_id,status,created_at,metadata) VALUES('new-chart','account-a','draft',?,'{}')",[stamp.to_rfc3339()]).unwrap();
    chart_snapshot(&mut all, |s| s.total() == 10_031).await;
    let receiver = all.receiver.clone();
    drop((all, human));
    tokio::time::timeout(Duration::from_secs(2), receiver.clone().changed())
        .await
        .unwrap()
        .unwrap_err();
    // Existing services remain available after the chart subscription is disposed.
    assert!(
        host.app
            .handle(AppRequest::NoteGet {
                id: "1".into(),
                archived: false
            })
            .await
            .is_ok()
    );
    host.shutdown().await;
    server.abort();
}

#[tokio::test]
async fn creation_chart_watch_rolls_at_local_four_without_database_write() {
    use crate::creation_chart::{ChartWatch, days};
    let (_root, config, _fake, _streams, server) = fixture().await;
    let host = start(config).await;
    let end = days(&chrono::Local::now()).unwrap()[29].range.1;
    let before = (end - chrono::Duration::milliseconds(700)).with_timezone(&chrono::Local);
    let clock = Arc::new(std::sync::Mutex::new(before));
    let old = days(&before).unwrap();
    {
        let writer = host.db.writer().await.unwrap();
        writer.execute("DELETE FROM notes", []).unwrap();
        for (id, date) in [("expires", old[0].range.0), ("enters", end)] {
            writer.execute("INSERT INTO notes(id,user_id,created_at,metadata) VALUES(?,'account-a',?,'{}')",[id.to_owned(),date.to_rfc3339()]).unwrap();
        }
    }
    let moving = clock.clone();
    let mut chart =
        ChartWatch::start_clock(host.db.clone(), host.user_id.clone(), false, move || {
            *moving.lock().unwrap()
        });
    let initial = chart_snapshot(&mut chart, |s| s.total() == 1).await;
    assert_eq!(initial.days[0].counts["unassigned"], 1);
    *clock.lock().unwrap() = end.with_timezone(&chrono::Local);
    let shifted = chart_snapshot(&mut chart, |s| s.range().0 != initial.range().0).await;
    assert_eq!(shifted.total(), 1);
    assert_eq!(shifted.days[29].counts["unassigned"], 1);
    assert_eq!(shifted.days[0].date, initial.days[1].date);
    drop(chart);
    host.shutdown().await;
    server.abort();
}

#[tokio::test]
async fn failed_watch_status_owner_source_and_membership_before_limit() {
    use crate::today::Destination;
    let (_root, config, _fake, _streams, server) = fixture().await;
    let host = start(config).await;
    seed_source_matrix(&host, "11111111-1111-4111-8111-111111111111").await;
    {
        let writer = host.db.writer().await.unwrap();
        writer.execute("UPDATE notes SET status='ai_failed' WHERE id LIKE 'source-%' OR id LIKE 'mcp-%' OR id IN ('foreign-source','deleted-source','old-source')", []).unwrap();
        writer
            .execute(
                "UPDATE notes SET status='source_failed' WHERE id='source-0'",
                [],
            )
            .unwrap();
        for (index, status) in ["draft", "source_queued", "ai_queued", "ready", "failed"]
            .iter()
            .enumerate()
        {
            writer.execute("INSERT INTO notes(id,user_id,short_id,content,status,created_at) VALUES(?,'account-a',?,'Nonfailure',?,'2020-01-01T00:00:00Z')", rusqlite::params![format!("other-{index}"), 20000 + index as i64, status]).unwrap();
        }
    }
    let mut mine = TodayWatch::start_destination(
        host.db.clone(),
        host.user_id.clone(),
        Destination::Failed,
        true,
    );
    let observed = snapshot(&mut mine, |s| s.rows.len() == 11).await;
    assert_eq!(observed.rows[0].uuid, "old-source");
    assert_eq!(observed.rows.last().unwrap().uuid, "source-0");
    assert!(observed.rows.windows(2).all(|r| r[0].id > r[1].id));
    assert!(observed.rows.iter().all(|r| !r.archived && !r.draft));
    assert!(
        observed
            .rows
            .iter()
            .all(|r| r.uuid == "old-source" || r.uuid.starts_with("source-"))
    );
    let mut full = TodayWatch::start_destination(
        host.db.clone(),
        host.user_id.clone(),
        Destination::Failed,
        false,
    );
    snapshot(&mut full, |s| s.rows.len() == crate::today::LIMIT).await;
    {
        let writer = host.db.writer().await.unwrap();
        writer
            .execute("UPDATE notes SET status='ready' WHERE id='source-0'", [])
            .unwrap();
        writer
            .execute(
                "UPDATE notes SET status='source_failed' WHERE id='other-0'",
                [],
            )
            .unwrap();
    }
    let observed = snapshot(&mut mine, |s| {
        s.rows.len() == 11
            && s.rows[0].uuid == "other-0"
            && s.rows.iter().all(|r| r.uuid != "source-0")
    })
    .await;
    assert!(!observed.rows[0].draft);
    host.db
        .writer()
        .await
        .unwrap()
        .execute(
            "UPDATE notes SET metadata='{\"created_by_ai\":true}' WHERE id='other-0'",
            [],
        )
        .unwrap();
    snapshot(&mut mine, |s| s.rows.len() == 10).await;
    drop((mine, full));
    host.shutdown().await;
    server.abort();
}

#[tokio::test]
#[allow(clippy::too_many_lines)] // Owned host checks exact local lifecycle and preservation matrix.
async fn retry_processing_exact_stage_active_account_and_preservation() {
    use crate::today::Destination;
    use flicknote_core::backend::{FailedStage, LocalPowerSyncBackend};
    let (_root, config, _fake, _streams, server) = fixture().await;
    let host = start(config).await;
    {
        let writer = host.db.writer().await.unwrap();
        for (id, stage, meta, owner, archived) in [
            (
                "ai",
                "ai_failed",
                Some(
                    r#"{"error":{"message":"bad"},"created_by_ai":true,"link":{"url":"kept"},"other":[1,false]}"#,
                ),
                "account-a",
                None,
            ),
            ("source", "source_failed", None, "account-a", None),
            ("foreign", "ai_failed", Some("{\"error\":1}"), "other", None),
            (
                "archived",
                "ai_failed",
                Some("{\"error\":2}"),
                "account-a",
                Some("2026-01-03"),
            ),
            ("ready", "ready", Some("{\"error\":3}"), "account-a", None),
            (
                "queued",
                "ai_queued",
                Some("{\"error\":4}"),
                "account-a",
                None,
            ),
            ("plain", "failed", Some("{\"error\":5}"), "account-a", None),
            (
                "opposite",
                "source_failed",
                Some("{\"error\":6}"),
                "account-a",
                None,
            ),
            ("duplicate", "ai_failed", Some("{}"), "account-a", None),
        ] {
            writer.execute("INSERT INTO notes(id,short_id,user_id,type,status,content,title,summary,source,project_id,is_flagged,metadata,created_at,updated_at,deleted_at) VALUES(?,(SELECT coalesce(max(short_id),0)+1 FROM notes),?,'link',?,'body','title','summary','{\"source\":true}','project',1,?,'2026-01-01','2026-01-02',?)", rusqlite::params![id,owner,stage,meta,archived]).unwrap();
        }
        writer.execute("INSERT INTO note_extractions(id,note_id,user_id,key,value) VALUES('extraction','ai','account-a','::topic','kept')", []).unwrap();
    }
    host.db
        .writer()
        .await
        .unwrap()
        .execute("DELETE FROM ps_crud", [])
        .unwrap();
    let local = LocalPowerSyncBackend::new(host.db.clone(), host.user_id.clone());
    let read = |id: &str| {
        let id = id.to_owned();
        let db = host.db.clone();
        async move {
            let reader = db.reader().await.unwrap();
            reader.query_row("SELECT json_object('owner',user_id,'type',type,'status',status,'content',content,'title',title,'summary',summary,'source',source,'project',project_id,'flag',is_flagged,'metadata',json(metadata),'created',created_at,'updated',updated_at,'deleted',deleted_at) FROM notes WHERE id=?", [id], |r| r.get::<_,String>(0)).map(|s| serde_json::from_str::<Value>(&s).unwrap()).unwrap()
        }
    };
    let mut watch = TodayWatch::start_destination(
        host.db.clone(),
        host.user_id.clone(),
        Destination::Failed,
        false,
    );
    snapshot(&mut watch, |s| {
        s.rows
            .iter()
            .any(|r| r.uuid == "ai" && r.failed_stage == Some(FailedStage::Ai))
    })
    .await;
    for (id, stage, queued) in [
        ("ai", FailedStage::Ai, "ai_queued"),
        ("source", FailedStage::Source, "source_queued"),
    ] {
        let mut expected = read(id).await;
        expected["status"] = json!(queued);
        if let Some(metadata) = expected["metadata"].as_object_mut() {
            metadata.remove("error");
        }
        assert!(
            host.app
                .retry_local_processing(&local, id, stage)
                .await
                .unwrap()
        );
        assert_eq!(
            read(id).await,
            expected,
            "all unrelated fields and both timestamps preserved"
        );
        let patch = host.db.next_crud_transaction().await.unwrap().unwrap();
        assert_eq!(patch.crud.len(), 1);
        assert_eq!(patch.crud[0].id, id);
        assert_eq!(patch.crud[0].table, "notes");
        assert!(matches!(
            patch.crud[0].update_type,
            powersync::UpdateType::Patch
        ));
        let data = patch.crud[0].data.as_ref().unwrap();
        assert_eq!(data["status"], json!(queued));
        assert!(!data.contains_key("created_at") && !data.contains_key("updated_at"));
        assert!(data.keys().all(|key| key == "status" || key == "metadata"));
        patch.complete().await.unwrap();
        assert!(
            !host
                .app
                .retry_local_processing(&local, id, stage)
                .await
                .unwrap()
        );
    }
    snapshot(&mut watch, |s| {
        s.rows.iter().all(|r| r.uuid != "ai" && r.uuid != "source")
    })
    .await;
    for id in [
        "foreign", "archived", "ready", "queued", "plain", "opposite",
    ] {
        let before = read(id).await;
        assert!(
            !host
                .app
                .retry_local_processing(&local, id, FailedStage::Ai)
                .await
                .unwrap()
        );
        assert_eq!(read(id).await, before);
    }
    assert!(
        !host
            .app
            .retry_local_processing(&local, "missing", FailedStage::Ai)
            .await
            .unwrap()
    );
    let other = LocalPowerSyncBackend::new(host.db.clone(), "other".into());
    assert!(
        !host
            .app
            .retry_local_processing(&other, "duplicate", FailedStage::Ai)
            .await
            .unwrap()
    );
    let (a, b) = tokio::join!(
        host.app
            .retry_local_processing(&local, "duplicate", FailedStage::Ai),
        host.app
            .retry_local_processing(&local, "duplicate", FailedStage::Ai)
    );
    assert_ne!(
        a.unwrap(),
        b.unwrap(),
        "exactly one atomic duplicate succeeds"
    );
    assert_eq!(
        host.db
            .reader()
            .await
            .unwrap()
            .query_row(
                "SELECT value FROM note_extractions WHERE id='extraction'",
                [],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
        "kept"
    );
    assert_eq!(
        host.app
            .observe_local_processing(&local, "opposite")
            .await
            .unwrap(),
        Some(FailedStage::Source)
    );
    assert_eq!(
        host.app
            .observe_local_processing(&local, "archived")
            .await
            .unwrap(),
        None
    );
    // A real SQLite error must propagate, never claim a retry completed.
    host.db
        .writer()
        .await
        .unwrap()
        .execute(
            "UPDATE notes SET metadata='invalid-json' WHERE id='opposite'",
            [],
        )
        .unwrap();
    assert!(
        host.app
            .retry_local_processing(&local, "opposite", FailedStage::Source)
            .await
            .is_err()
    );
    assert_eq!(read("ready").await["status"], json!("ready"));
    assert_eq!(
        host.app
            .observe_local_processing(&local, "opposite")
            .await
            .unwrap(),
        Some(FailedStage::Source)
    );
    drop(watch);
    host.shutdown().await;
    server.abort();
}
