//! Owned PowerSync read-path work probe. Run explicitly; timings are descriptive only.
#![allow(clippy::print_stdout)] // Explicit owned-fixture work evidence.
#![allow(unsafe_code)] // Owned SQLite profile callback for bounded query-work evidence.
use crate::services::note::NoteService;
use flicknote_client::dto::NoteListInput;
use powersync::PowerSyncDatabase;
use rusqlite::params;
use std::ffi::{CStr, c_void};
use std::sync::Mutex;

#[derive(Default)]
struct Trace(Mutex<Vec<(String, i32)>>);

struct TraceSession<'a> {
    state: Box<Trace>,
    handles: Vec<*mut libsqlite3_sys::sqlite3>,
    _database: &'a PowerSyncDatabase,
}

impl<'a> TraceSession<'a> {
    async fn start(db: &'a PowerSyncDatabase) -> Self {
        let state = Box::<Trace>::default();
        let mut readers = Vec::new();
        for _ in 0..5 {
            readers.push(db.reader().await.unwrap());
        }
        let mut handles = Vec::new();
        for reader in &readers {
            // SAFETY: Exclusive leases; callback storage is heap-stable. This
            // owned, unsynced fixture has no concurrent tasks using its readers.
            let handle = unsafe {
                let handle = reader.handle();
                libsqlite3_sys::sqlite3_trace_v2(
                    handle,
                    libsqlite3_sys::SQLITE_TRACE_PROFILE,
                    Some(profile),
                    (&*state as *const Trace).cast_mut().cast(),
                );
                handle
            };
            handles.push(handle);
        }
        Self {
            state,
            handles,
            _database: db,
        }
    }
}

impl Drop for TraceSession<'_> {
    fn drop(&mut self) {
        for handle in &self.handles {
            // SAFETY: Database borrow keeps connections alive; sequential fixture
            // operations have returned their leases, including during unwinding.
            unsafe {
                libsqlite3_sys::sqlite3_trace_v2(*handle, 0, None, std::ptr::null_mut());
            }
        }
    }
}

// SAFETY: SQLite calls this synchronously with its live statement; the boxed
// Trace stays alive until tracing is explicitly removed from every connection.
unsafe extern "C" fn profile(
    _: u32,
    context: *mut c_void,
    stmt: *mut c_void,
    _: *mut c_void,
) -> i32 {
    unsafe {
        let sql = libsqlite3_sys::sqlite3_sql(stmt.cast());
        if !sql.is_null() {
            let sql = CStr::from_ptr(sql).to_string_lossy();
            if sql.trim_start().starts_with("SELECT") {
                let vm = libsqlite3_sys::sqlite3_stmt_status(
                    stmt.cast(),
                    libsqlite3_sys::SQLITE_STMTSTATUS_VM_STEP,
                    0,
                );
                if let Ok(mut records) = (&*context.cast::<Trace>()).0.lock() {
                    records.push((sql.into_owned(), vm));
                }
            }
        }
    }
    0
}

#[tokio::test]
#[ignore = "explicit synthetic work/timing evidence"]
async fn read_path_work_probe() {
    for n in [300, 3000] {
        for bytes in [16, 16384] {
            let (_directory, db, backend) = super::tests::make_powersync_backend().await;
            {
                let mut writer = db.writer().await.unwrap();
                let tx = writer.transaction().unwrap();
                tx.execute("INSERT INTO projects(id,user_id,name) VALUES ('project','test-user-id','Project')", []).unwrap();
                for id in 1..=n {
                    let uuid = format!("note-{id}");
                    tx.execute("INSERT INTO notes(id,short_id,user_id,type,status,content,source,project_id,updated_at) VALUES (?,?,?,'normal','ready',?,?,'project','2026-01-01T00:00:00Z')", params![uuid,id,"test-user-id","x".repeat(bytes),"y".repeat(bytes)]).unwrap();
                    for (key, value) in [
                        ("::topic", "alpha"),
                        ("::topic", "zeta"),
                        ("::product", "entity"),
                    ] {
                        tx.execute("INSERT INTO note_extractions(id,note_id,user_id,key,value) VALUES (?,?,?,?,?)", params![format!("{uuid}-{value}"),uuid,"test-user-id",key,value]).unwrap();
                    }
                }
                tx.commit().unwrap();
            }
            let capture = TraceSession::start(&db).await;
            let trace = &capture.state;
            let service = NoteService::new(&backend);
            for limit in [1, 20, 100] {
                for pass in 0..2 {
                    trace.0.lock().unwrap().clear();
                    let start = std::time::Instant::now();
                    let rows = service
                        .list(NoteListInput {
                            limit,
                            note_type: None,
                            status: None,
                            project: None,
                            no_project: false,
                            created_after: None,
                            created_before: None,
                            human: false,
                            archived: false,
                            shared: false,
                            cursor: None,
                        })
                        .await
                        .unwrap();
                    assert_eq!(rows.len(), limit as usize);
                    assert_eq!(rows[0].content_bytes, bytes as u64);
                    let elapsed = start.elapsed();
                    let records = trace.0.lock().unwrap();
                    assert_eq!(
                        records.len(),
                        3,
                        "one page, one topic batch, one project batch"
                    );
                    println!(
                        "n={n} body_source_bytes_each={bytes} limit={limit} pass={pass} selects={} vm={} elapsed_us={}",
                        records.len(),
                        records.iter().map(|(_, vm)| i64::from(*vm)).sum::<i64>(),
                        elapsed.as_micros()
                    );
                }
            }
            trace.0.lock().unwrap().clear();
            service.get(&n.to_string(), false).await.unwrap();
            assert_eq!(
                trace.0.lock().unwrap().len(),
                4,
                "get resolves, fetches, reads extractions once and resolves project"
            );
            for (sql, vm) in trace.0.lock().unwrap().iter() {
                println!("GET vm={vm} sql={sql}");
            }
            let reader = db.reader().await.unwrap();
            let schema: Vec<(String,String)> = reader.prepare("SELECT name,sql FROM sqlite_master WHERE type='index' AND sql IS NOT NULL AND tbl_name LIKE '%note_extractions%'").unwrap().query_map([],|r| Ok((r.get(0)?,r.get(1)?))).unwrap().collect::<Result<_,_>>().unwrap();
            println!("indexes={schema:?}");
            drop(reader);
        }
    }
}

fn list_input(limit: u32) -> NoteListInput {
    serde_json::from_value(serde_json::json!({"limit": limit})).unwrap()
}

async fn seed_discovery_contract(db: &PowerSyncDatabase) {
    let bodies = [
        None,
        Some("中文🙂\0after".to_string()),
        Some("x".repeat(32768)),
    ];
    {
        let writer = db.writer().await.unwrap();
        for (id, owner, name, archived) in [
            ("old", "test-user-id", "Archived", 1),
            ("other", "test-user-id", "Other", 0),
            ("foreign", "foreign-owner", "Secret", 0),
        ] {
            writer
                .execute(
                    "INSERT INTO projects(id,user_id,name,is_archived) VALUES (?,?,?,?)",
                    params![id, owner, name, archived],
                )
                .unwrap();
        }
        for id in 1_i64..=8 {
            let project = match id {
                1 | 2 => Some("old"),
                3 => Some("missing"),
                4 => Some("foreign"),
                8 => Some("other"),
                _ => None,
            };
            let owner = if id == 6 {
                "foreign-owner"
            } else {
                "test-user-id"
            };
            let status = if id == 5 { "draft" } else { "ready" };
            let deleted = if id == 7 {
                Some("2026-01-02T00:00:00Z")
            } else {
                None
            };
            let metadata = serde_json::json!({"nested": [id, "🙂"], "created_by_ai": id==3});
            writer.execute("INSERT INTO notes(id,short_id,user_id,type,status,content,source,project_id,metadata,title,summary,is_flagged,created_at,updated_at,deleted_at) VALUES (?,?,?,'normal',?,?,?,?,?,?,'summary',1,'2026-01-01T00:00:00Z',?,?)",params![format!("note-{id}"),id,owner,status,bodies[(id as usize-1)%3],"source".repeat(32768),project,metadata.to_string(),if id==2 {Some("")} else {None},format!("2026-01-01T00:00:0{id}Z"),deleted]).unwrap();
            for (key, value) in [
                ("::topic", "zeta"),
                ("::topic", "alpha"),
                ("::product", "entity"),
            ] {
                writer.execute("INSERT INTO note_extractions(id,note_id,user_id,key,value) VALUES (?,?,?,?,?)",params![format!("{id}-{value}"),format!("note-{id}"),owner,key,value]).unwrap();
            }
        }
        writer.execute("INSERT INTO note_extractions(id,note_id,user_id,key,value) VALUES ('foreign-extraction','note-2','foreign-owner','::topic','Secret')",[]).unwrap();
        writer.execute("INSERT INTO note_extractions(id,note_id,user_id,key,value) VALUES ('duplicate','note-2','test-user-id','::topic','alpha'),('null-key','note-2','test-user-id',NULL,'ignored'),('null-note',NULL,'test-user-id','::topic','ignored')",[]).unwrap();
    }
}

#[tokio::test]
async fn discovery_matches_detail_across_pages_and_structured_find() {
    use flicknote_client::dto::{ExtractionFilterDto, NoteFindInput, NoteListItem};
    let (_directory, db, backend) = super::tests::make_powersync_backend().await;
    seed_discovery_contract(&db).await;
    let capture = TraceSession::start(&db).await;
    let service = NoteService::new(&backend);
    for limit in [1, 20, 100] {
        capture.state.0.lock().unwrap().clear();
        service.list(list_input(limit)).await.unwrap();
        assert_eq!(capture.state.0.lock().unwrap().len(), 3);
    }
    capture.state.0.lock().unwrap().clear();
    service.get("8", false).await.unwrap();
    assert_eq!(capture.state.0.lock().unwrap().len(), 4);

    let mut all = Vec::new();
    let mut input = list_input(2);
    loop {
        let page = service.list(input.clone()).await.unwrap();
        if page.is_empty() {
            break;
        }
        input.cursor = page.last().unwrap().id;
        all.extend(page);
    }
    assert_eq!(
        all.iter().map(|n| n.id.unwrap()).collect::<Vec<_>>(),
        [8, 5, 4, 3, 2, 1]
    );
    for item in &all {
        let detail = service
            .get(&item.id.unwrap().to_string(), false)
            .await
            .unwrap();
        let mut expected: NoteListItem = detail.note.into();
        expected.metadata = detail.metadata;
        assert_eq!(item, &expected);
        let expected_topics = if item.id == Some(2) {
            vec!["alpha", "alpha", "zeta"]
        } else {
            vec!["alpha", "zeta"]
        };
        assert_eq!(item.topics, expected_topics);
        assert_eq!(detail.extractions.len(), expected_topics.len() + 1);
        assert_eq!(item.content_bytes, detail.content.len() as u64);
    }
    assert_eq!(all[0].project.as_deref(), Some("Other"));
    assert_eq!(all[2].project, None); // A foreign project cannot leak its name.
    assert_eq!(all[3].project, None); // Missing project stays missing.
    assert_eq!(all[4].project.as_deref(), Some("Archived"));
    assert_eq!(all[4].content_bytes, "中文🙂\0after".len() as u64);
    assert_eq!(all[5].content_bytes, 0);
    let input = NoteFindInput {
        keywords: vec![],
        extractions: vec![
            ExtractionFilterDto {
                key: "::topic".into(),
                value: "alpha".into(),
            },
            ExtractionFilterDto {
                key: "::product".into(),
                value: "entity".into(),
            },
        ],
        project: None,
        created_after: None,
        created_before: None,
        human: false,
        archived: false,
        limit: 20,
    };
    let found = service.find(input.clone()).await.unwrap();
    assert_eq!(
        found.iter().map(|n| n.id.unwrap()).collect::<Vec<_>>(),
        [8, 4, 3, 2, 1]
    );
    for item in found {
        assert_eq!(Some(&item), all.iter().find(|n| n.id == item.id));
    }
    let mut human = list_input(20);
    human.human = true;
    assert!(
        !service
            .list(human)
            .await
            .unwrap()
            .iter()
            .any(|n| n.id == Some(3))
    );
    let mut archived = list_input(20);
    archived.archived = true;
    let archive = service.list(archived).await.unwrap();
    assert_eq!(archive.len(), 1);
    assert_eq!(archive[0].id, Some(7));
    let mut search = input;
    search.archived = true;
    assert_eq!(service.find(search).await.unwrap(), archive);
}

#[tokio::test]
async fn extraction_work_is_bounded_by_page_not_account_size() {
    use super::{NoteDb, local::SQ_LIST_EXTRACTIONS};
    let mut measured = Vec::new();
    for n in [300, 3000] {
        let (_directory, db, backend) = super::tests::make_powersync_backend().await;
        {
            let writer = db.writer().await.unwrap();
            writer.execute("WITH RECURSIVE seq(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM seq WHERE x < ?) INSERT INTO note_extractions(id,note_id,user_id,key,value) SELECT 'extraction-'||x,'note-'||x,'test-user-id','::topic','alpha' FROM seq",[n]).unwrap();
        }
        let ids = (1..=20).map(|id| format!("note-{id}")).collect::<Vec<_>>();
        let ids_json = serde_json::to_string(&ids).unwrap();
        let keys_json = r#"["::topic"]"#;
        let reader = db.reader().await.unwrap();
        let mut stmt = reader.prepare(SQ_LIST_EXTRACTIONS).unwrap();
        let values = stmt
            .query_map(params!["test-user-id", keys_json, ids_json], |r| {
                r.get::<_, String>(2)
            })
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(values, vec!["alpha"; 20]);
        measured.push(stmt.get_status(rusqlite::StatementStatus::VmStep));
        let plan = reader
            .prepare(&format!("EXPLAIN QUERY PLAN {SQ_LIST_EXTRACTIONS}"))
            .unwrap()
            .query_map(params!["test-user-id", keys_json, ids_json], |r| {
                r.get::<_, String>(3)
            })
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        println!(
            "n={n} extraction_vm={} plan={plan:?}",
            measured.last().unwrap()
        );
        assert!(
            plan.iter().any(
                |step| step.contains("SEARCH") && step.contains("note_extractions_note_id_idx")
            ),
            "{plan:?}"
        );
        drop(stmt);
        drop(reader);
        assert_eq!(
            backend
                .list_note_topics(&ids.iter().map(String::as_str).collect::<Vec<_>>())
                .await
                .unwrap()
                .len(),
            20
        );
        assert!(backend.list_note_topics(&[]).await.unwrap().is_empty());
    }
    assert_eq!(
        measured[0], measured[1],
        "unrelated notes must not add extraction VM work"
    );
}
