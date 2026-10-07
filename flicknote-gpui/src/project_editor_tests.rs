use super::*;
use gpui_kit::{TestAppContext, point, test::TestWindowExt};
use project_editor::Kind;
#[gpui_kit::test]
#[allow(clippy::too_many_lines)] // One window checks retained composer/native input across editor transitions.
fn real_project_editor_watch_clear_cancel_validation_and_input_isolation(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let root = tempfile::tempdir().unwrap();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let host = runtime
        .block_on(flicknote_sync::spike::SpikeHost::start(
            root.path(),
            0,
            0,
            Duration::ZERO,
        ))
        .unwrap();
    let services = Arc::new(Services {
        app: host.app.clone(),
        db: host.db.clone(),
        runtime: runtime.handle().clone(),
        operations: Mutex::default(),
        user_id: flicknote_sync::spike::USER.into(),
        real_account: false,
        first_sync: Mutex::default(),
        destination: Mutex::default(),
        temporal: Mutex::default(),
        capture: Arc::default(),
        draft: Mutex::default(),
        organization: Mutex::default(),
        source: crate::source::Control::default(),
        capture_changed: tokio::sync::watch::channel(()).0,
    });
    cx.update(|cx| {
        gpui_kit::init(cx);
        install_today_keys(cx);
    });
    let (window, view) = cx.update(|cx| {
        let (w, v) = gpui_kit::open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds {
                    origin: point(px(0.), px(0.)),
                    size: size(px(980.), px(720.)),
                })),
                ..Default::default()
            },
            cx,
            |w, cx| cx.new(|cx| Today::new(services.clone(), w, cx)),
        )
        .unwrap();
        (w.downcast::<gpui_kit::base::Root>().unwrap(), v)
    });
    tests::settle(cx, |cx| cx.update(|cx| view.read(cx).loaded));
    let composer = cx.update(|cx| view.read(cx).composer.clone());
    cx.update_window(window.into(), |_, w, cx| {
        composer.update(cx, |i, cx| {
            i.set_value("capture draft", w, cx);
            i.set_selected_range(3..3, cx);
        });
        for mode in [
            gpui_kit::component::ThemeMode::Light,
            gpui_kit::component::ThemeMode::Dark,
        ] {
            apply_theme(mode, cx);
            w.resize(size(px(760.), px(560.)));
            w.bounds_changed(cx);
            w.render_frame(cx);
            let heading = w.find("projects-heading").bounds();
            let plus = w.find("add-project").bounds();
            assert!(plus.size.width >= px(24.) && plus.size.height >= px(24.));
            assert!(plus.right() <= heading.right());
            assert_eq!(plus.center().y, heading.center().y);
        }
        w.click("add-project", cx);
    })
    .unwrap();
    let input = cx.update(|cx| view.read(cx).editor.as_ref().unwrap().input.clone());
    cx.update_window(window.into(), |_, w, cx| {
        window_save(&view, w, cx);
    })
    .unwrap();
    cx.update(|cx| {
        assert_eq!(
            view.read(cx).editor.as_ref().unwrap().error.as_deref(),
            Some("Enter a project name")
        )
    });
    cx.update_window(window.into(), |_, w, cx| {
        input.update(cx, |i, cx| i.set_value("  New project  ", w, cx));
        w.render_frame(cx);
        w.press("cmd-1", cx);
        w.press("alt-j", cx);
        w.press("enter", cx);
    })
    .unwrap();
    tests::settle(cx, |cx| {
        cx.update(|cx| {
            view.read(cx).editor.is_none()
                && matches!(view.read(cx).destination, Destination::Project(_))
        })
    });
    let pid = cx.update(|cx| {
        assert_eq!(composer.read(cx).value(), "capture draft");
        assert_eq!(composer.read(cx).selected_range(), 3..3);
        let Destination::Project(id) = &view.read(cx).destination else {
            panic!()
        };
        assert!(
            view.read(cx)
                .projects
                .iter()
                .any(|p| p.id == *id && p.name == "New project")
        );
        id.clone()
    });
    cx.update_window(window.into(), |_, w, cx| {
        for _ in 0..24 {
            w.press("tab", cx);
            w.render_frame(cx);
            if w.find("add-project").focused() == Some(true) {
                break;
            }
        }
        assert_eq!(w.find("add-project").focused(), Some(true));
        w.press("enter", cx);
    })
    .unwrap();
    cx.update_window(window.into(), |_, w, cx| {
        view.update(cx, |v, cx| {
            v.editor
                .as_ref()
                .unwrap()
                .input
                .update(cx, |i, cx| i.set_value("New project", w, cx));
            v.save_editor(w, cx);
        });
    })
    .unwrap();
    tests::settle(cx, |cx| {
        cx.update(|cx| {
            view.read(cx)
                .editor
                .as_ref()
                .is_some_and(|e| !e.busy && e.error.is_some())
        })
    });
    cx.update_window(window.into(), |_, w, cx| {
        view.update(cx, |v, cx| {
            v.cancel_editor(w, cx);
            v.edit(Kind::Summary(pid.clone()), w, cx);
        });
    })
    .unwrap();
    let input = cx.update(|cx| view.read(cx).editor.as_ref().unwrap().input.clone());
    cx.update_window(window.into(), |_, w, cx| {
        input.update(cx, |i, cx| i.set_value("Rust\nSystems", w, cx));
        w.render_frame(cx);
        w.press("enter", cx);
    })
    .unwrap();
    cx.update(|cx| {
        assert!(view.read(cx).editor.is_some());
        assert_eq!(composer.read(cx).value(), "capture draft");
        assert_eq!(view.read(cx).model.capture().pending.len(), 0);
    });
    cx.update_window(window.into(), |_, w, cx| {
        input.update(cx, |i, cx| i.set_value("Rust\nSystems", w, cx));
        w.press("cmd-enter", cx);
    })
    .unwrap();
    tests::settle(cx, |cx| {
        cx.update(|cx| {
            view.read(cx)
                .projects
                .iter()
                .any(|p| p.id == pid && p.summary.as_deref() == Some("Rust\nSystems"))
                && view.read(cx).editor.is_none()
        })
    });
    cx.update_window(window.into(), |_, w, cx| {
        view.update(cx, |v, cx| {
            v.edit(Kind::Summary(pid.clone()), w, cx);
            v.editor
                .as_ref()
                .unwrap()
                .input
                .update(cx, |i, cx| i.set_value("discard", w, cx));
            v.cancel_editor(w, cx);
        });
    })
    .unwrap();
    cx.update(|cx| {
        assert!(
            view.read(cx)
                .projects
                .iter()
                .any(|p| p.id == pid && p.summary.as_deref() == Some("Rust\nSystems"))
        )
    });
    cx.update_window(window.into(), |_, w, cx| {
        view.update(cx, |v, cx| {
            v.edit(Kind::Summary(pid.clone()), w, cx);
            v.editor
                .as_ref()
                .unwrap()
                .input
                .update(cx, |i, cx| i.set_value(" \n ", w, cx));
            v.save_editor(w, cx);
        });
    })
    .unwrap();
    tests::settle(cx, |cx| {
        cx.update(|cx| {
            view.read(cx)
                .projects
                .iter()
                .any(|p| p.id == pid && p.summary.is_none())
                && view.read(cx).editor.is_none()
        })
    });
    // Composition in the summary owns Return/Escape; a deleted target produces recoverable save text.
    cx.update_window(window.into(), |_, w, cx| {
        view.update(cx, |v, cx| v.edit(Kind::Summary(pid.clone()), w, cx));
        let i = view.read(cx).editor.as_ref().unwrap().input.clone();
        i.update(cx, |i, cx| {
            i.replace_and_mark_text_in_range(None, "ni", Some(2..2), w, cx)
        });
        w.render_frame(cx);
        w.press("cmd-enter", cx);
    })
    .unwrap();
    cx.update(|cx| assert!(view.read(cx).editor.is_some()));
    cx.update_window(window.into(), |_, w, cx| {
        let input = view.read(cx).editor.as_ref().unwrap().input.clone();
        input.update(cx, |i, cx| {
            i.replace_and_mark_text_in_range(None, "ni", Some(2..2), w, cx)
        });
        w.render_frame(cx);
        w.press("escape", cx);
    })
    .unwrap();
    cx.update(|cx| assert!(view.read(cx).editor.is_some()));

    runtime.block_on(async {
        host.db
            .writer()
            .await
            .unwrap()
            .execute("DELETE FROM projects WHERE id=?", [&pid])
            .unwrap();
    });
    cx.update_window(window.into(), |_, w, cx| {
        view.update(cx, |v, cx| {
            v.editor.as_ref().unwrap().input.update(cx, |i, cx| {
                i.unmark_text(w, cx);
                i.set_value("recovery summary", w, cx);
            });
            v.save_editor(w, cx);
        });
    })
    .unwrap();
    tests::settle(cx, |cx| {
        cx.update(|cx| {
            view.read(cx)
                .editor
                .as_ref()
                .is_some_and(|e| !e.busy && e.error.is_some())
        })
    });
    cx.update(|cx| {
        assert_eq!(
            view.read(cx)
                .editor
                .as_ref()
                .unwrap()
                .input
                .read(cx)
                .value(),
            "recovery summary"
        )
    });
    cx.update_window(window.into(), |_, w, cx| {
        view.update(cx, |v, cx| {
            v.cancel_editor(w, cx);
            v.edit(Kind::Organization, w, cx);
        });
        let key = view.read(cx).editor.as_ref().unwrap().key.clone().unwrap();
        key.update(cx, |i, cx| {
            i.set_value("fixture-secret", w, cx);
            i.set_selected_range(0..14, cx);
        });
        cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string("sentinel".into()));
        w.render_frame(cx);
        w.press("cmd-c", cx);
        assert_eq!(
            cx.read_from_clipboard().and_then(|c| c.text()),
            Some("sentinel".into())
        );
        assert!(!key.read(cx).is_copyable());
        view.update(cx, |v, cx| v.save_editor(w, cx));
    })
    .unwrap();
    tests::settle(cx, |cx| {
        cx.update(|cx| {
            view.read(cx)
                .editor
                .as_ref()
                .is_some_and(|e| !e.busy && e.error.is_some())
        })
    });
    cx.update(|cx| {
        let e = view.read(cx).editor.as_ref().unwrap();
        assert_eq!(e.key.as_ref().unwrap().read(cx).value(), "fixture-secret");
        assert!(!e.error.as_ref().unwrap().contains("fixture-secret"));
    });
    cx.update_window(window.into(), |_, w, _cx| w.remove_window())
        .unwrap();
    services.cancel_operations();
    runtime.block_on(host.run_until(async { Ok(()) })).unwrap();
}
fn window_save(view: &Entity<Today>, w: &mut Window, cx: &mut App) {
    view.update(cx, |v, cx| v.save_editor(w, cx));
}

#[gpui_kit::test]
#[allow(clippy::too_many_lines)] // One owned LocalHost/window lifecycle demonstrates background routing and shutdown.
fn organization_retains_foreground_close_reopen_and_quit(cx: &mut TestAppContext) {
    use flicknote_client::dto::{Patch, ProjectAddInput, ProjectModifyInput};
    use flicknote_sync::organization::{Credential, run_with_endpoint};
    cx.executor().allow_parking();
    let root = tempfile::tempdir().unwrap();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let host = append_tests::fixture(&runtime, root.path());
    let services = append_tests::services(&host, &runtime);
    let AppResponse::Project(project) = runtime
        .block_on(host.app.handle(AppRequest::ProjectAdd(ProjectAddInput {
            name: "Systems".into(),
            color: None,
        })))
        .unwrap()
    else {
        panic!()
    };
    runtime
        .block_on(
            host.app
                .handle(AppRequest::ProjectModify(ProjectModifyInput {
                    id: project.id.clone(),
                    color: Patch::Value("#123456".into()),
                    summary: Patch::Value("Rust systems".into()),
                })),
        )
        .unwrap();
    let gate = Arc::new(tokio::sync::Notify::new());
    let received = tokio::sync::watch::channel(None::<serde_json::Value>).0;
    let calls = received.subscribe();
    let listener = runtime
        .block_on(tokio::net::TcpListener::bind((
            std::net::Ipv4Addr::LOCALHOST,
            0,
        )))
        .unwrap();
    let endpoint = format!("http://{}/decisions", listener.local_addr().unwrap());
    let requests = received.clone();
    let delayed = gate.clone();
    let provider=runtime.spawn(async move {
        let router=axum::Router::new().route("/decisions",axum::routing::post(move|axum::Json(body):axum::Json<serde_json::Value>|{let requests=requests.clone();let gate=delayed.clone();async move {
            requests.send_replace(Some(body.clone()));gate.notified().await;
            let answers:serde_json::Map<String,serde_json::Value>=body["questions"].as_object().unwrap().keys().map(|key|(key.clone(),serde_json::json!({"type":"choice","choice":"project_0","probabilities":{"project_0":0.9}}))).collect();
            axum::Json(serde_json::json!({"answers":answers}))
        }}));axum::serve(listener,router).await.unwrap();
    });
    let (key, credential) = tokio::sync::watch::channel(Credential {
        key: Some("fixture-secret".into()),
        generation: 1,
        catch_up: None,
    });
    let (status, _) = tokio::sync::watch::channel(None);
    let db = host.db.clone();
    let app = host.app.clone();
    let user = host.user_id.clone();
    let job = runtime.spawn(async move {
        run_with_endpoint(
            db,
            app,
            user,
            "2026-01-01T00:00:00Z".into(),
            flicknote_sync::organization::RoutingControl {
                credential,
                status,
                catch_up: tokio::sync::watch::channel(Default::default()).0,
            },
            (&endpoint, Duration::from_millis(10)),
        )
        .await;
    });
    services.track(&job);
    cx.update(|cx| {
        gpui_kit::init(cx);
        install_today_keys(cx);
    });
    let (window, view) = append_tests::open(cx, services.clone());
    tests::settle(cx, |cx| {
        cx.update(|cx| view.read(cx).loaded) && calls.borrow().is_some()
    });
    let composer = cx.update(|cx| view.read(cx).composer.clone());
    cx.update_window(window.into(), |_, w, cx| {
        w.input("draft while deciding", cx);
        w.render_frame(cx);
    })
    .unwrap();
    cx.update(|cx| assert_eq!(composer.read(cx).value(), "draft while deciding"));
    assert!(
        calls.borrow().as_ref().unwrap()["state"]["projects"]["project_0"]["summary"]
            .as_str()
            .unwrap()
            == "Rust systems"
    );
    // An actual summary edit invalidates the held request and the next compact payload includes the edit.
    runtime
        .block_on(
            host.app
                .handle(AppRequest::ProjectModify(ProjectModifyInput {
                    id: project.id.clone(),
                    color: Patch::Missing,
                    summary: Patch::Value("Updated summary".into()),
                })),
        )
        .unwrap();
    tests::settle(cx, |_| {
        calls
            .borrow()
            .as_ref()
            .is_some_and(|v| v["state"]["projects"]["project_0"]["summary"] == "Updated summary")
    });
    cx.update_window(window.into(), |_, w, _| w.remove_window())
        .unwrap();
    gate.notify_waiters();
    let count = || {
        runtime.block_on(async {
            host.db
                .writer()
                .await
                .unwrap()
                .query_row(
                    "SELECT count(*) FROM notes WHERE project_id=?",
                    [&project.id],
                    |r| r.get::<_, i64>(0),
                )
                .unwrap()
        })
    };
    tests::settle(cx, |_| count() == 1);
    let (reopened, reopened_view) = append_tests::open(cx, services.clone());
    tests::settle(cx, |cx| {
        cx.update(|cx| {
            reopened_view
                .read(cx)
                .model
                .rows
                .iter()
                .any(|r| r.id == 2 && r.project_color.as_deref() == Some("#123456"))
        })
    });
    cx.update(|cx| {
        assert_eq!(
            reopened_view.read(cx).composer.read(cx).value(),
            "draft while deciding"
        )
    });
    received.send_replace(None);
    runtime.block_on(async{host.db.writer().await.unwrap().execute("INSERT INTO notes(id,short_id,user_id,type,status,content,metadata,created_at) VALUES('00000000-0000-4000-8000-000000000003',3,'append-owner','normal','ready','later','{}',strftime('%Y-%m-%dT%H:%M:%SZ','now'))",[]).unwrap();});
    tests::settle(cx, |_| calls.borrow().is_some());
    cx.update_window(reopened.into(), |_, w, _| w.remove_window())
        .unwrap();
    let db = host.db.clone();
    let socket = host.socket.clone();
    runtime
        .block_on(host.run_until(async { Ok(()) }, || services.cancel_operations()))
        .unwrap();
    assert!(runtime.block_on(job).unwrap_err().is_cancelled());
    gate.notify_waiters();
    assert!(!socket.exists());
    assert_eq!(
        runtime.block_on(async {
            db.writer()
                .await
                .unwrap()
                .query_row("SELECT project_id FROM notes WHERE short_id=3", [], |r| {
                    r.get::<_, Option<String>>(0)
                })
                .unwrap()
        }),
        None
    );
    assert!(key.borrow().key.is_some());
    provider.abort();
}

#[gpui_kit::test]
#[allow(clippy::too_many_lines)] // Owned editor, provider and LocalHost lifetime share one manual run.
fn catch_up_control_preview_start_duplicates_close_reopen_stop_and_restart(
    cx: &mut TestAppContext,
) {
    use flicknote_sync::organization::CatchPhase;
    struct FakeKey;
    impl crate::organization::SecretStore for FakeKey {
        fn read(&self, _: &str) -> Result<Option<String>, String> {
            Ok(Some("fixture-secret".into()))
        }
        fn save(&self, _: &str, _: &str) -> Result<(), String> {
            Ok(())
        }
        fn remove(&self, _: &str) -> Result<(), String> {
            Ok(())
        }
    }
    cx.executor().allow_parking();
    let root = tempfile::tempdir().unwrap();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let host = append_tests::fixture(&runtime, root.path());
    let services = append_tests::services(&host, &runtime);
    let AppResponse::Project(_) = runtime
        .block_on(host.app.handle(AppRequest::ProjectAdd(
            flicknote_client::dto::ProjectAddInput {
                name: "Research".into(),
                color: None,
            },
        )))
        .unwrap()
    else {
        panic!("project")
    };
    let gate = Arc::new(tokio::sync::Notify::new());
    let (received, calls) = tokio::sync::watch::channel(None::<serde_json::Value>);
    let listener = runtime
        .block_on(tokio::net::TcpListener::bind((
            std::net::Ipv4Addr::LOCALHOST,
            0,
        )))
        .unwrap();
    let endpoint = format!("http://{}/decisions", listener.local_addr().unwrap());
    let delayed = gate.clone();
    let provider = runtime.spawn(async move {
        axum::serve(listener, axum::Router::new().route("/decisions", axum::routing::post(move |axum::Json(body): axum::Json<serde_json::Value>| {
            let gate = delayed.clone(); let received = received.clone(); async move {
                received.send_replace(Some(body.clone())); gate.notified().await;
                let answers: serde_json::Map<String, serde_json::Value> = body["questions"].as_object().unwrap().keys()
                    .map(|id| (id.clone(), serde_json::json!({"type":"choice","choice":"none","probabilities":{"none":0.8}}))).collect();
                axum::Json(serde_json::json!({"answers":answers}))
            }
        }))).await.unwrap();
    });
    let path = root.path().join("catch-preferences.json");
    let cutoff = (chrono::Utc::now() + chrono::Duration::days(1)).to_rfc3339();
    let control = crate::organization::start_with_provider(
        &services,
        path.clone(),
        Arc::new(FakeKey),
        cutoff.clone(),
        endpoint.clone(),
    );
    *services.organization.lock().unwrap() = Some(control.clone());
    tests::settle(cx, |_| control.state.borrow().ready);
    cx.update(|cx| {
        gpui_kit::init(cx);
        install_today_keys(cx);
    });
    let (window, view) = append_tests::open(cx, services.clone());
    tests::settle(cx, |cx| cx.update(|cx| view.read(cx).loaded));
    cx.update_window(window.into(), |_, w, cx| {
        view.update(cx, |v, cx| {
            v.composer
                .update(cx, |i, cx| i.set_value("retained draft", w, cx));
            v.edit(Kind::Organization, w, cx);
        });
    })
    .unwrap();
    tests::settle(cx, |_| control.catch_up.borrow().remaining == 1);
    assert!(calls.borrow().is_none());
    cx.update_window(window.into(), |_, w, cx| {
        w.render_frame(cx);
        w.click("catch-up-start-stop", cx);
    })
    .unwrap();
    assert!(
        calls.borrow().is_none(),
        "disabled Start performs no inference"
    );
    runtime
        .block_on(control.change(crate::organization::Change::Enable(true)))
        .unwrap();
    cx.update_window(window.into(), |_, w, cx| {
        w.render_frame(cx);
        w.click("catch-up-7-days", cx);
    })
    .unwrap();
    tests::settle(cx, |cx| {
        control.catch_up.borrow().days == 7
            && cx.update(|cx| !view.read(cx).editor.as_ref().unwrap().busy)
    });
    cx.update_window(window.into(), |_, w, cx| {
        w.render_frame(cx);
        w.click("catch-up-start-stop", cx);
    })
    .unwrap();
    tests::settle(cx, |cx| {
        calls.borrow().is_some() && cx.update(|cx| !view.read(cx).editor.as_ref().unwrap().busy)
    });
    assert_eq!(control.catch_up.borrow().phase, CatchPhase::Running);
    assert!(
        runtime
            .block_on(control.change(crate::organization::Change::Start(7)))
            .is_err()
    );
    cx.update_window(window.into(), |_, w, cx| {
        w.press("cmd-1", cx);
        w.press("alt-j", cx);
        assert!(view.read(cx).editor.is_some());
        w.click("cancel-editor", cx);
        assert!(view.read(cx).editor.is_none());
        w.remove_window();
    })
    .unwrap();
    assert_eq!(control.catch_up.borrow().phase, CatchPhase::Running);
    gate.notify_waiters();
    tests::settle(cx, |_| {
        control.catch_up.borrow().phase == CatchPhase::Complete
    });
    assert_eq!(control.catch_up.borrow().processed, 1);
    let (reopened, second) = append_tests::open(cx, services.clone());
    tests::settle(cx, |cx| cx.update(|cx| second.read(cx).loaded));
    cx.update_window(reopened.into(), |_, w, cx| {
        assert_eq!(second.read(cx).composer.read(cx).value(), "retained draft");
        second.update(cx, |v, cx| v.edit(Kind::Organization, w, cx));
        w.render_frame(cx);
        assert!(w.try_find("catch-up-progress").is_some());
    })
    .unwrap();
    assert_eq!(control.catch_up.borrow().processed, 1);
    assert_eq!(control.catch_up.borrow().days, 7);
    let saved: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(saved[&services.user_id]["cutoff"], cutoff);
    assert!(
        !std::fs::read_to_string(&path)
            .unwrap()
            .contains("fixture-secret")
    );
    runtime.block_on(async { host.db.writer().await.unwrap().execute("INSERT INTO notes(id,short_id,user_id,type,status,content,metadata,created_at) VALUES('00000000-0000-4000-8000-000000000003',3,'append-owner','normal','ready','historical','{}',strftime('%Y-%m-%dT%H:%M:%SZ','now','-1 hour'))", []).unwrap(); });
    cx.update_window(reopened.into(), |_, w, cx| {
        w.click("catch-up-start-stop", cx);
    })
    .unwrap();
    tests::settle(cx, |cx| {
        control.catch_up.borrow().phase == CatchPhase::Running
            && cx.update(|cx| !second.read(cx).editor.as_ref().unwrap().busy)
    });
    cx.update_window(reopened.into(), |_, w, cx| {
        w.render_frame(cx);
        w.click("catch-up-start-stop", cx);
    })
    .unwrap();
    tests::settle(cx, |_| {
        control.catch_up.borrow().phase == CatchPhase::Stopped
    });
    cx.update_window(reopened.into(), |_, w, _| w.remove_window())
        .unwrap();
    services.cancel_operations();
    let restarted = crate::organization::start_with_provider(
        &services,
        path,
        Arc::new(FakeKey),
        chrono::Utc::now().to_rfc3339(),
        endpoint,
    );
    tests::settle(cx, |_| restarted.state.borrow().ready);
    assert_eq!(restarted.catch_up.borrow().phase, CatchPhase::Preview);
    assert_eq!(restarted.catch_up.borrow().processed, 0);
    runtime
        .block_on(host.run_until(async { Ok(()) }, || services.cancel_operations()))
        .unwrap();
    provider.abort();
}
