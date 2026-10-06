use super::*;
use axum::{
    Json, Router,
    extract::State,
    http::{HeaderMap, StatusCode},
    routing::post,
};
use flicknote_client::{
    AppRequest, AppResponse,
    dto::{Patch, ProjectAddInput, ProjectModifyInput},
};
use std::sync::{
    Mutex,
    atomic::{AtomicUsize, Ordering},
};
#[derive(Default)]
struct Provider {
    requests: Mutex<Vec<Value>>,
    mode: AtomicUsize,
    active: AtomicUsize,
    max: AtomicUsize,
    gate: tokio::sync::Notify,
}
async fn answer(
    State(p): State<Arc<Provider>>,
    headers: HeaderMap,
    Json(v): Json<Value>,
) -> (StatusCode, Json<Value>) {
    assert_eq!(headers["authorization"], "Bearer fixture-secret");
    let active = p.active.fetch_add(1, Ordering::SeqCst) + 1;
    p.max.fetch_max(active, Ordering::SeqCst);
    p.requests.lock().unwrap().push(v.clone());
    let mode = p.mode.load(Ordering::SeqCst);
    if mode == 4 {
        p.gate.notified().await;
    }
    p.active.fetch_sub(1, Ordering::SeqCst);
    if mode == 2 {
        return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({})));
    }
    if mode == 3 {
        return (StatusCode::UNAUTHORIZED, Json(json!({})));
    }
    let answers:serde_json::Map<String,Value>=v["questions"].as_object().unwrap().keys().map(|k| (k.clone(),json!({"type":"choice","choice":if mode==5{"none"}else{"project_0"},"probabilities":{"project_0":0.8,"none":0.2}}))).collect();
    (
        StatusCode::OK,
        Json(if mode == 1 {
            json!({"answers":{}})
        } else {
            json!({"answers":answers})
        }),
    )
}
async fn server(p: Arc<Provider>) -> (String, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let endpoint = format!("http://{}/decisions", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/decisions", post(answer))
                .with_state(p),
        )
        .await
        .unwrap();
    });
    (endpoint, task)
}
async fn until(predicate: impl Fn() -> bool + Send + Sync) {
    tokio::time::timeout(Duration::from_secs(5), async {
        while !predicate() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
}
async fn project(host: &crate::spike::SpikeHost) -> String {
    host.db
        .writer()
        .await
        .unwrap()
        .execute("DELETE FROM projects", [])
        .unwrap();
    let AppResponse::Project(p) = host
        .app
        .handle(AppRequest::ProjectAdd(ProjectAddInput {
            name: "Research".into(),
            color: None,
        }))
        .await
        .unwrap()
    else {
        panic!()
    };
    host.app
        .handle(AppRequest::ProjectModify(ProjectModifyInput {
            id: p.id.clone(),
            color: Patch::Value("#123456".into()),
            summary: Patch::Value("Work on Rust".into()),
        }))
        .await
        .unwrap();
    p.id
}
async fn note(host: &crate::spike::SpikeHost, id: i64, summary: Option<&str>, content: &str) {
    host.db.writer().await.unwrap().execute("INSERT INTO notes(id,short_id,user_id,type,status,summary,title,content,metadata,created_at) VALUES (?1,?2,?3,'normal','ready',?4,'SECRET TITLE',?5,'{}','2026-10-06T01:00:00Z')",rusqlite::params![format!("00000000-0000-0000-0000-{id:012}"),id,crate::spike::USER,summary,content]).unwrap();
}
async fn count(host: &crate::spike::SpikeHost) -> i64 {
    host.db.writer().await.unwrap().query_row("SELECT count(*) FROM notes WHERE json_type(metadata,'$.project_routing.routed')='true'",[],|r|r.get(0)).unwrap()
}
fn credentials() -> (watch::Sender<Credential>, watch::Receiver<Credential>) {
    watch::channel(Credential {
        key: Some("fixture-secret".into()),
        generation: 1,
        catch_up: None,
    })
}
fn spawn(
    host: &crate::spike::SpikeHost,
    credential: watch::Receiver<Credential>,
    endpoint: String,
) -> (tokio::task::JoinHandle<()>, watch::Receiver<Option<String>>) {
    let db = host.db.clone();
    let app = host.app.clone();
    let (status, error) = watch::channel(None);
    (
        tokio::spawn(async move {
            run_with_endpoint(
                db,
                app,
                crate::spike::USER.into(),
                "2026-10-01T00:00:00Z".into(),
                RoutingControl {
                    credential,
                    status,
                    catch_up: watch::channel(CatchProgress::default()).0,
                },
                (&endpoint, Duration::from_millis(10)),
            )
            .await;
        }),
        error,
    )
}
async fn assert_hidden_mcp_routed(
    host: &crate::spike::SpikeHost,
    human: &mut crate::today::TodayWatch,
) {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if human
                .receiver
                .borrow_and_update()
                .as_ref()
                .is_some_and(|s| s.as_ref().is_ok_and(|s| s.rows.len() == 18))
            {
                break;
            }
            human.receiver.changed().await.unwrap();
        }
    })
    .await
    .unwrap();
    assert!(
        !human
            .receiver
            .borrow()
            .as_ref()
            .unwrap()
            .as_ref()
            .unwrap()
            .rows
            .iter()
            .any(|r| r.id == 19)
    );
    let metadata: String = host
        .db
        .writer()
        .await
        .unwrap()
        .query_row("SELECT metadata FROM notes WHERE short_id=19", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&metadata).unwrap()["created_by_ai"],
        true
    );
}
#[tokio::test]
async fn real_compact_choice_batches_drain_and_watch_membership() {
    let root = tempfile::tempdir().unwrap();
    let host = crate::spike::SpikeHost::start(root.path(), 0, 0, Duration::ZERO)
        .await
        .unwrap();
    let pid = project(&host).await;
    for id in 1..=17 {
        note(&host, id, Some(" Rust summary "), "NOT SENT body").await;
    }
    note(&host, 18, None, &"中".repeat(800)).await;
    note(&host, 19, Some("  "), " \n meaningful fallback ").await;
    note(&host, 20, None, " \n ").await;
    for (id, change) in [
        (21, "status='draft'"),
        (22, "status='ai_queued'"),
        (23, "deleted_at='2026-10-07'"),
        (24, "user_id='foreign'"),
        (25, "created_at='2026-09-01'"),
        (26, "metadata='{\"project_routing\":{\"routed\":true}}'"),
    ] {
        note(&host, id, None, "skip").await;
        host.db
            .writer()
            .await
            .unwrap()
            .execute(
                &format!("UPDATE notes SET {change} WHERE short_id={id}"),
                [],
            )
            .unwrap();
    }
    host.db.writer().await.unwrap().execute("UPDATE notes SET metadata='{\"created_by_ai\":true,\"project_routing\":{\"routed\":\"true\"}}' WHERE short_id=19",[]).unwrap();
    let watch = crate::today::TodayWatch::start_destination(
        host.db.clone(),
        crate::spike::USER.into(),
        crate::today::Destination::Project(pid.clone()),
        false,
    );
    let mut human = crate::today::TodayWatch::start_destination(
        host.db.clone(),
        crate::spike::USER.into(),
        crate::today::Destination::Project(pid.clone()),
        true,
    );
    let p = Arc::new(Provider::default());
    let (endpoint, server) = server(p.clone()).await;
    let (_key, credential) = credentials();
    let (task, _) = spawn(&host, credential, endpoint);
    tokio::time::timeout(Duration::from_secs(5), async {
        while count(&host).await != 20 {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    assert_hidden_mcp_routed(&host, &mut human).await;
    let requests = p.requests.lock().unwrap().clone();
    assert_eq!(requests.len(), 3);
    assert!(
        requests
            .iter()
            .all(|v| v["questions"].as_object().unwrap().len() <= 8
                && serde_json::to_vec(v).unwrap().len() <= BODY_LIMIT)
    );
    let combined = serde_json::to_string(&requests).unwrap();
    assert!(!combined.contains("SECRET TITLE"));
    assert!(!combined.contains("NOT SENT"));
    assert!(!combined.contains(&pid));
    assert!(!combined.contains("2026-"));
    assert!(!combined.contains("week"));
    assert!(!combined.contains("00000000"));
    assert!(combined.contains("Rust summary"));
    assert!(combined.contains("meaningful fallback"));
    let fallback = requests
        .iter()
        .flat_map(|v| v["questions"].as_object().unwrap().values())
        .find(|q| q["instructions"].as_str().unwrap().contains("中"))
        .unwrap()["instructions"]
        .as_str()
        .unwrap();
    assert_eq!(fallback.split('\n').next_back().unwrap().len(), 2046);
    until(|| {
        watch.receiver.borrow().as_ref().is_some_and(|s| {
            s.as_ref().is_ok_and(|s| {
                s.rows.len() == 19
                    && s.rows
                        .iter()
                        .all(|r| r.project_color.as_deref() == Some("#123456"))
            })
        })
    })
    .await;
    assert_eq!(p.max.load(Ordering::SeqCst), 1);
    task.abort();
    server.abort();
    drop(watch);
    host.run_until(async { Ok(()) }).await.unwrap();
}
#[tokio::test]
async fn invalid_answer_budget_unrelated_emission_auth_pause_and_none() {
    let root = tempfile::tempdir().unwrap();
    let host = crate::spike::SpikeHost::start(root.path(), 0, 0, Duration::ZERO)
        .await
        .unwrap();
    let pid = project(&host).await;
    note(&host, 1, None, "Rust").await;
    let p = Arc::new(Provider::default());
    p.mode.store(1, Ordering::SeqCst);
    let (endpoint, server) = server(p.clone()).await;
    let (key, credential) = credentials();
    let (task, error) = spawn(&host, credential, endpoint);
    until(|| {
        p.requests.lock().unwrap().len() == 3
            && error
                .borrow()
                .as_ref()
                .is_some_and(|s| s.contains("paused"))
    })
    .await;
    assert_eq!(count(&host).await, 0);
    host.app
        .handle(AppRequest::ProjectModify(ProjectModifyInput {
            id: pid,
            color: Patch::Value("#abcdef".into()),
            summary: Patch::Missing,
        }))
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(p.requests.lock().unwrap().len(), 3);
    p.mode.store(3, Ordering::SeqCst);
    key.send_replace(Credential {
        key: Some("fixture-secret".into()),
        generation: 2,
        catch_up: None,
    });
    until(|| {
        p.requests.lock().unwrap().len() == 4
            && error.borrow().as_ref().is_some_and(|s| s.contains("401"))
    })
    .await;
    host.db
        .writer()
        .await
        .unwrap()
        .execute(
            "UPDATE notes SET content='edited Rust' WHERE short_id=1",
            [],
        )
        .unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(p.requests.lock().unwrap().len(), 4);
    p.mode.store(5, Ordering::SeqCst);
    key.send_replace(Credential {
        key: Some("fixture-secret".into()),
        generation: 3,
        catch_up: None,
    });
    tokio::time::timeout(Duration::from_secs(5), async {
        while count(&host).await != 1 {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    let writer = host.db.writer().await.unwrap();
    let project: Option<String> = writer
        .query_row("SELECT project_id FROM notes WHERE short_id=1", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(project, None);
    drop(writer);
    task.abort();
    server.abort();
    host.run_until(async { Ok(()) }).await.unwrap();
}
#[tokio::test]
async fn delayed_response_manual_archive_key_and_quit_guards() {
    for catch in [false, true] {
        for race in ["manual", "project", "content", "archive", "key", "quit"] {
            let root = tempfile::tempdir().unwrap();
            let host = crate::spike::SpikeHost::start(root.path(), 0, 0, Duration::ZERO)
                .await
                .unwrap();
            let pid = project(&host).await;
            note(&host, 1, None, "Rust").await;
            let p = Arc::new(Provider::default());
            p.mode.store(4, Ordering::SeqCst);
            let (endpoint, server) = server(p.clone()).await;
            let (task, key, progress) = if catch {
                let (task, key, progress) = spawn_catch_up(&host, endpoint);
                (task, key, Some(progress))
            } else {
                let (key, credential) = credentials();
                let (task, _) = spawn(&host, credential, endpoint);
                (task, key, None)
            };
            until(|| !p.requests.lock().unwrap().is_empty()).await;
            match race {
                "manual" => {
                    host.db
                        .writer()
                        .await
                        .unwrap()
                        .execute("UPDATE notes SET project_id=? WHERE short_id=1", [&pid])
                        .unwrap();
                }
                "project" => {
                    host.app
                        .handle(AppRequest::ProjectArchive { id: pid })
                        .await
                        .unwrap();
                }
                "content" => {
                    host.db
                        .writer()
                        .await
                        .unwrap()
                        .execute("UPDATE notes SET content='  ' WHERE short_id=1", [])
                        .unwrap();
                }
                "archive" => {
                    host.db
                        .writer()
                        .await
                        .unwrap()
                        .execute(
                            "UPDATE notes SET deleted_at='2026-10-06T02:00:00Z' WHERE short_id=1",
                            [],
                        )
                        .unwrap();
                }
                "key" => {
                    key.send_replace(Credential {
                        key: None,
                        generation: 2,
                        catch_up: None,
                    });
                }
                _ => task.abort(),
            }
            p.gate.notify_one();
            tokio::time::sleep(Duration::from_millis(100)).await;
            assert_eq!(count(&host).await, 0, "{race}");
            if let Some(progress) = progress {
                assert_eq!(progress.borrow().processed, 0, "{race}");
                if matches!(race, "manual" | "archive" | "content") {
                    assert_eq!(progress.borrow().skipped, 1, "{race}");
                }
            }
            task.abort();
            server.abort();
            host.run_until(async { Ok(()) }).await.unwrap();
        }
    }
}

#[test]
fn exact_answer_keys_types_probabilities_and_payload_budget() {
    let projects = vec![Project {
        id: "canonical".into(),
        name: "p".into(),
        summary: None,
    }];
    let notes = vec![Note {
        id: 12,
        uuid: "canonical note".into(),
        regular: true,
        catch_up: false,
        text: "summary".into(),
    }];
    let b = Batch::new(&projects, &notes).unwrap();
    for answer in [
        json!({"answers":{"wrong":{"type":"choice","choice":"none","probabilities":{"none":0.8}}}}),
        json!({"answers":{"note_0":{"choice":"none","probabilities":{"none":0.8}}}}),
        json!({"answers":{"note_0":{"type":"choice","choice":"unknown","probabilities":{"unknown":0.8}}}}),
        json!({"answers":{"note_0":{"type":"choice","choice":"none","probabilities":{"none":1.1}}}}),
    ] {
        assert!(b.routes(&answer).is_err());
    }
    let huge = vec![Project {
        id: "c".into(),
        name: "p".into(),
        summary: Some("x".repeat(BODY_LIMIT)),
    }];
    assert!(Batch::new(&huge, &[]).is_err());
    assert!(Batch::new(&huge, &notes).is_err());
}

#[tokio::test]
async fn missing_key_catchup_ready_transition_transient_retry_and_oversized_projects() {
    let root = tempfile::tempdir().unwrap();
    let host = crate::spike::SpikeHost::start(root.path(), 0, 0, Duration::ZERO)
        .await
        .unwrap();
    let pid = project(&host).await;
    note(&host, 1, None, "ready Rust").await;
    let p = Arc::new(Provider::default());
    p.mode.store(2, Ordering::SeqCst);
    let (endpoint, server) = server(p.clone()).await;
    let (key, credential) = watch::channel(Credential {
        key: None,
        generation: 1,
        catch_up: None,
    });
    let (task, error) = spawn(&host, credential, endpoint);
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(p.requests.lock().unwrap().is_empty());
    key.send_replace(Credential {
        key: Some("fixture-secret".into()),
        generation: 2,
        catch_up: None,
    });
    until(|| p.requests.lock().unwrap().len() == 1 && error.borrow().is_some()).await;
    p.mode.store(0, Ordering::SeqCst);
    tokio::time::timeout(Duration::from_secs(5), async {
        while count(&host).await != 1 {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(p.requests.lock().unwrap().len(), 2);
    key.send_replace(Credential {
        key: None,
        generation: 3,
        catch_up: None,
    });
    note(&host, 2, None, "later Rust").await;
    host.db
        .writer()
        .await
        .unwrap()
        .execute("UPDATE notes SET status='ai_queued' WHERE short_id=2", [])
        .unwrap();
    key.send_replace(Credential {
        key: Some("fixture-secret".into()),
        generation: 4,
        catch_up: None,
    });
    tokio::time::sleep(Duration::from_millis(50)).await;
    let prior = p.requests.lock().unwrap().len();
    host.db
        .writer()
        .await
        .unwrap()
        .execute("UPDATE notes SET status='ready' WHERE short_id=2", [])
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        while count(&host).await != 2 {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(p.requests.lock().unwrap().len(), prior + 1);
    host.app
        .handle(AppRequest::ProjectModify(ProjectModifyInput {
            id: pid,
            color: Patch::Missing,
            summary: Patch::Value("x".repeat(BODY_LIMIT)),
        }))
        .await
        .unwrap();
    note(&host, 3, None, "cannot fit").await;
    until(|| {
        error
            .borrow()
            .as_ref()
            .is_some_and(|e| e.contains("too large"))
    })
    .await;
    let prior = p.requests.lock().unwrap().len();
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(p.requests.lock().unwrap().len(), prior);
    assert_eq!(count(&host).await, 2);
    task.abort();
    server.abort();
    host.run_until(async { Ok(()) }).await.unwrap();
}

#[tokio::test]
async fn changes_in_verification_to_transaction_gap_never_route_stale_input() {
    for change in ["content", "summary", "project"] {
        let root = tempfile::tempdir().unwrap();
        let host = crate::spike::SpikeHost::start(root.path(), 0, 0, Duration::ZERO)
            .await
            .unwrap();
        let pid = project(&host).await;
        note(&host, 1, None, "Original compact text").await;
        let params = [
            crate::spike::USER.into(),
            "2026-10-01T00:00:00Z".into(),
            String::new(),
            String::new(),
        ];
        let snapshot = host
            .db
            .writer()
            .await
            .unwrap()
            .prepare(sql())
            .and_then(|mut s| read(&mut s, &params))
            .unwrap();
        let batch = Batch::new(&snapshot.projects, &snapshot.notes).unwrap();
        let (_, credential) = credentials();
        let entered = tokio::sync::Notify::new();
        let release = tokio::sync::Notify::new();
        let response = Ok(vec![NoteRouteProjectInput {
            note_id: 1,
            project_id: Some(pid.clone()),
            probability: 0.8,
        }]);
        let mutate = async {
            entered.notified().await;
            let writer = host.db.writer().await.unwrap();
            match change {
                "content" => writer.execute("UPDATE notes SET content='Changed compact content' WHERE short_id=1", []).unwrap(),
                "summary" => writer.execute("UPDATE notes SET summary='Changed compact summary' WHERE short_id=1", []).unwrap(),
                _ => writer.execute("UPDATE projects SET metadata=json_set(metadata,'$.summary','Changed project summary') WHERE id=?", [&pid]).unwrap(),
            };
            drop(writer);
            release.notify_one();
        };
        let (result, ()) = tokio::time::timeout(Duration::from_secs(5), async {
            tokio::join!(
                apply_response(
                    &host.db,
                    &host.app,
                    &params,
                    &batch,
                    (1, &credential),
                    response,
                    Some((&entered, &release))
                ),
                mutate
            )
        })
        .await
        .expect("gated routing must finish");
        assert_eq!(
            count(&host).await,
            0,
            "{change}: stale decision marked routed"
        );
        assert!(!result.unwrap_or_else(|_| panic!("route failed")).1);
        let detail = host
            .app
            .handle(AppRequest::NoteGet {
                id: "1".into(),
                archived: false,
            })
            .await
            .unwrap();
        let AppResponse::NoteDetail(detail) = detail else {
            panic!("note detail")
        };
        assert!(detail.note.project.is_none(), "{change}");
        host.run_until(async { Ok(()) }).await.unwrap();
    }
}

#[test]
fn catch_up_semantic_days_and_frozen_end() {
    use chrono::TimeZone as _;
    let zone = chrono_tz::Asia::Shanghai;
    let now = zone.with_ymd_and_hms(2026, 10, 6, 16, 0, 0).unwrap();
    let three = CatchUp::recent(&now, 3, true).unwrap();
    let seven = CatchUp::recent(&now, 7, false).unwrap();
    assert_eq!(three.start, "2026-10-03T20:00:00+00:00");
    assert_eq!(seven.start, "2026-09-29T20:00:00+00:00");
    assert_eq!(three.end, "2026-10-06T08:00:00+00:00");
    let early = zone.with_ymd_and_hms(2026, 10, 6, 3, 0, 0).unwrap();
    assert_eq!(
        CatchUp::recent(&early, 3, true).unwrap().start,
        "2026-10-02T20:00:00+00:00"
    );
    let zone = chrono_tz::America::New_York;
    let now = zone.with_ymd_and_hms(2026, 11, 2, 4, 0, 0).unwrap();
    let range = CatchUp::recent(&now, 3, true).unwrap();
    let start = chrono::DateTime::parse_from_rfc3339(&range.start).unwrap();
    assert_eq!(now.signed_duration_since(start).num_hours(), 49);
    assert!(CatchUp::recent(&now, 5, true).is_err());
}

async fn wall_until(predicate: impl Fn() -> bool + Send + Sync) {
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while !predicate() {
        assert!(std::time::Instant::now() < deadline, "owned actor deadline");
        tokio::task::yield_now().await;
        std::thread::sleep(Duration::from_millis(1));
    }
}

#[tokio::test]
#[allow(clippy::too_many_lines)] // One owned actor verifies priority, cadence and complete drain together.
async fn catch_up_preview_priority_drain_shared_cadence_and_none() {
    let root = tempfile::tempdir().unwrap();
    let host = crate::spike::SpikeHost::start(root.path(), 0, 0, Duration::ZERO)
        .await
        .unwrap();
    project(&host).await;
    for id in 1..=17 {
        note(&host, id, None, &format!("Historical {id}")).await;
    }
    note(&host, 18, None, "Regular priority").await;
    let writer = host.db.writer().await.unwrap();
    writer
        .execute(
            "UPDATE notes SET created_at='2026-10-06T03:00:00Z' WHERE short_id=18",
            [],
        )
        .unwrap();
    writer
        .execute(
            r#"UPDATE notes SET metadata='{"created_by_ai":true}' WHERE short_id=17"#,
            [],
        )
        .unwrap();
    drop(writer);
    let provider = Arc::new(Provider::default());
    provider.mode.store(5, Ordering::SeqCst);
    let (endpoint, server) = server(provider.clone()).await;
    let range = CatchUp {
        days: 3,
        start: "2026-10-03T20:00:00Z".into(),
        end: "2026-10-06T02:00:00Z".into(),
        running: false,
    };
    let (key, credential) = watch::channel(Credential {
        key: None,
        generation: 1,
        catch_up: Some(range.clone()),
    });
    let (status, _) = watch::channel(None);
    let (progress, result) = watch::channel(CatchProgress::default());
    let db = host.db.clone();
    let app = host.app.clone();
    let task = tokio::spawn(async move {
        run_with_endpoint(
            db,
            app,
            crate::spike::USER.into(),
            "2026-10-06T02:30:00Z".into(),
            RoutingControl {
                credential,
                status,
                catch_up: progress,
            },
            (&endpoint, Duration::from_secs(2)),
        )
        .await;
    });
    wall_until(|| result.borrow().remaining == 17).await;
    assert!(
        provider.requests.lock().unwrap().is_empty(),
        "preview costs no requests"
    );
    tokio::time::pause();
    key.send_replace(Credential {
        key: Some("fixture-secret".into()),
        generation: 2,
        catch_up: Some(CatchUp {
            running: true,
            ..range
        }),
    });
    wall_until(|| result.borrow().processed == 7).await;
    assert_eq!(provider.requests.lock().unwrap().len(), 1);
    assert!(
        provider.requests.lock().unwrap()[0]["questions"]
            .as_object()
            .unwrap()
            .values()
            .any(|q| q["instructions"]
                .as_str()
                .unwrap()
                .contains("Regular priority"))
    );
    tokio::time::advance(Duration::from_secs(9)).await;
    for _ in 0..20 {
        tokio::task::yield_now().await;
    }
    assert_eq!(provider.requests.lock().unwrap().len(), 1);
    tokio::time::advance(Duration::from_millis(1001)).await;
    wall_until(|| result.borrow().processed == 15).await;
    tokio::time::advance(Duration::from_millis(10001)).await;
    wall_until(|| result.borrow().phase == CatchPhase::Complete).await;
    assert_eq!(result.borrow().processed, 17);
    assert_eq!(result.borrow().remaining, 0);
    assert_eq!(result.borrow().skipped, 0);
    assert_eq!(provider.requests.lock().unwrap().len(), 3);
    assert_eq!(provider.max.load(Ordering::SeqCst), 1);
    tokio::time::resume();
    assert_eq!(count(&host).await, 18);
    let writer = host.db.writer().await.unwrap();
    assert_eq!(
        writer
            .query_row(
                "SELECT count(*) FROM notes WHERE project_id IS NOT NULL",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
    assert_eq!(
        writer
            .query_row(
                "SELECT json_type(metadata,'$.created_by_ai') FROM notes WHERE short_id=17",
                [],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
        "true"
    );
    drop(writer);
    task.abort();
    server.abort();
    host.run_until(async { Ok(()) }).await.unwrap();
}

fn spawn_catch_up(
    host: &crate::spike::SpikeHost,
    endpoint: String,
) -> (
    tokio::task::JoinHandle<()>,
    watch::Sender<Credential>,
    watch::Receiver<CatchProgress>,
) {
    let (key, credential) = watch::channel(Credential {
        key: Some("fixture-secret".into()),
        generation: 1,
        catch_up: Some(CatchUp {
            days: 3,
            start: "2026-10-03T20:00:00Z".into(),
            end: "2026-10-06T02:00:00Z".into(),
            running: true,
        }),
    });
    let (status, _) = watch::channel(None);
    let (progress, result) = watch::channel(CatchProgress::default());
    let db = host.db.clone();
    let app = host.app.clone();
    let task = tokio::spawn(async move {
        run_with_endpoint(
            db,
            app,
            crate::spike::USER.into(),
            "2026-10-06T02:30:00Z".into(),
            RoutingControl {
                credential,
                status,
                catch_up: progress,
            },
            (&endpoint, Duration::from_secs(2)),
        )
        .await;
    });
    (task, key, result)
}

#[tokio::test]
async fn catch_up_bounded_failures_do_not_starve_independent_batches() {
    let root = tempfile::tempdir().unwrap();
    let host = crate::spike::SpikeHost::start(root.path(), 0, 0, Duration::ZERO)
        .await
        .unwrap();
    project(&host).await;
    for id in 1..=17 {
        note(&host, id, None, &format!("Historical {id}")).await;
    }
    let provider = Arc::new(Provider::default());
    provider.mode.store(2, Ordering::SeqCst);
    let (endpoint, server) = server(provider.clone()).await;
    tokio::time::pause();
    let (task, _key, result) = spawn_catch_up(&host, endpoint);
    wall_until(|| provider.requests.lock().unwrap().len() == 1).await;
    for expected in 2..=9 {
        // Let the prior HTTP failure reach the actor before advancing its clock.
        for _ in 0..50 {
            tokio::task::yield_now().await;
            std::thread::sleep(Duration::from_millis(1));
        }
        tokio::time::advance(Duration::from_millis(10001)).await;
        wall_until(|| provider.requests.lock().unwrap().len() == expected).await;
    }
    wall_until(|| result.borrow().phase == CatchPhase::Unfinished).await;
    assert_eq!(result.borrow().processed, 0);
    assert_eq!(result.borrow().remaining, 17);
    assert_eq!(result.borrow().failed, 17);
    tokio::time::advance(Duration::from_secs(100)).await;
    for _ in 0..30 {
        tokio::task::yield_now().await;
    }
    assert_eq!(provider.requests.lock().unwrap().len(), 9);
    assert_eq!(provider.max.load(Ordering::SeqCst), 1);
    tokio::time::resume();
    assert_eq!(count(&host).await, 0);
    task.abort();
    server.abort();
    host.run_until(async { Ok(()) }).await.unwrap();
}

#[tokio::test]
async fn catch_up_stop_and_credential_changes_cancel_old_generation() {
    for enabled in [true, false] {
        let root = tempfile::tempdir().unwrap();
        let host = crate::spike::SpikeHost::start(root.path(), 0, 0, Duration::ZERO)
            .await
            .unwrap();
        project(&host).await;
        note(&host, 1, None, "Historical").await;
        let provider = Arc::new(Provider::default());
        provider.mode.store(4, Ordering::SeqCst);
        let (endpoint, server) = server(provider.clone()).await;
        let (task, key, result) = spawn_catch_up(&host, endpoint);
        until(|| provider.requests.lock().unwrap().len() == 1).await;
        key.send_replace(Credential {
            key: enabled.then(|| "fixture-secret".into()),
            generation: 2,
            catch_up: None,
        });
        until(|| result.borrow().phase == CatchPhase::Stopped).await;
        provider.gate.notify_one();
        tokio::time::sleep(Duration::from_millis(30)).await;
        assert_eq!(count(&host).await, 0);
        assert_eq!(result.borrow().remaining, 1);
        assert_eq!(provider.requests.lock().unwrap().len(), 1);
        task.abort();
        server.abort();
        host.run_until(async { Ok(()) }).await.unwrap();
    }
}

fn attempts_for(provider: &Provider, input: &str) -> usize {
    provider
        .requests
        .lock()
        .unwrap()
        .iter()
        .filter(|request| {
            request["questions"]
                .as_object()
                .unwrap()
                .values()
                .any(|question| question["instructions"].as_str().unwrap().contains(input))
        })
        .count()
}

#[tokio::test]
async fn review_regular_arrival_does_not_reset_unchanged_catch_up_failure_budget() {
    let root = tempfile::tempdir().unwrap();
    let host = crate::spike::SpikeHost::start(root.path(), 0, 0, Duration::ZERO)
        .await
        .unwrap();
    project(&host).await;
    for id in 1..=16 {
        note(&host, id, None, &format!("Historical {id} END")).await;
    }
    let provider = Arc::new(Provider::default());
    provider.mode.store(2, Ordering::SeqCst);
    let (endpoint, server) = server(provider.clone()).await;
    tokio::time::pause();
    let (task, _key, result) = spawn_catch_up(&host, endpoint);
    wall_until(|| provider.requests.lock().unwrap().len() == 1).await;
    for expected in 2..=4 {
        for _ in 0..50 {
            tokio::task::yield_now().await;
            std::thread::sleep(Duration::from_millis(1));
        }
        tokio::time::advance(Duration::from_millis(10001)).await;
        wall_until(|| provider.requests.lock().unwrap().len() == expected).await;
    }
    assert_eq!(result.borrow().phase, CatchPhase::Running);
    host.db.writer().await.unwrap().execute(
        "INSERT INTO notes(id,short_id,user_id,type,status,content,metadata,created_at) VALUES('00000000-0000-0000-0000-000000000018',18,?1,'normal','ready','Unrelated regular note','{}','2026-10-06T03:00:00Z')",
        [crate::spike::USER],
    ).unwrap();
    for _ in 0..50 {
        tokio::task::yield_now().await;
        std::thread::sleep(Duration::from_millis(1));
    }
    tokio::time::advance(Duration::from_millis(10001)).await;
    wall_until(|| provider.requests.lock().unwrap().len() == 5).await;
    let attempts = attempts_for(&provider, "Historical 1 END");
    assert_eq!(
        attempts, 3,
        "unrelated arrival preserves the exhausted budget"
    );
    provider.mode.store(0, Ordering::SeqCst);
    for expected in 6..=7 {
        for _ in 0..50 {
            tokio::task::yield_now().await;
            std::thread::sleep(Duration::from_millis(1));
        }
        tokio::time::advance(Duration::from_millis(10001)).await;
        wall_until(|| provider.requests.lock().unwrap().len() == expected).await;
    }
    wall_until(|| result.borrow().phase == CatchPhase::Unfinished).await;
    assert_eq!(result.borrow().processed, 8);
    assert_eq!(result.borrow().remaining, 8);
    assert_eq!(result.borrow().failed, 8);
    assert_eq!(
        count(&host).await,
        9,
        "regular and independent historical group drain"
    );
    assert_eq!(provider.max.load(Ordering::SeqCst), 1);
    assert_eq!(attempts_for(&provider, "Historical 1 END"), 3);
    task.abort();
    server.abort();
    tokio::time::resume();
    host.run_until(async { Ok(()) }).await.unwrap();
}
