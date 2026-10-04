#![allow(clippy::print_stderr)] // Fixture-only measurement evidence under --nocapture.
#![cfg(feature = "experimental-spike")]
use flicknote_client::{
    AppRequest, AppResponse, DaemonClient,
    dto::{NoteAddInput, NoteListInput},
};
use flicknote_sync::spike::{
    SpikeHost,
    today::{Snapshot, TodayWatch, bounds},
};
use serde_json::{Value, json};
use std::{sync::Arc, time::Duration};

fn add(content: &str) -> AppRequest {
    AppRequest::NoteAdd(NoteAddInput {
        content: content.into(),
        project: None,
        interpret_as_url: false,
        draft: false,
        topics: vec![],
        created_by: None,
        created_at: None,
    })
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
    .expect("bounded watched snapshot")
}
fn response(body: &str) -> Value {
    if body.starts_with('{') {
        serde_json::from_str(body).unwrap()
    } else {
        serde_json::from_str(
            body.lines()
                .find_map(|line| {
                    line.strip_prefix("data: ")
                        .filter(|data| data.starts_with('{'))
                })
                .expect("SSE data"),
        )
        .unwrap()
    }
}
struct Mcp {
    http: reqwest::Client,
    url: String,
    session: String,
}
impl Mcp {
    async fn connect(port: u16) -> Self {
        let http = reqwest::Client::new();
        let url = format!("http://127.0.0.1:{port}/mcp");
        let initialize = json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"synthetic-spike-test","version":"0"}}});
        for (header, value) in [("Origin", "https://evil.example"), ("Host", "evil.example")] {
            assert_eq!(
                http.post(&url)
                    .header(header, value)
                    .json(&initialize)
                    .send()
                    .await
                    .unwrap()
                    .status(),
                reqwest::StatusCode::FORBIDDEN
            );
        }
        let initialized = http
            .post(&url)
            .header("Accept", "application/json, text/event-stream")
            .json(&initialize)
            .send()
            .await
            .unwrap();
        assert!(initialized.status().is_success());
        let session = initialized.headers()["mcp-session-id"]
            .to_str()
            .unwrap()
            .to_owned();
        assert!(response(&initialized.text().await.unwrap())["result"]["serverInfo"].is_object());
        Self { http, url, session }
    }
    async fn request(&self, method: &str, params: Value) -> Value {
        let reply = self
            .http
            .post(&self.url)
            .header("Accept", "application/json, text/event-stream")
            .header("Mcp-Session-Id", &self.session)
            .json(&json!({"jsonrpc":"2.0","id":2,"method":method,"params":params}))
            .send()
            .await
            .unwrap();
        assert!(reply.status().is_success());
        response(&reply.text().await.unwrap())
    }
    async fn call(&self, name: &str, arguments: Value) -> Value {
        self.request("tools/call", json!({"name":name,"arguments":arguments}))
            .await
    }
}

#[tokio::test]
#[allow(clippy::too_many_lines)] // End-to-end ownership/persistence sequence shares one fixture.
async fn isolated_host_real_ipc_mcp_watch_ownership_and_persistence() {
    let root = tempfile::tempdir().unwrap();
    let host = SpikeHost::start(root.path(), 0, 5, Duration::ZERO)
        .await
        .unwrap();
    let socket = host.socket.clone();
    let port = host.mcp_port;
    let client = DaemonClient::new(&socket);
    let mut watch = TodayWatch::start(host.db.clone());
    let initial = snapshot(&mut watch, |s| s.rows.len() == 5).await;
    assert_eq!(
        initial.rows.iter().map(|r| r.id).collect::<Vec<_>>(),
        [5, 4, 3, 2, 1]
    );
    assert!(initial.rows.iter().all(|r| !r.preview.contains('\n')));
    assert_eq!(
        initial
            .rows
            .iter()
            .filter(|r| r.note_type == "link")
            .count(),
        1
    );
    assert!(initial.rows.iter().all(|r| r.project_color.is_some()));
    // Context belongs to the same watched projection; a project edit emits new row context.
    {
        let writer = host.db.writer().await.unwrap();
        writer
            .execute(
                "UPDATE projects SET color = '123456' WHERE id = 'fixture-reading'",
                [],
            )
            .unwrap();
    }
    let context = snapshot(&mut watch, |s| {
        s.rows
            .iter()
            .any(|r| r.project_color.as_deref() == Some("123456"))
    })
    .await;
    assert_eq!(context.rows.len(), 5);
    assert!(client.health().await.is_ok());
    let duplicate = SpikeHost::start(root.path(), 0, 0, Duration::ZERO)
        .await
        .err()
        .unwrap();
    assert!(duplicate.contains("already owns"));
    assert!(client.health().await.is_ok());
    let AppResponse::NoteCreate(created) =
        client.app(add("Human synthetic\n第二行")).await.unwrap()
    else {
        panic!()
    };
    let confirmed = snapshot(&mut watch, |s| s.rows.iter().any(|r| r.id == created.id)).await;
    assert_eq!(confirmed.rows[0].content, "Human synthetic\n第二行");
    let mcp = Mcp::connect(port).await;
    let tools = mcp.request("tools/list", json!({})).await;
    assert!(
        tools["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .any(|t| t["name"] == "note_add")
    );
    let rejected = mcp
        .call("note_add", json!({"content":"invalid draft", "draft":true}))
        .await;
    assert!(rejected.get("error").is_some() || rejected["result"]["isError"] == true);
    let created_mcp = mcp
        .call("note_add", json!({"content":"MCP synthetic"}))
        .await;
    let id = created_mcp["result"]["structuredContent"]["id"]
        .as_i64()
        .unwrap();
    snapshot(&mut watch, |s| s.rows.first().is_some_and(|r| r.id == id)).await;
    let detail = mcp.call("note_get", json!({"id":id})).await;
    assert_eq!(
        detail["result"]["structuredContent"]["content"],
        "MCP synthetic"
    );
    assert!(
        detail["result"]["structuredContent"]["metadata"]["created_by"]
            .as_str()
            .unwrap()
            .starts_with("mcp")
    );
    let AppResponse::NoteDetail(human) = client
        .app(AppRequest::NoteGet {
            id: created.id.to_string(),
            archived: false,
        })
        .await
        .unwrap()
    else {
        panic!()
    };
    assert!(
        human
            .metadata
            .as_ref()
            .is_none_or(|m| m.get("created_by").is_none())
    );
    let archived = mcp.call("note_archive", json!({"id":id})).await;
    assert_eq!(archived["result"]["structuredContent"]["archived"], true);
    snapshot(&mut watch, |s| !s.rows.iter().any(|r| r.id == id)).await;
    client
        .app(AppRequest::NoteArchive {
            id: created.id.to_string(),
        })
        .await
        .unwrap();
    snapshot(&mut watch, |s| s.rows.len() == 5).await;
    let mut after_close = watch.receiver.clone();
    drop(watch);
    assert!(
        after_close.changed().await.is_err(),
        "subscription teardown closes publication channel"
    );
    host.shutdown().await;
    assert!(!socket.exists());
    assert!(
        reqwest::Client::new()
            .get(format!("http://127.0.0.1:{port}/mcp"))
            .send()
            .await
            .is_err()
    );
    let host = SpikeHost::start(root.path(), 0, 500, Duration::ZERO)
        .await
        .unwrap();
    let AppResponse::NoteListItems(notes) = host
        .app
        .handle(AppRequest::NoteList(
            serde_json::from_value::<NoteListInput>(json!({"limit":100})).unwrap(),
        ))
        .await
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(
        notes.len(),
        5,
        "restart keeps existing fixtures without reseeding"
    );
    let mut reopened = TodayWatch::start(host.db.clone());
    snapshot(&mut reopened, |s| s.rows.len() == 5).await;
    drop(reopened);
    host.shutdown().await;
}

#[tokio::test]
async fn delayed_failure_and_10000_rows_with_concurrent_traffic() {
    let root = tempfile::tempdir().unwrap();
    let host = SpikeHost::start(root.path(), 0, 10_000, Duration::from_millis(150))
        .await
        .unwrap();
    let mut watch = TodayWatch::start(host.db.clone());
    let first = snapshot(&mut watch, |s| s.rows.len() == 10_000).await;
    eprintln!(
        "MEASURE seed=10000 first_snapshot_ms={:.3} emission={}",
        first.elapsed_ms, first.emission
    );
    let app = host.app.clone();
    let fail = tokio::spawn(async move { app.handle(add("[fixture-fail]")).await });
    assert!(!fail.is_finished());
    assert!(DaemonClient::new(&host.socket).health().await.is_ok());
    assert!(fail.await.unwrap().is_err());
    let mcp = Mcp::connect(host.mcp_port).await;
    let traffic = async {
        for _ in 0..20 {
            let reply = mcp.call("note_list", json!({"limit":5})).await;
            assert_eq!(
                reply["result"]["structuredContent"]["notes"]
                    .as_array()
                    .unwrap()
                    .len(),
                5
            );
        }
    };
    let started = std::time::Instant::now();
    let (burst, ()) = tokio::join!(host.burst(25), traffic);
    burst.unwrap();
    let last = snapshot(&mut watch, |s| {
        s.rows
            .iter()
            .any(|r| r.id == 1 && r.content.starts_with("Synthetic update batch 24"))
    })
    .await;
    assert!(last.rows.windows(2).all(|p| p[0].id > p[1].id));
    assert!(
        last.emission <= 27,
        "backpressure batches storage notifications"
    );
    eprintln!(
        "MEASURE batches=25 rows_per_batch=100 concurrent_mcp=20 elapsed_ms={:.3} watch_emissions={}",
        started.elapsed().as_secs_f64() * 1000.,
        last.emission
    );
    let before = Arc::clone(&last.rows);
    assert!(
        host.app
            .handle(AppRequest::NoteArchive {
                id: "999999".into()
            })
            .await
            .is_err()
    );
    assert_eq!(
        *watch
            .receiver
            .borrow()
            .as_ref()
            .unwrap()
            .as_ref()
            .unwrap()
            .rows,
        *before
    );
    drop(watch);
    host.shutdown().await;
}

#[test]
fn semantic_calendar_four_am_and_dst() {
    use chrono::{TimeZone, Timelike};
    let zone = chrono_tz::America::New_York;
    let before = zone.with_ymd_and_hms(2026, 3, 8, 3, 59, 59).unwrap();
    let at = zone.with_ymd_and_hms(2026, 3, 8, 4, 0, 0).unwrap();
    let (start, end) = bounds(&before).unwrap();
    assert_eq!((end - start).num_hours(), 23);
    assert_eq!(start.with_timezone(&zone).hour(), 4);
    assert_eq!(end.with_timezone(&zone), at);
    assert_eq!(bounds(&at).unwrap().0, end);
    let fall = zone.with_ymd_and_hms(2026, 11, 1, 3, 59, 59).unwrap();
    let (start, end) = bounds(&fall).unwrap();
    assert_eq!((end - start).num_hours(), 25);
}

#[tokio::test]
async fn empty_fixture_restarts_without_duplicating_project_context() {
    let root = tempfile::tempdir().unwrap();
    for count in [0, 0, 5] {
        let host = SpikeHost::start(root.path(), 0, count, Duration::ZERO)
            .await
            .unwrap();
        let mut watch = TodayWatch::start(host.db.clone());
        let rows = snapshot(&mut watch, |s| s.rows.len() == count as usize).await;
        assert!(rows.rows.iter().all(|row| row.project_color.is_some()));
        drop(watch);
        host.shutdown().await;
    }
}
