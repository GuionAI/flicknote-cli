//! Owned production LocalHost, canonical ShareGateway HTTP, pointer hit testing; no native launch.
use super::tests::settle;
use super::*;
use gpui_kit::{
    InputEvent as _, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, TestAppContext,
    point, test::TestWindowExt,
};
use std::sync::atomic::{AtomicU8, Ordering};

#[derive(Clone, Default)]
struct Gateway {
    db: Arc<Mutex<Option<flicknote_sync::PowerSyncDatabase>>>,
    calls: Arc<Mutex<Vec<(String, String)>>>,
    mode: Arc<AtomicU8>,
    gate: Arc<tokio::sync::Notify>,
}
async fn share_http(
    axum::extract::State(fake): axum::extract::State<Gateway>,
    axum::extract::Path(id): axum::extract::Path<String>,
    method: axum::http::Method,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    fake.calls
        .lock()
        .unwrap()
        .push((method.to_string(), id.clone()));
    let db = fake.db.lock().unwrap().as_ref().unwrap().clone();
    if method == axum::http::Method::GET {
        let exists = db
            .reader()
            .await
            .unwrap()
            .query_row("SELECT count(*) FROM note_shares WHERE id=?", [&id], |r| {
                r.get::<_, i64>(0)
            })
            .unwrap()
            > 0;
        if !exists {
            return (
                axum::http::StatusCode::NOT_FOUND,
                axum::Json(serde_json::json!({"errorCode":"SHARE_NOT_FOUND"})),
            )
                .into_response();
        }
    }
    let mode = fake.mode.load(Ordering::SeqCst);
    if mode == 1 {
        fake.gate.notified().await;
    }
    if mode == 2 {
        return (axum::http::StatusCode::SERVICE_UNAVAILABLE, "owned failure").into_response();
    }
    if method == axum::http::Method::POST && mode != 3 {
        db.writer().await.unwrap().execute("INSERT INTO note_shares(id,user_id,token,created_at) VALUES(?,'append-owner','owned-token','2026-01-01T00:00:00Z')", [&id]).unwrap();
    }
    if method == axum::http::Method::DELETE {
        db.writer()
            .await
            .unwrap()
            .execute("DELETE FROM note_shares WHERE id=?", [&id])
            .unwrap();
        return axum::http::StatusCode::NO_CONTENT.into_response();
    }
    if mode == 4 {
        fake.gate.notified().await;
    }
    axum::Json(serde_json::json!({"url":format!("https://owned.invalid/share/{id}")}))
        .into_response()
}
async fn create_http(
    axum::Json(mut note): axum::Json<serde_json::Value>,
) -> axum::Json<serde_json::Value> {
    note["short_id"] = serde_json::json!(5);
    note["summary"] = serde_json::Value::Null;
    note["source"] = serde_json::Value::Null;
    note["deleted_at"] = serde_json::Value::Null;
    axum::Json(serde_json::json!([note]))
}
fn fixture(
    runtime: &tokio::runtime::Runtime,
    root: &std::path::Path,
) -> (flicknote_sync::LocalHost, Gateway) {
    let fake = Gateway::default();
    let listener = runtime
        .block_on(tokio::net::TcpListener::bind((
            std::net::Ipv4Addr::LOCALHOST,
            0,
        )))
        .unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let router = axum::Router::new()
        .route("/rest/v1/notes", axum::routing::post(create_http))
        .route(
            "/api/v1/notes/{id}/share",
            axum::routing::get(share_http)
                .post(share_http)
                .delete(share_http),
        )
        .fallback(|| async { axum::http::StatusCode::SERVICE_UNAVAILABLE })
        .with_state(fake.clone());
    runtime.spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    let mut config = flicknote_core::profile::load(root).unwrap();
    config.supabase_url.clone_from(&origin);
    config.powersync_url.clone_from(&origin);
    config.api_url.clone_from(&origin);
    config.gateway_url = origin;
    flicknote_auth::session::save_session(
        &config.paths.session_file,
        &flicknote_auth::client::AuthSession {
            access_token: "owned-access".into(),
            refresh_token: "owned-refresh".into(),
            expires_at: Some(u64::MAX),
            user: flicknote_auth::client::AuthUser {
                id: "append-owner".into(),
                email: None,
            },
        },
    )
    .unwrap();
    let host = runtime
        .block_on(flicknote_sync::LocalHost::start(
            config,
            Some(0),
            tokio::sync::watch::channel(false).1,
        ))
        .unwrap();
    *fake.db.lock().unwrap() = Some(host.db.clone());
    runtime.block_on(async {
        let writer = host.db.writer().await.unwrap();
        for id in 1..=4 {
            writer.execute("INSERT INTO notes(id,short_id,user_id,content,type,status,title,metadata,created_at) VALUES(?,?,'append-owner',?,'normal','ready',?,'{\"created_by_ai\":false}',strftime('%Y-%m-%dT%H:%M:%SZ','now'))", [uuid(id), id.to_string(), "Owned content ".repeat(100), "Known colored long title ".repeat(100)]).unwrap();
        }
        for (id, name, color) in [(1,"Ideas",Some("123456")),(2,"Reading",Some("abcdef")),(3,"Colorless",None)] {
            writer.execute("INSERT INTO projects(id,user_id,name,color,is_archived) VALUES(?,'append-owner',?,?,0)", [project(id),name.to_owned(),color.unwrap_or_default().to_owned()]).unwrap();
        }
        writer.execute("UPDATE projects SET color=NULL WHERE name='Colorless'", []).unwrap();
        writer.execute("UPDATE notes SET project_id=? WHERE short_id=4", [project(1)]).unwrap();
    });
    (host, fake)
}
fn uuid(id: i64) -> String {
    format!("00000000-0000-4000-8000-{id:012}")
}
fn project(id: i64) -> String {
    format!("11111111-1111-4111-8111-{id:012}")
}
fn drag(window: &mut Window, cx: &mut App, id: u64, target: &str) {
    window.render_frame(cx);
    let from = window.find(("note", id)).bounds().center();
    let to = window
        .find(gpui_kit::SharedString::from(target.to_owned()))
        .bounds()
        .center();
    window.drag(from, to, cx);
}
fn pointer(window: &mut Window, cx: &mut App, id: u64) {
    window.render_frame(cx);
    let position = window.find(("note", id)).bounds().center();
    window.dispatch_event(
        MouseMoveEvent {
            position,
            pressed_button: None,
            modifiers: Default::default(),
        }
        .to_platform_input(),
        cx,
    );
    window.render_frame(cx);
}
fn ready(cx: &mut TestAppContext, view: &Entity<Today>, count: usize) {
    settle(cx, |cx| {
        cx.update(|cx| view.read(cx).loaded && view.read(cx).model.rows.len() == count)
    });
}

#[gpui_kit::test]
#[allow(clippy::too_many_lines)] // One owned pointer journey retains input/selection across native drag actions.
fn native_drag_classifies_archives_publishes_and_browses_collections(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let root = tempfile::tempdir().unwrap();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let (host, fake) = fixture(&runtime, root.path());
    let services = append_tests::services(&host, &runtime);
    cx.update(|cx| {
        gpui_kit::init(cx);
        install_today_keys(cx);
    });
    let (window, view) = append_tests::open(cx, services.clone());
    ready(cx, &view, 4);
    // Drag UUID3 while selection/reader belongs to UUID4; accepted drag must not click/retarget.
    cx.update_window(window.into(), |_, w, cx| {
        w.render_frame(cx);
        w.click(("note", 4_u64), cx);
        drag(w, cx, 3, &format!("project-{}", project(1)));
        assert_eq!(view.read(cx).model.selected, Some(4));
        assert_eq!(view.read(cx).detail.as_ref().unwrap().uuid, uuid(4));
        assert_eq!(
            view.read(cx)
                .model
                .rows
                .iter()
                .find(|r| r.id == 3)
                .unwrap()
                .project_color
                .as_deref(),
            Some("123456")
        );
    })
    .unwrap();
    settle(cx, |cx| cx.update(|cx| !view.read(cx).note_busy(&uuid(3))));
    assert_eq!(
        runtime
            .block_on(host.db.reader())
            .unwrap()
            .query_row("SELECT project_id FROM notes WHERE short_id=3", [], |r| {
                r.get::<_, String>(0)
            })
            .unwrap(),
        project(1)
    );
    cx.update_window(window.into(), |_, w, cx| {
        drag(w, cx, 3, &format!("project-{}", project(1)));
        assert!(!view.read(cx).note_busy(&uuid(3)), "same project no-op");
        drag(w, cx, 3, &format!("project-{}", project(2)));
    })
    .unwrap();
    settle(cx, |cx| cx.update(|cx| !view.read(cx).note_busy(&uuid(3))));
    cx.update_window(window.into(), |_, w, cx| {
        pointer(w, cx, 3);
        assert_eq!(view.read(cx).related_project(), Some(project(2).as_str()));
        w.press("alt-j", cx);
        assert_eq!(view.read(cx).related_project(), Some(project(2).as_str()));
        w.press("alt-j", cx);
        assert_eq!(
            view.read(cx).related_project(),
            None,
            "parked mouse loses to keyboard"
        );
        pointer(w, cx, 4);
        assert_eq!(view.read(cx).related_project(), Some(project(1).as_str()));
        w.render_frame(cx);
        let from = w.find(("note", 3_u64)).bounds().center();
        let selected = view.read(cx).model.selected;
        let detail_open = view.read(cx).detail_open;
        w.dispatch_event(
            MouseDownEvent {
                button: MouseButton::Left,
                position: from,
                modifiers: Default::default(),
                click_count: 1,
                first_mouse: false,
            }
            .to_platform_input(),
            cx,
        );
        w.render_frame(cx);
        w.dispatch_event(
            MouseMoveEvent {
                position: from + point(px(20.), px(0.)),
                pressed_button: Some(MouseButton::Left),
                modifiers: Default::default(),
            }
            .to_platform_input(),
            cx,
        );
        w.render_frame(cx);
        assert!(cx.has_active_drag());
        w.press("escape", cx);
        assert!(!cx.has_active_drag());
        w.dispatch_event(
            MouseUpEvent {
                button: MouseButton::Left,
                position: point(px(600.), px(20.)),
                modifiers: Default::default(),
                click_count: 1,
            }
            .to_platform_input(),
            cx,
        );
        w.render_frame(cx);
        assert_eq!(view.read(cx).model.selected, selected);
        assert_eq!(view.read(cx).detail_open, detail_open);
        assert!(!view.read(cx).note_busy(&uuid(3)));
        w.drag(from, point(px(600.), px(20.)), cx); // outside any target
        assert!(!view.read(cx).note_busy(&uuid(3)));
        drag(w, cx, 3, "home");
        assert!(!view.read(cx).note_busy(&uuid(3)));
        drag(w, cx, 3, "shared");
    })
    .unwrap();
    settle(cx, |cx| {
        cx.update(|cx| {
            !view.read(cx).note_busy(&uuid(3))
                && cx
                    .read_from_clipboard()
                    .and_then(|c| c.text())
                    .is_some_and(|s| s == format!("https://owned.invalid/share/{}", uuid(3)))
        })
    });
    assert_eq!(
        fake.calls.lock().unwrap().as_slice(),
        [("GET".into(), uuid(3)), ("POST".into(), uuid(3))]
    );
    cx.update_window(window.into(), |_, w, cx| {
        drag(w, cx, 3, "shared");
    })
    .unwrap();
    settle(cx, |cx| cx.update(|cx| !view.read(cx).note_busy(&uuid(3))));
    assert_eq!(
        fake.calls.lock().unwrap().len(),
        3,
        "already shared GET reuses link"
    );
    cx.update_window(window.into(), |_, w, cx| {
        w.click("shared", cx);
    })
    .unwrap();
    ready(cx, &view, 1);
    cx.update_window(window.into(), |_, w, cx| {
        drag(w, cx, 3, &format!("project-{}", project(1)));
    })
    .unwrap();
    settle(cx, |cx| cx.update(|cx| !view.read(cx).note_busy(&uuid(3))));
    assert_eq!(
        cx.update(|cx| view.read(cx).destination.clone()),
        Destination::Shared
    );
    cx.update_window(window.into(), |_, w, cx| {
        w.render_frame(cx);
        w.click(("note", 3_u64), cx);
        w.render_frame(cx);
        assert_eq!(w.find("share-note").label(), Some("Copy share link"));
        w.click("unshare-note", cx);
    })
    .unwrap();
    ready(cx, &view, 0);
    assert_eq!(
        cx.update(|cx| view.read(cx).destination.clone()),
        Destination::Shared
    );
    cx.update_window(window.into(), |_, w, cx| {
        w.click("home", cx);
    })
    .unwrap();
    ready(cx, &view, 4);
    cx.update_window(window.into(), |_, w, cx| {
        w.render_frame(cx);
        w.click(("note", 3_u64), cx);
        drag(w, cx, 3, "archive-destination");
        assert_eq!(
            view.read(cx).model.rows.len(),
            3,
            "accepted archive removes immediately"
        );
        assert_eq!(view.read(cx).model.selected, Some(2));
    })
    .unwrap();
    settle(cx, |cx| cx.update(|cx| !view.read(cx).note_busy(&uuid(3))));
    cx.update_window(window.into(), |_, w, cx| {
        w.click("archive-destination", cx);
    })
    .unwrap();
    ready(cx, &view, 1);
    cx.update_window(window.into(), |_, w, cx| {
        w.render_frame(cx);
        w.click(("note", 3_u64), cx);
        w.render_frame(cx);
        let composer = view.read(cx).composer.clone();
        assert_eq!(
            composer.read(cx).presentation().placeholder(),
            "Create a new note"
        );
        assert!(
            !view
                .read(cx)
                .drag_eligible(&view.read(cx).model.rows[0], cx)
        );
        assert!(w.try_find("share-note").is_none());
        w.click("archive", cx);
    })
    .unwrap();
    ready(cx, &view, 0);
    settle(cx, |cx| cx.update(|cx| !view.read(cx).note_busy(&uuid(3))));
    cx.update_window(window.into(), |_, w, cx| {
        w.click("home", cx);
    })
    .unwrap();
    ready(cx, &view, 4);
    cx.update_window(window.into(), |_, w, cx| drag(w, cx, 3, "shared"))
        .unwrap();
    settle(cx, |cx| cx.update(|cx| !view.read(cx).note_busy(&uuid(3))));
    cx.update_window(window.into(), |_, w, cx| w.click("shared", cx))
        .unwrap();
    ready(cx, &view, 1);
    cx.update_window(window.into(), |_, w, cx| {
        drag(w, cx, 3, "archive-destination")
    })
    .unwrap();
    ready(cx, &view, 0);
    assert_eq!(
        cx.update(|cx| view.read(cx).destination.clone()),
        Destination::Shared
    );
    cx.update_window(window.into(), |_, w, _| {
        w.remove_window();
    })
    .unwrap();
    services.cancel_operations();
    runtime.block_on(host.shutdown());
}

#[gpui_kit::test]
#[allow(clippy::too_many_lines)] // One owned rendered fixture verifies its complete geometry/input contract.
fn known_color_geometry_and_plus_accessory_alignment(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let root = tempfile::tempdir().unwrap();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let (host, _) = fixture(&runtime, root.path());
    let services = append_tests::services(&host, &runtime);
    cx.update(gpui_kit::init);
    let (window, view) = append_tests::open(cx, services.clone());
    ready(cx, &view, 4);
    for mode in [
        gpui_kit::component::ThemeMode::Light,
        gpui_kit::component::ThemeMode::Dark,
    ] {
        for width in [980., 760.] {
            for detail in [false, true] {
                cx.update_window(window.into(), |_, w, cx| {
                    apply_theme(mode, cx);
                    w.resize(size(px(width), px(560.)));
                    w.bounds_changed(cx);
                    view.update(cx, |v, cx| {
                        if detail {
                            v.select(4, w, cx);
                        } else {
                            v.close_detail(w, cx);
                        }
                    });
                    w.render_frame(cx);
                    let row = w.find(("note", 4_u64)).bounds();
                    let dot = w.within(("note", 4_u64)).find("project-dot").bounds();
                    let text = w.within(("note", 4_u64)).find("note-preview").bounds();
                    assert_eq!(row.size.height, px(32.));
                    assert_eq!(dot.size, size(px(5.), px(5.)));
                    assert!(dot.right() <= row.right() - px(7.));
                    assert!(dot.left() >= text.right());
                    assert!(
                        dot.left() > row.right() - px(20.),
                        "known-colored long row stays trailing without layout repair"
                    );
                    let plus = w.find("project-plus-glyph").bounds();
                    let hit = w.find("add-project").bounds();
                    let home = w.find("home-accessory").bounds();
                    let project = w
                        .find(gpui_kit::SharedString::from(format!(
                            "project-accessory-{}",
                            project(1)
                        )))
                        .bounds();
                    assert_eq!(hit.size, size(px(28.), px(28.)));
                    assert!((plus.center().x - home.center().x).abs() <= px(0.5));
                    assert!((plus.center().x - project.center().x).abs() <= px(0.5));
                    if detail {
                        let pane = w.find("detail-surface").bounds();
                        for id in ["copy-detail", "share-note", "archive", "close-detail"] {
                            let action = w.find(id).bounds();
                            assert!(action.left() >= pane.left());
                            assert!(action.right() <= pane.right());
                        }
                    }
                })
                .unwrap();
            }
        }
    }
    // A watch-driven Jev-like assignment/color change is visible immediately; colorless stays absent.
    runtime.block_on(async {
        host.db
            .writer()
            .await
            .unwrap()
            .execute(
                "UPDATE notes SET project_id=? WHERE short_id=4",
                [project(3)],
            )
            .unwrap();
    });
    settle(cx, |cx| {
        cx.update(|cx| view.read(cx).model.rows[0].project_name.as_deref() == Some("Colorless"))
    });
    cx.update_window(window.into(), |_, w, cx| {
        w.render_frame(cx);
        assert!(w.within(("note", 4_u64)).try_find("project-dot").is_none());
    })
    .unwrap();
    runtime.block_on(async {
        host.db
            .writer()
            .await
            .unwrap()
            .execute(
                "UPDATE notes SET project_id=? WHERE short_id=4",
                [project(2)],
            )
            .unwrap();
    });
    settle(cx, |cx| {
        cx.update(|cx| view.read(cx).model.rows[0].project_color.as_deref() == Some("abcdef"))
    });
    cx.update_window(window.into(), |_, w, cx| {
        w.render_frame(cx);
        assert!(w.within(("note", 4_u64)).find("project-dot").visible());
        w.remove_window();
    })
    .unwrap();
    services.cancel_operations();
    runtime.block_on(host.shutdown());
}

#[gpui_kit::test]
#[allow(clippy::too_many_lines)] // Delayed HTTP covers disposal and gateway watch/ack order on one host.
fn sharing_pending_guards_navigation_close_reopen_and_uncertain_failure(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let root = tempfile::tempdir().unwrap();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let (host, fake) = fixture(&runtime, root.path());
    let services = append_tests::services(&host, &runtime);
    cx.update(|cx| {
        gpui_kit::init(cx);
        install_today_keys(cx);
    });
    let (window, view) = append_tests::open(cx, services.clone());
    ready(cx, &view, 4);
    fake.mode.store(4, Ordering::SeqCst); // canonical write/watch precedes gateway acknowledgement
    cx.update_window(window.into(), |_, w, cx| {
        drag(w, cx, 3, "shared");
    })
    .unwrap();
    settle(cx, |cx| {
        cx.update(|cx| {
            view.read(cx)
                .model
                .rows
                .iter()
                .find(|r| r.id == 3)
                .unwrap()
                .shared
        })
    });
    cx.update_window(window.into(), |_, w, cx| {
        assert!(view.read(cx).note_busy(&uuid(3)));
        drag(w, cx, 3, "shared"); // duplicate source is no longer draggable
        view.update(cx, |v, cx| v.select(3, w, cx));
        let composer = view.read(cx).composer.clone();
        composer.update(cx, |i, cx| i.set_value("retained typing", w, cx));
        view.update(cx, |v, cx| v.submit(w, cx));
        assert_eq!(composer.read(cx).value(), "retained typing");
        w.click("shared", cx);
        assert_eq!(composer.read(cx).value(), "retained typing");
        w.remove_window();
    })
    .unwrap();
    assert_eq!(fake.calls.lock().unwrap().len(), 2);
    fake.gate.notify_one();
    settle(cx, |_| {
        !services.capture.lock().unwrap().note_actions.busy(&uuid(3))
    });
    assert_eq!(
        services.capture.lock().unwrap().note_actions.links.len(),
        1,
        "completed URL retained while closed"
    );
    drop(view);
    fake.mode.store(0, Ordering::SeqCst);
    let (window, view) = append_tests::open(cx, services.clone());
    ready(cx, &view, 1);
    cx.update(|cx| {
        assert_eq!(
            cx.read_from_clipboard().unwrap().text().unwrap(),
            format!("https://owned.invalid/share/{}", uuid(3))
        )
    });
    let composer = cx.update(|cx| view.read(cx).composer.clone());
    cx.update_window(window.into(), |_, w, cx| {
        composer.update(cx, |i, cx| i.set_value("", w, cx));
        w.click("home", cx);
    })
    .unwrap();
    ready(cx, &view, 4);
    fake.mode.store(3, Ordering::SeqCst); // acknowledgement before canonical share watch
    cx.update_window(window.into(), |_, w, cx| {
        drag(w, cx, 2, "shared");
        w.remove_window();
    })
    .unwrap();
    settle(cx, |_| {
        !services
            .capture
            .lock()
            .unwrap()
            .note_actions
            .links
            .is_empty()
    });
    drop(view);
    let (window, view) = append_tests::open(cx, services.clone());
    ready(cx, &view, 4);
    cx.update(|cx| {
        assert_eq!(
            cx.read_from_clipboard().unwrap().text().unwrap(),
            format!("https://owned.invalid/share/{}", uuid(2))
        );
        assert!(view.read(cx).note_busy(&uuid(2)));
    });
    assert!(cx.update(|cx| {
        view.read(cx)
            .model
            .rows
            .iter()
            .find(|r| r.id == 2)
            .unwrap()
            .shared
    }));
    runtime.block_on(async {host.db.writer().await.unwrap().execute("INSERT INTO note_shares(id,user_id,token,created_at) VALUES(?,'append-owner','ack-first','2026-01-01T00:00:00Z')",[uuid(2)]).unwrap();});
    settle(cx, |cx| {
        cx.update(|cx| {
            !view.read(cx).note_busy(&uuid(2))
                && view
                    .read(cx)
                    .model
                    .rows
                    .iter()
                    .find(|r| r.id == 2)
                    .unwrap()
                    .shared
        })
    });
    fake.mode.store(2, Ordering::SeqCst);
    cx.update_window(window.into(), |_, w, cx| {
        cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string("sentinel".into()));
        drag(w, cx, 1, "shared");
    })
    .unwrap();
    settle(cx, |cx| cx.update(|cx| view.read(cx).error.is_some()));
    cx.update(|cx| {
        assert!(
            view.read(cx)
                .error
                .as_deref()
                .unwrap()
                .contains("may have completed")
        );
        assert_eq!(
            cx.read_from_clipboard().unwrap().text().unwrap(),
            "sentinel"
        );
    });
    let requests = fake.calls.lock().unwrap().len();
    cx.run_until_parked();
    assert_eq!(
        fake.calls.lock().unwrap().len(),
        requests,
        "no blind publish retry"
    );
    cx.update_window(window.into(), |_, w, _| w.remove_window())
        .unwrap();
    services.cancel_operations();
    runtime.block_on(host.shutdown());
}

#[gpui_kit::test]
#[allow(clippy::too_many_lines)] // One owned rendered fixture verifies its complete geometry/input contract.
fn archived_composer_creates_new_and_drop_guards_keep_draft_ime_append(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let root = tempfile::tempdir().unwrap();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let (host, fake) = fixture(&runtime, root.path());
    let services = append_tests::services(&host, &runtime);
    runtime
        .block_on(host.app.handle(AppRequest::NoteArchive { id: "3".into() }))
        .unwrap();
    cx.update(|cx| {
        gpui_kit::init(cx);
        install_today_keys(cx);
    });
    let (window, view) = append_tests::open(cx, services.clone());
    ready(cx, &view, 3);
    let composer = cx.update(|cx| view.read(cx).composer.clone());
    cx.update_window(window.into(), |_, w, cx| {
        view.update(cx, |v, cx| {
            v.change_destination(Destination::Archive, w, cx)
        });
    })
    .unwrap();
    ready(cx, &view, 1);
    cx.update_window(window.into(), |_, w, cx| {
        w.render_frame(cx);
        w.click(("note", 3_u64), cx);
        composer.update(cx, |i, cx| i.set_value("New from Archive", w, cx));
        w.press("enter", cx);
        assert!(view.read(cx).model.capture().appends.pending.is_empty());
        assert_eq!(view.read(cx).model.rows.len(), 1);
        assert!(w.try_find(("pending", 1_u64)).is_none());
    })
    .unwrap();
    settle(cx, |_| {
        services
            .capture
            .lock()
            .unwrap()
            .pending
            .first()
            .is_some_and(|p| p.id == Some(5))
    });
    let created = runtime
        .block_on(host.app.handle(AppRequest::NoteGet {
            id: "5".into(),
            archived: false,
        }))
        .unwrap();
    assert!(
        matches!(created,AppResponse::NoteDetail(ref n) if n.content=="New from Archive" && n.note.project.is_none())
    );
    let archived = runtime
        .block_on(host.app.handle(AppRequest::NoteGet {
            id: "3".into(),
            archived: true,
        }))
        .unwrap();
    assert!(
        matches!(archived,AppResponse::NoteDetail(ref n) if n.content=="Owned content ".repeat(100))
    );
    cx.update_window(window.into(), |_, w, cx| {
        w.click("home", cx);
    })
    .unwrap();
    ready(cx, &view, 4);
    cx.update_window(window.into(), |_, w, cx| {
        composer.update(cx, |i, cx| {
            i.set_value("draft", w, cx);
            i.set_selected_range(2..2, cx);
        });
        drag(w, cx, 2, "shared");
        assert!(!view.read(cx).note_busy(&uuid(2)));
        assert_eq!(composer.read(cx).value(), "draft");
        assert_eq!(composer.read(cx).selected_range(), 2..2);
        composer.update(cx, |i, cx| {
            i.set_value("", w, cx);
            i.replace_and_mark_text_in_range(None, "ni", Some(2..2), w, cx);
        });
        drag(w, cx, 2, "archive-destination");
        assert_eq!(view.read(cx).model.rows.len(), 4);
        composer.update(cx, |i, cx| {
            i.unmark_text(w, cx);
            i.set_value("", w, cx);
        });
        let row = view
            .read(cx)
            .model
            .rows
            .iter()
            .find(|r| r.id == 2)
            .unwrap()
            .clone();
        view.read(cx)
            .model
            .capture()
            .appends
            .accept(&row, "outstanding".into())
            .unwrap();
        drag(w, cx, 2, "shared");
        assert!(fake.calls.lock().unwrap().is_empty());
        view.update(cx, |v, cx| {
            let foreign = NoteDrag {
                row: row.clone(),
                owner: "foreign".into(),
            };
            assert!(!v.accepts_drop(&foreign, &Destination::Shared, w, cx));
        });
        w.remove_window();
    })
    .unwrap();
    services.cancel_operations();
    runtime.block_on(host.shutdown());
}
