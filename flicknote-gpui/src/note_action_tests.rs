//! Owned production LocalHost, canonical ShareGateway HTTP, pointer hit testing; no native launch.
use super::tests::settle;
use super::*;
use gpui_kit::{
    InputEvent as _, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, TestAppContext,
    point, test::TestWindowExt,
};
use std::sync::atomic::{AtomicU8, Ordering};

#[derive(Clone, Default)]
pub(super) struct Gateway {
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
pub(super) fn fixture(
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
    services.source.choose(true);
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
    assert!(cx.update(|cx| view.read(cx).copy_toast.is_some()));
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
    assert!(cx.update(|cx| view.read(cx).copy_toast.is_some()));
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
    // A watch-driven backend assignment/color change is visible immediately; colorless stays absent.
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
        view.update(cx, |this, _| this.copy_toast = None);
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
        assert!(view.read(cx).copy_toast.is_none());
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

#[gpui_kit::test]
#[allow(clippy::too_many_lines)] // Accepted capture/append/share identity across real calendar watch changes.
fn historical_capture_append_and_share_keep_original_membership(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let root = tempfile::tempdir().unwrap();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let (host, fake) = fixture(&runtime, root.path());
    let historical = Period::Day(None)
        .shifted(false, &chrono::Local::now())
        .unwrap();
    let range = historical.range(&chrono::Local::now()).unwrap().unwrap();
    runtime.block_on(async {
        host.db
            .writer()
            .await
            .unwrap()
            .execute(
                "UPDATE notes SET created_at=? WHERE short_id IN (2,3)",
                [range.0.to_rfc3339()],
            )
            .unwrap();
    });
    let services = append_tests::services(&host, &runtime);
    cx.update(|cx| {
        gpui_kit::init(cx);
        install_today_keys(cx);
    });
    let (window, view) = append_tests::open(cx, services.clone());
    ready(cx, &view, 2);
    cx.update_window(window.into(), |_, w, cx| {
        w.render_frame(cx);
        w.press("alt-h", cx);
    })
    .unwrap();
    ready(cx, &view, 2);
    let writer = runtime.block_on(host.db.writer()).unwrap();
    cx.update_window(window.into(), |_, w, cx| {
        w.render_frame(cx);
        let composer = view.read(cx).composer.clone();
        composer.update(cx, |i, cx| {
            i.focus(w, cx);
            i.set_value("Current unassigned capture from history", w, cx)
        });
        w.render_frame(cx);
        w.press("enter", cx);
        w.render_frame(cx);
    })
    .unwrap();
    settle(cx, |cx| {
        cx.update(|cx| view.read(cx).model.capture().pending.len() == 1)
    });
    cx.update_window(window.into(), |_, w, cx| {
        w.render_frame(cx);
        assert_eq!(view.read(cx).model.capture().pending.len(), 1);
        let capture = view.read(cx).model.capture();
        assert!(!view.read(cx).pending_visible(&capture.pending[0]));
        drop(capture);
        assert_eq!(view.read(cx).model.rows.len(), 2);
        w.click("period-current", cx);
    })
    .unwrap();
    drop(writer);
    ready(cx, &view, 3);
    let created = runtime
        .block_on(host.app.handle(AppRequest::NoteGet {
            id: "5".into(),
            archived: false,
        }))
        .unwrap();
    assert!(
        matches!(created,AppResponse::NoteDetail(ref n) if n.note.project.is_none() && n.content=="Current unassigned capture from history")
    );
    runtime.block_on(async {
        let created_at: String = host
            .db
            .reader()
            .await
            .unwrap()
            .query_row("SELECT created_at FROM notes WHERE short_id=5", [], |r| {
                r.get(0)
            })
            .unwrap();
        let created_at = chrono::DateTime::parse_from_rfc3339(&created_at)
            .unwrap()
            .with_timezone(&chrono::Utc);
        let today = Period::Day(None)
            .range(&chrono::Local::now())
            .unwrap()
            .unwrap();
        assert!(created_at >= today.0 && created_at < today.1);
    });
    cx.update_window(window.into(), |_, w, cx| {
        w.render_frame(cx);
        w.press("alt-left", cx);
    })
    .unwrap();
    ready(cx, &view, 2);
    let writer = runtime.block_on(host.db.writer()).unwrap();
    cx.update_window(window.into(), |_, w, cx| {
        w.render_frame(cx);
        w.click(("note", 2_u64), cx);
        let composer = view.read(cx).composer.clone();
        composer.update(cx, |i, cx| {
            i.focus(w, cx);
            i.set_value("Historical immutable append", w, cx)
        });
        w.render_frame(cx);
        w.press("enter", cx);
    })
    .unwrap();
    settle(cx, |cx| {
        cx.update(|cx| !view.read(cx).model.capture().appends.pending.is_empty())
    });
    cx.update_window(window.into(), |_, w, cx| {
        w.render_frame(cx);
        assert_eq!(view.read(cx).period, historical);
        assert!(
            view.read(cx)
                .detail
                .as_ref()
                .unwrap()
                .source
                .ends_with("Historical immutable append")
        );
        w.render_frame(cx);
        w.click("period-back", cx);
    })
    .unwrap();
    drop(writer);
    ready(cx, &view, 0);
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let note = runtime
            .block_on(host.app.handle(AppRequest::NoteGet {
                id: uuid(2),
                archived: false,
            }))
            .unwrap();
        if matches!(note,AppResponse::NoteDetail(ref n) if n.content.ends_with("Historical immutable append"))
        {
            break;
        }
        assert!(Instant::now() < deadline);
        cx.run_until_parked();
    }
    cx.update_window(window.into(), |_, w, cx| {
        w.render_frame(cx);
        w.press("alt-l", cx);
    })
    .unwrap();
    ready(cx, &view, 2);
    fake.mode.store(3, Ordering::SeqCst); // Gateway acknowledges without a canonical share emission.
    cx.update_window(window.into(), |_, w, cx| {
        w.render_frame(cx);
        w.click(("note", 3_u64), cx);
        w.render_frame(cx);
        w.click("share-note", cx);
    })
    .unwrap();
    settle(cx, |cx| {
        cx.update(|cx| {
            view.read(cx).model.capture().note_actions.links.len() == 1
                || view
                    .read(cx)
                    .model
                    .rows
                    .iter()
                    .any(|r| r.id == 3 && r.shared)
        })
    });
    cx.update_window(window.into(), |_, w, cx| {
        w.render_frame(cx);
        w.click("period-back", cx);
    })
    .unwrap();
    ready(cx, &view, 0);
    assert!(
        services.capture.lock().unwrap().note_actions.busy(&uuid(3)),
        "absence in a different date cannot confirm Share"
    );
    cx.update_window(window.into(), |_, w, cx| {
        w.render_frame(cx);
        w.click("period-forward", cx);
    })
    .unwrap();
    ready(cx, &view, 2);
    cx.update(|cx| {
        assert!(
            view.read(cx)
                .model
                .rows
                .iter()
                .any(|r| r.id == 3 && r.shared),
            "confirmed Share projects in its accepted period"
        )
    });
    runtime.block_on(async { host.db.writer().await.unwrap().execute("INSERT INTO note_shares(id,user_id,token,created_at) VALUES(?,'append-owner','canonical','2026-01-01T00:00:00Z')",[uuid(3)]).unwrap(); });
    settle(cx, |_| {
        !services.capture.lock().unwrap().note_actions.busy(&uuid(3))
    });
    cx.update_window(window.into(), |_, w, _| w.remove_window())
        .unwrap();
    services.cancel_operations();
    runtime.block_on(host.shutdown());
}

#[gpui_kit::test]
#[allow(clippy::too_many_lines)] // One owned gateway/window journey verifies canonical search actions and membership.
fn outside_origin_search_actions_keep_canonical_identity_and_query_membership(
    cx: &mut TestAppContext,
) {
    cx.executor().allow_parking();
    let root = tempfile::tempdir().unwrap();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let (host, fake) = fixture(&runtime, root.path());
    runtime.block_on(async {
        let writer=host.db.writer().await.unwrap();
        writer.execute("UPDATE notes SET short_id=CAST(short_id AS INTEGER)",[]).unwrap();
        writer.execute("UPDATE notes SET content='searchaction canonical body',created_at='2020-01-01T12:00:00Z' WHERE short_id=4",[]).unwrap();
    });
    let services = append_tests::services(&host, &runtime);
    cx.update(|cx| {
        gpui_kit::init(cx);
        install_today_keys(cx);
    });
    let (window, view) = append_tests::open(cx, services.clone());
    settle(cx, |cx| cx.update(|cx| view.read(cx).loaded));
    cx.update_window(window.into(), |_, w, cx| {
        super::search_tests::query(&view, "searchaction", w, cx);
    })
    .unwrap();
    settle(cx, |cx| {
        cx.update(|cx| !view.read(cx).search.loading && view.read(cx).model.rows.len() == 1)
    });
    cx.update_window(window.into(), |_, w, cx| {
        w.render_frame(cx);
        w.click(("note", 4_u64), cx);
    })
    .unwrap();
    settle(cx, |cx| cx.update(|cx| view.read(cx).detail.is_some()));
    cx.update_window(window.into(), |_, w, cx| {
        w.render_frame(cx);
        w.click("share-note", cx);
    })
    .unwrap();
    settle(cx, |cx| cx.update(|cx| view.read(cx).model.rows[0].shared));
    cx.update(|cx| {
        assert_eq!(
            cx.read_from_clipboard().unwrap().text().unwrap(),
            format!("https://owned.invalid/share/{}", uuid(4))
        )
    });
    cx.update_window(window.into(), |_, w, cx| {
        w.render_frame(cx);
        w.click("unshare-note", cx);
    })
    .unwrap();
    settle(cx, |cx| {
        cx.update(|cx| !view.read(cx).model.rows[0].shared && !view.read(cx).note_busy(&uuid(4)))
    });
    cx.update_window(window.into(), |_, w, cx| {
        drag(w, cx, 4, &format!("project-{}", project(2)));
    })
    .unwrap();
    settle(cx, |cx| {
        cx.update(|cx| {
            view.read(cx)
                .model
                .rows
                .first()
                .is_some_and(|r| r.project_id == Some(project(2)))
                && !view.read(cx).note_busy(&uuid(4))
        })
    });
    cx.update_window(window.into(), |_, w, cx| {
        drag(w, cx, 4, "archive-destination");
    })
    .unwrap();
    settle(cx, |cx| {
        cx.update(|cx| view.read(cx).model.rows.is_empty() && view.read(cx).detail.is_none())
    });
    cx.update_window(window.into(), |_, w, cx| {
        super::search_tests::query(&view, "#4", w, cx);
    })
    .unwrap();
    settle(cx, |cx| {
        cx.update(|cx| {
            !view.read(cx).search.loading
                && view.read(cx).model.rows.first().is_some_and(|r| r.archived)
        })
    });
    cx.update_window(window.into(), |_, w, cx| {
        w.render_frame(cx);
        w.click(("note", 4_u64), cx);
    })
    .unwrap();
    settle(cx, |cx| cx.update(|cx| view.read(cx).detail.is_some()));
    cx.update_window(window.into(), |_, w, cx| {
        w.render_frame(cx);
        assert!(w.try_find("share-note").is_none());
        w.click("archive", cx);
    })
    .unwrap();
    settle(cx, |cx| {
        cx.update(|cx| {
            view.read(cx)
                .model
                .rows
                .first()
                .is_some_and(|r| !r.archived)
                && !view.read(cx).note_busy(&uuid(4))
        })
    });
    // A delayed accepted share keeps its immutable outside-origin target after rail exit.
    fake.mode.store(1, Ordering::SeqCst);
    cx.update_window(window.into(), |_, w, cx| {
        w.render_frame(cx);
        w.click(("note", 4_u64), cx);
    })
    .unwrap();
    settle(cx, |cx| cx.update(|cx| view.read(cx).detail.is_some()));
    cx.update_window(window.into(), |_, w, cx| {
        w.render_frame(cx);
        w.click("share-note", cx);
        w.click("home", cx);
    })
    .unwrap();
    settle(cx, |cx| {
        cx.update(|cx| view.read(cx).loaded && !view.read(cx).search.active())
    });
    cx.update(|cx| {
        assert!(
            view.read(cx).note_busy(&uuid(4)),
            "origin absence cannot acknowledge search action"
        )
    });
    fake.gate.notify_one();
    settle(cx, |cx| {
        cx.update(|cx| {
            cx.read_from_clipboard().unwrap().text().unwrap()
                == format!("https://owned.invalid/share/{}", uuid(4))
        })
    });
    assert!(
        fake.calls
            .lock()
            .unwrap()
            .iter()
            .all(|(_, id)| id == &uuid(4))
    );
    cx.update_window(window.into(), |_, w, _| w.remove_window())
        .unwrap();
    services.cancel_operations();
    runtime.block_on(host.shutdown());
}

#[gpui_kit::test]
#[allow(clippy::too_many_lines)] // One owned end-to-end destination fixture.
fn failed_destination_render_watch_capture_and_reopen(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let root = tempfile::tempdir().unwrap();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let (host, _fake) = fixture(&runtime, root.path());
    runtime.block_on(async {
        host.db.writer().await.unwrap().execute("UPDATE notes SET status=CASE short_id WHEN 3 THEN 'ai_failed' ELSE 'source_failed' END WHERE short_id IN (3,4)", []).unwrap();
    });
    let services = append_tests::services(&host, &runtime);
    cx.update(|cx| {
        gpui_kit::init(cx);
        install_today_keys(cx);
    });
    let (window, view) = append_tests::open(cx, services.clone());
    ready(cx, &view, 4);
    let stale = cx.update(|cx| {
        let v = view.read(cx);
        (
            v.destination.clone(),
            v.source.human_only,
            v.watch_epoch,
            v.period.clone(),
        )
    });
    cx.update_window(window.into(), |_, w, cx| {
        w.render_frame(cx);
        let composer = view.read(cx).composer.clone();
        composer.update(cx, |input, cx| input.set_value("retained", w, cx));
        w.render_frame(cx);
        w.click("failed", cx);
        assert_eq!(composer.read(cx).value(), "retained");
        view.update(cx, |v, cx| {
            v.receive_snapshot(&stale, Err("stale error".into()), w, cx)
        });
        assert!(view.read(cx).error.is_none());
    })
    .unwrap();
    ready(cx, &view, 2);
    cx.update_window(window.into(), |_, w, cx| {
        for (theme, size) in [
            (gpui_kit::component::ThemeMode::Light, (980., 720.)),
            (gpui_kit::component::ThemeMode::Dark, (760., 560.)),
        ] {
            apply_theme(theme, cx);
            w.resize(gpui_kit::size(px(size.0), px(size.1)));
            w.bounds_changed(cx);
            w.render_frame(cx);
            assert!(w.find("failed").bounds().size.width > px(0.));
            assert!(w.find(("note", 4_u64)).bounds().size.width > px(0.));
        }
        view.update(cx, |v, cx| {
            let drag = NoteDrag {
                row: v.model.rows[0].clone(),
                owner: v.services.user_id.clone(),
            };
            assert!(!v.accepts_drop(&drag, &Destination::Failed, w, cx));
            assert!(
                !v.accepts_drop(&drag, &Destination::Project(project(2)), w, cx),
                "draft composer retains guard"
            );
            v.composer
                .update(cx, |input, cx| input.set_value("", w, cx));
        });
        w.render_frame(cx);
        view.update(cx, |v, cx| {
            let drag = NoteDrag {
                row: v.model.rows[0].clone(),
                owner: v.services.user_id.clone(),
            };
            assert!(v.accepts_drop(&drag, &Destination::Project(project(2)), w, cx));
        });
        w.click(("note", 4_u64), cx);
    })
    .unwrap();
    settle(cx, |cx| cx.update(|cx| view.read(cx).detail.is_some()));
    cx.update_window(window.into(), |_, w, cx| {
        view.update(cx, |v, cx| {
            v.close_detail(w, cx);
            v.composer
                .update(cx, |input, cx| input.set_value("Owned capture", w, cx));
            v.submit(w, cx);
            assert_eq!(
                v.model.rows.len()
                    + v.model
                        .capture()
                        .pending
                        .iter()
                        .filter(|p| v.pending_visible(p))
                        .count(),
                2,
                "queued capture has no Failed pending row"
            );
        });
    })
    .unwrap();
    settle(cx, |cx| {
        cx.update(|cx| {
            view.read(cx)
                .model
                .capture()
                .pending
                .iter()
                .any(|p| p.id == Some(5))
        })
    });
    runtime.block_on(async {
        let writer = host.db.writer().await.unwrap();
        writer
            .execute("UPDATE notes SET status='ready' WHERE short_id=4", [])
            .unwrap();
        writer
            .execute(
                "UPDATE notes SET status='source_failed' WHERE short_id=2",
                [],
            )
            .unwrap();
    });
    settle(cx, |cx| {
        cx.update(|cx| {
            view.read(cx)
                .model
                .rows
                .iter()
                .map(|r| r.id)
                .collect::<Vec<_>>()
                == vec![3, 2]
        })
    });
    cx.update_window(window.into(), |_, w, _| w.remove_window())
        .unwrap();
    drop(view);
    let (window, view) = append_tests::open(cx, services);
    ready(cx, &view, 2);
    cx.update(|cx| assert_eq!(view.read(cx).destination, Destination::Failed));
    runtime.block_on(async {
        host.db
            .writer()
            .await
            .unwrap()
            .execute("UPDATE notes SET status='ready'", [])
            .unwrap();
    });
    ready(cx, &view, 0);
    cx.update_window(window.into(), |_, w, cx| {
        w.render_frame(cx);
        assert!(w.find("failed").bounds().size.height > px(0.));
        w.remove_window();
    })
    .unwrap();
    runtime.block_on(host.shutdown());
}

fn assert_retry_input_guards(view: &Entity<Today>, w: &mut Window, cx: &mut App) {
    let row = view
        .read(cx)
        .model
        .rows
        .iter()
        .find(|r| r.id == 3)
        .unwrap()
        .clone();
    let composer = view.read(cx).composer.clone();
    composer.update(cx, |input, cx| {
        input.replace_and_mark_text_in_range(None, "ni", Some(2..2), w, cx);
    });
    let marked = composer.read(cx).value().to_string();
    w.render_frame(cx);
    w.click(("retry-note", 3_u64), cx);
    assert!(!view.read(cx).note_busy(&row.uuid));
    assert_eq!(composer.read(cx).value(), marked);
    composer.update(cx, |input, cx| {
        input.unmark_text(w, cx);
        input.set_value("retained draft", w, cx);
        input.set_selected_range(3..3, cx);
    });
    view.update(cx, |v, cx| {
        v.edit(project_editor::Kind::Add, w, cx);
        v.retry_row(row.clone(), w, cx);
        assert!(!v.note_busy(&row.uuid));
        v.cancel_editor(w, cx);
    });
    assert_eq!(composer.read(cx).value(), "retained draft");
    assert_eq!(composer.read(cx).selected_range(), 3..3);
    w.render_frame(cx);
}

#[gpui_kit::test]
#[allow(clippy::too_many_lines)] // One owned rendered journey tests immutable row target and retained host operation.
fn failed_retry_row_target_busy_navigation_and_geometry(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let root = tempfile::tempdir().unwrap();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let (host, _fake) = fixture(&runtime, root.path());
    runtime.block_on(async {
        host.db.writer().await.unwrap().execute("UPDATE notes SET status=CASE short_id WHEN 3 THEN 'ai_failed' ELSE 'source_failed' END WHERE short_id IN (3,4)", []).unwrap();
    });
    let services = append_tests::services(&host, &runtime);
    services.source.choose(true);
    cx.update(|cx| {
        gpui_kit::init(cx);
        install_today_keys(cx);
    });
    let (window, view) = append_tests::open(cx, services.clone());
    ready(cx, &view, 4);
    cx.update_window(window.into(), |_, w, cx| {
        w.render_frame(cx);
        w.click("failed", cx);
    })
    .unwrap();
    ready(cx, &view, 2);
    cx.update_window(window.into(), |_, w, cx| {
        for mode in [
            gpui_kit::component::ThemeMode::Light,
            gpui_kit::component::ThemeMode::Dark,
        ] {
            apply_theme(mode, cx);
            for width in [760., 980.] {
                w.resize(gpui_kit::size(px(width), px(560.)));
                w.bounds_changed(cx);
                w.render_frame(cx);
                let row = w.find(("note", 3_u64)).bounds();
                let button = w.find(("retry-note", 3_u64)).bounds();
                assert_eq!(row.size.height, px(32.));
                assert!(button.right() <= row.right());
                assert!(button.size.width > px(30.));
                assert!(w.find(("row-preview", 3_u64)).bounds().right() <= button.left());
            }
        }
        w.click(("note", 4_u64), cx);
        assert_eq!(view.read(cx).model.selected, Some(4));
        let composer = view.read(cx).composer.clone();
        composer.update(cx, |input, cx| input.set_value("retained draft", w, cx));
        assert_retry_input_guards(&view, w, cx);
        w.render_frame(cx);
    })
    .unwrap();
    // Hold the actual PowerSync writer: accepted retry cannot finish before its duplicate guard is checked.
    let writer = runtime.block_on(host.db.writer()).unwrap();
    cx.update_window(window.into(), |_, w, cx| {
        w.click(("retry-note", 3_u64), cx);
        assert_eq!(
            view.read(cx).model.selected,
            Some(4),
            "retry cannot bubble to another selection"
        );
        assert_eq!(view.read(cx).detail.as_ref().unwrap().uuid, uuid(4));
        assert_eq!(view.read(cx).composer.read(cx).value(), "retained draft");
        assert!(view.read(cx).note_busy(&uuid(3)));
        w.render_frame(cx);
        w.click(("retry-note", 3_u64), cx);
        assert_eq!(
            view.read(cx).model.selected,
            Some(4),
            "disabled retry cannot select its parent row"
        );
        view.update(cx, |v, cx| {
            let target = v.model.rows.iter().find(|r| r.id == 3).unwrap().clone();
            v.perform_note_action(target, NoteAction::Archive, w, cx);
            v.change_destination(Destination::Shared, w, cx);
        });
        assert!(view.read(cx).note_busy(&uuid(3)));
    })
    .unwrap();
    window.update(cx, |_, w, _| w.remove_window()).unwrap();
    drop(writer);
    settle(cx, |_| {
        runtime
            .block_on(host.db.reader())
            .unwrap()
            .query_row("SELECT status FROM notes WHERE short_id=3", [], |r| {
                r.get::<_, String>(0)
            })
            .unwrap()
            == "ai_queued"
    });
    let (reopened, view) = append_tests::open(cx, services.clone());
    ready(cx, &view, 0);
    cx.update_window(reopened.into(), |_, w, cx| {
        view.update(cx, |v, cx| v.change_destination(Destination::Failed, w, cx));
    })
    .unwrap();
    ready(cx, &view, 1);
    cx.update_window(reopened.into(), |_, w, cx| {
        assert_eq!(view.read(cx).model.rows[0].id, 4);
        assert_eq!(view.read(cx).composer.read(cx).value(), "retained draft");
        w.render_frame(cx);
        for _ in 0..40 {
            if w.find(("retry-note", 4_u64)).focused() == Some(true) {
                break;
            }
            w.focus_next(cx);
            w.render_frame(cx);
        }
        assert_eq!(w.find(("retry-note", 4_u64)).focused(), Some(true));
        w.press("space", cx);
    })
    .unwrap();
    ready(cx, &view, 0);
    assert_eq!(
        runtime
            .block_on(host.db.reader())
            .unwrap()
            .query_row("SELECT status FROM notes WHERE short_id=4", [], |r| r
                .get::<_, String>(0))
            .unwrap(),
        "source_queued"
    );
    reopened.update(cx, |_, w, _| w.remove_window()).unwrap();
    runtime.block_on(host.shutdown());
}

#[gpui_kit::test]
fn failed_retry_database_error_requires_check_across_destinations(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let root = tempfile::tempdir().unwrap();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let (host, _fake) = fixture(&runtime, root.path());
    runtime.block_on(async {
        host.db
            .writer()
            .await
            .unwrap()
            .execute("UPDATE notes SET status='ai_failed' WHERE short_id=3", [])
            .unwrap();
    });
    let services = append_tests::services(&host, &runtime);
    services.source.choose(true);
    cx.update(|cx| {
        gpui_kit::init(cx);
        install_today_keys(cx);
    });
    let (window, view) = append_tests::open(cx, services.clone());
    cx.update_window(window.into(), |_, w, cx| {
        w.render_frame(cx);
        w.click("failed", cx);
    })
    .unwrap();
    ready(cx, &view, 1);
    let writer = runtime.block_on(host.db.writer()).unwrap();
    writer
        .execute(
            "UPDATE notes SET metadata='invalid-json' WHERE short_id=3",
            [],
        )
        .unwrap();
    cx.update_window(window.into(), |_, w, cx| {
        w.render_frame(cx);
        w.click(("retry-note", 3_u64), cx);
    })
    .unwrap();
    drop(writer);
    settle(cx, |_| {
        !services
            .capture
            .lock()
            .unwrap()
            .note_actions
            .unchecked
            .is_empty()
    });
    cx.update_window(window.into(), |_, w, cx| {
        view.update(cx, |v, cx| v.change_destination(Destination::Shared, w, cx));
        w.render_frame(cx);
        assert!(
            w.try_find(("check-retry", 3_u64)).is_some(),
            "unknown feedback lives outside vanished row"
        );
        assert!(view.read(cx).note_busy(&uuid(3)));
    })
    .unwrap();
    runtime.block_on(async {
        host.db
            .writer()
            .await
            .unwrap()
            .execute(
                "UPDATE notes SET metadata='{}',status='source_failed' WHERE short_id=3",
                [],
            )
            .unwrap();
    });
    cx.update_window(window.into(), |_, w, cx| {
        view.update(cx, |v, cx| v.change_destination(Destination::Failed, w, cx));
    })
    .unwrap();
    ready(cx, &view, 1);
    cx.update_window(window.into(), |_, w, cx| {
        assert!(
            view.read(cx).note_busy(&uuid(3)),
            "a new processor failure cannot silently unlock retry"
        );
        w.render_frame(cx);
        w.click(("check-retry", 3_u64), cx);
    })
    .unwrap();
    settle(cx, |cx| cx.update(|cx| !view.read(cx).note_busy(&uuid(3))));
    cx.update_window(window.into(), |_, w, cx| {
        w.render_frame(cx);
        w.click(("retry-note", 3_u64), cx);
    })
    .unwrap();
    ready(cx, &view, 0);
    assert_eq!(
        runtime
            .block_on(host.db.reader())
            .unwrap()
            .query_row("SELECT status FROM notes WHERE short_id=3", [], |r| r
                .get::<_, String>(0))
            .unwrap(),
        "source_queued"
    );
    window.update(cx, |_, w, _| w.remove_window()).unwrap();
    runtime.block_on(host.shutdown());
}
