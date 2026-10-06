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
                credential,
                status,
                (&endpoint, Duration::from_millis(10)),
            )
            .await;
        }),
        error,
    )
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
    for race in ["manual", "project", "content", "key", "quit"] {
        let root = tempfile::tempdir().unwrap();
        let host = crate::spike::SpikeHost::start(root.path(), 0, 0, Duration::ZERO)
            .await
            .unwrap();
        let pid = project(&host).await;
        note(&host, 1, None, "Rust").await;
        let p = Arc::new(Provider::default());
        p.mode.store(4, Ordering::SeqCst);
        let (endpoint, server) = server(p.clone()).await;
        let (key, credential) = credentials();
        let (task, _) = spawn(&host, credential, endpoint);
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
            "key" => {
                key.send_replace(Credential {
                    key: None,
                    generation: 2,
                });
            }
            _ => task.abort(),
        }
        p.gate.notify_one();
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(count(&host).await, 0, "{race}");
        task.abort();
        server.abort();
        host.run_until(async { Ok(()) }).await.unwrap();
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
    });
    let (task, error) = spawn(&host, credential, endpoint);
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(p.requests.lock().unwrap().is_empty());
    key.send_replace(Credential {
        key: Some("fixture-secret".into()),
        generation: 2,
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
        let params = [crate::spike::USER.into(), "2026-10-01T00:00:00Z".into()];
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
