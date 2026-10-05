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
use flicknote_client::{AppRequest, DaemonClient, dto::NoteAddInput};
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
    );
    for watch in [&mut home, &mut all] {
        let observed = snapshot(watch, |s| s.rows.len() == cases.len() + 1).await;
        assert_eq!(
            observed.rows.iter().map(|r| r.id).collect::<Vec<_>>(),
            (100..108).rev().chain([3]).collect::<Vec<_>>()
        );
        for (index, (content, _, preview, kind)) in cases.iter().enumerate() {
            let row = observed
                .rows
                .iter()
                .find(|r| r.id == 100 + index as i64)
                .unwrap();
            assert_eq!(&row.content, content);
            assert_eq!(&row.preview, preview);
            assert_eq!(&row.note_type, kind);
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
            assert_eq!(row.uuid, "preview-0");
            assert_eq!(observed.rows.len(), cases.len() + 1);
        }
    }
    drop(home);
    drop(all);
    host.shutdown().await;
    server.abort();
}
