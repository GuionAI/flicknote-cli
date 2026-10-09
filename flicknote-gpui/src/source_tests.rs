//! Real owned LocalHost, watch, storage, rendered Kit controls and simulated input.
use super::tests::settle;
use super::*;
use gpui_kit::{Focusable, TestAppContext, test::TestWindowExt};

fn services(
    host: &flicknote_sync::LocalHost,
    runtime: &tokio::runtime::Runtime,
    path: std::path::PathBuf,
) -> Arc<Services> {
    let mut services = Arc::try_unwrap(append_tests::services(host, runtime))
        .ok()
        .unwrap();
    services.source = crate::source::Control::start(&services, path);
    Arc::new(services)
}
fn minimum(window: &mut Window, cx: &mut App) {
    window.resize(size(px(760.), px(560.)));
    window.render_frame(cx);
    let header = window.find("destination-header").bounds();
    let control = window.find("only-mine");
    assert_eq!(control.label(), Some("Only mine"));
    assert!(control.bounds().right() <= header.right());
    assert!(control.bounds().left() >= header.left());
    assert!(control.bounds().size.width > px(60.));
}
fn tab_to_source(window: &mut Window, cx: &mut App) {
    for _ in 0..12 {
        window.press("tab", cx);
        window.render_frame(cx);
        if window.find("only-mine").focused() == Some(true) {
            return;
        }
    }
    panic!("Only mine must be keyboard focusable");
}
fn synthetic(runtime: &tokio::runtime::Runtime, root: &std::path::Path) -> crate::launch::Host {
    let host = runtime
        .block_on(flicknote_sync::spike::SpikeHost::start(
            root,
            0,
            2,
            Duration::from_millis(120),
        ))
        .unwrap();
    crate::launch::Host::Synthetic(host)
}

#[gpui_kit::test]
#[allow(clippy::too_many_lines)] // One window retains draft, reader and input identity through projection changes.
fn source_control_watch_detail_draft_keyboard_guards_and_stale_emissions(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let root = tempfile::tempdir().unwrap();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let host = append_tests::fixture(&runtime, root.path());
    runtime.block_on(async {
        host.db
            .writer()
            .await
            .unwrap()
            .execute(
                "UPDATE notes SET metadata='{\"created_by_ai\":false}' WHERE short_id=1",
                [],
            )
            .unwrap();
    });
    let services = services(&host, &runtime, root.path().join("source.json"));
    cx.update(|cx| {
        gpui_kit::init(cx);
        install_today_keys(cx);
    });
    let (window, view) = append_tests::open(cx, services.clone());
    settle(cx, |cx| {
        cx.update(|cx| view.read(cx).loaded && view.read(cx).model.rows.len() == 2)
    });
    let composer = cx.update(|cx| view.read(cx).composer.clone());
    let stale_scope = cx.update(|cx| {
        (
            view.read(cx).destination.clone(),
            false,
            view.read(cx).watch_epoch,
            view.read(cx).period.clone(),
        )
    });
    let stale_snapshot = cx.update(|cx| flicknote_sync::today::Snapshot {
        rows: view.read(cx).model.rows.clone(),
        projects: view.read(cx).projects.clone(),
        emission: 999,
        elapsed_ms: 0.,
        range: view.read(cx).range,
    });
    cx.update_window(window.into(), |_, w, cx| {
        w.render_frame(cx);
        w.click(("note", 1_u64), cx);
        composer.update(cx, |i, cx| {
            i.set_value("retained draft", w, cx);
            i.set_selected_range(4..4, cx);
        });
    })
    .unwrap();
    let reader = cx.update(|cx| view.read(cx).detail.as_ref().unwrap().state.clone());
    for mode in [
        gpui_kit::component::ThemeMode::Light,
        gpui_kit::component::ThemeMode::Dark,
    ] {
        cx.update_window(window.into(), |_, w, cx| {
            apply_theme(mode, cx);
            minimum(w, cx);
            w.click("only-mine", cx);
        })
        .unwrap();
        settle(cx, |cx| {
            cx.update(|cx| {
                view.read(cx).model.rows.len()
                    == if view.read(cx).source.human_only {
                        1
                    } else {
                        2
                    }
            })
        });
        cx.update(|cx| {
            assert_eq!(view.read(cx).model.selected, Some(1));
            assert_eq!(
                view.read(cx).detail.as_ref().unwrap().state.entity_id(),
                reader.entity_id()
            );
            assert_eq!(composer.read(cx).value(), "retained draft");
            assert_eq!(composer.read(cx).selected_range(), 4..4);
        });
    }
    cx.update_window(window.into(), |_, w, cx| {
        view.update(cx, |v, cx| {
            v.receive_snapshot(&stale_scope, Err("old same-source scope".into()), w, cx);
            assert!(v.watch_error.is_none());
        });
        tab_to_source(w, cx);
        w.press("space", cx);
        w.render_frame(cx);
        assert_eq!(w.find("only-mine").checked(), Some(true));
    })
    .unwrap();
    settle(cx, |cx| cx.update(|cx| view.read(cx).model.rows.len() == 1));
    // A delayed old emission (including an error) cannot replace rows/detail or reset destination.
    cx.update_window(window.into(), |_, w, cx| {
        view.update(cx, |v, cx| {
            v.receive_snapshot(&stale_scope, Ok(stale_snapshot.clone()), w, cx);
            v.receive_snapshot(&stale_scope, Err("stale".into()), w, cx);
            v.apply_source(crate::source::State::default(), w, cx);
            assert!(
                v.source.human_only,
                "older preference emission cannot reset choice"
            );
            assert_eq!(v.model.rows.len(), 1);
            assert!(v.watch_error.is_none());
        })
    })
    .unwrap();
    cx.update_window(window.into(), |_, w, cx| {
        composer.update(cx, |i, cx| {
            i.focus(w, cx);
            i.replace_and_mark_text_in_range(None, "ni", Some(2..2), w, cx);
        });
        view.update(cx, |v, cx| v.toggle_source(w, cx));
        assert!(view.read(cx).source.human_only);
        composer.update(cx, |i, cx| i.unmark_text(w, cx));
        view.update(cx, |v, cx| v.edit(project_editor::Kind::Add, w, cx));
        view.update(cx, |v, cx| v.toggle_source(w, cx));
        assert!(view.read(cx).source.human_only);
        view.update(cx, |v, cx| v.cancel_editor(w, cx));
        composer.update(cx, |i, cx| i.set_value("", w, cx));
        view.update(cx, |v, cx| v.toggle_source(w, cx));
    })
    .unwrap();
    settle(cx, |cx| cx.update(|cx| view.read(cx).model.rows.len() == 2));
    cx.update_window(window.into(), |_, w, cx| {
        w.render_frame(cx);
        w.click(("note", 2_u64), cx);
        composer.update(cx, |i, cx| i.set_value("accepted append", w, cx));
        w.render_frame(cx);
        w.press("enter", cx);
        view.update(cx, |v, cx| {
            v.toggle_source(w, cx);
            v.toggle_source(w, cx);
            v.toggle_source(w, cx);
        });
    })
    .unwrap();
    settle(cx, |cx| {
        cx.update(|cx| view.read(cx).model.rows.len() == 1 && !view.read(cx).detail_open)
    });
    cx.update(|cx| {
        assert_eq!(view.read(cx).model.selected, Some(1));
        assert!(view.read(cx).source.human_only);
    });
    runtime.block_on(async {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let text: String = host
                    .db
                    .writer()
                    .await
                    .unwrap()
                    .query_row("SELECT content FROM notes WHERE short_id=2", [], |r| {
                        r.get(0)
                    })
                    .unwrap();
                if text.ends_with("accepted append") {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
    });
    cx.update_window(window.into(), |_, w, cx| {
        assert!(composer.read(cx).focus_handle(cx).is_focused(w));
        w.remove_window();
    })
    .unwrap();
    drop(view);
    let (window, reopened) = append_tests::open(cx, services.clone());
    settle(cx, |cx| {
        cx.update(|cx| reopened.read(cx).loaded && reopened.read(cx).model.rows.len() == 1)
    });
    cx.update_window(window.into(), |_, w, _| w.remove_window())
        .unwrap();
    services.cancel_operations();
    runtime.block_on(host.shutdown());
}

#[gpui_kit::test]
fn initial_source_load_precedes_projection_without_holding_host_readiness(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let root = tempfile::tempdir().unwrap();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .max_blocking_threads(1)
        .build()
        .unwrap();
    let host = append_tests::fixture(&runtime, root.path());
    runtime.block_on(async {
        host.db
            .writer()
            .await
            .unwrap()
            .execute("UPDATE notes SET metadata=NULL WHERE short_id=1", [])
            .unwrap();
    });
    let path = root.path().join("source.json");
    std::fs::write(&path, r#"{"append-owner":true}"#).unwrap();
    let (release, gate) = std::sync::mpsc::channel();
    let (started, waiting) = std::sync::mpsc::channel();
    let blocked = runtime.spawn_blocking(move || {
        started.send(()).unwrap();
        gate.recv().unwrap();
    });
    waiting.recv_timeout(Duration::from_secs(5)).unwrap();
    let services = services(&host, &runtime, path);
    cx.update(|cx| {
        gpui_kit::init(cx);
        install_today_keys(cx);
    });
    let (window, view) = append_tests::open(cx, services.clone());
    cx.run_until_parked();
    cx.update(|cx| {
        assert!(!view.read(cx).loaded);
        assert!(view.read(cx).watch.is_none());
        assert!(view.read(cx).model.rows.is_empty());
    });
    runtime.block_on(async {
        assert!(
            tokio::time::timeout(
                Duration::from_secs(1),
                flicknote_client::DaemonClient::new(&host.socket).health()
            )
            .await
            .unwrap()
            .is_ok()
        );
    });
    release.send(()).unwrap();
    runtime.block_on(blocked).unwrap();
    settle(cx, |cx| cx.update(|cx| view.read(cx).loaded));
    cx.update(|cx| {
        assert!(view.read(cx).source.human_only);
        assert_eq!(view.read(cx).model.rows.len(), 1);
    });
    // A storage error is local to the control and never reverses the session choice.
    std::fs::create_dir(root.path().join("source.json.tmp")).unwrap();
    cx.update_window(window.into(), |_, w, cx| {
        w.render_frame(cx);
        w.click("only-mine", cx);
    })
    .unwrap();
    settle(cx, |cx| {
        cx.update(|cx| view.read(cx).source.error.is_some() && view.read(cx).model.rows.len() == 2)
    });
    cx.update_window(window.into(), |_, w, cx| {
        w.render_frame(cx);
        assert_eq!(
            w.find("retry-source").label(),
            Some("Retry saving Only mine")
        );
    })
    .unwrap();
    std::fs::remove_dir(root.path().join("source.json.tmp")).unwrap();
    cx.update_window(window.into(), |_, w, cx| w.click("retry-source", cx))
        .unwrap();
    settle(cx, |cx| {
        cx.update(|cx| view.read(cx).source.persisted && view.read(cx).source.error.is_none())
    });
    cx.update_window(window.into(), |_, w, _| w.remove_window())
        .unwrap();
    services.cancel_operations();
    runtime.block_on(host.shutdown());
}

#[gpui_kit::test]
fn source_project_pending_capture_and_reopen_preserve_identity(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let root = tempfile::tempdir().unwrap();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let host = synthetic(&runtime, root.path());
    let services = Arc::new(host.services(runtime.handle().clone()));
    cx.update(|cx| {
        gpui_kit::init(cx);
        install_today_keys(cx);
    });
    let (window, view) = append_tests::open(cx, services.clone());
    settle(cx, |cx| cx.update(|cx| view.read(cx).loaded));
    let project = cx.update(|cx| view.read(cx).projects[0].id.clone());
    let composer = cx.update(|cx| view.read(cx).composer.clone());
    // Hold only the owned writer so capture cannot complete before pending identity is checked.
    let writer = runtime.block_on(services.db.writer()).unwrap();
    cx.update_window(window.into(), |_, w, cx| {
        w.render_frame(cx);
        w.click("only-mine", cx);
        composer.update(cx, |i, cx| {
            i.set_value("human capture", w, cx);
            i.focus(w, cx);
        });
        w.render_frame(cx);
        w.press("enter", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(window.into(), |_, w, cx| {
        composer.update(cx, |i, cx| {
            i.set_value("next draft", w, cx);
            i.set_selected_range(3..3, cx);
        });
        assert_eq!(view.read(cx).model.capture().pending.len(), 1);
        view.update(cx, |v, cx| {
            v.change_destination(Destination::Project(project.clone()), w, cx)
        });
    })
    .unwrap();
    drop(writer);
    settle(cx, |cx| {
        cx.update(|cx| {
            view.read(cx).loaded
                && view.read(cx).destination == Destination::Project(project.clone())
                && view
                    .read(cx)
                    .model
                    .capture()
                    .pending
                    .iter()
                    .any(|p| p.id == Some(3))
        })
    });
    let rail = cx.update(|cx| view.read(cx).projects.len());
    cx.update_window(window.into(), |_, w, cx| {
        view.update(cx, |v, cx| v.change_destination(Destination::Home, w, cx));
    })
    .unwrap();
    settle(cx, |cx| cx.update(|cx| view.read(cx).model.rows.len() == 3));
    cx.update(|cx| {
        assert!(view.read(cx).model.capture().pending.is_empty());
        assert_eq!(composer.read(cx).value(), "next draft");
        assert_eq!(composer.read(cx).selected_range(), 3..3);
        assert_eq!(view.read(cx).projects.len(), rail);
    });
    // Marker update removes a selected detail in the current source scope; rail stays available.
    cx.update_window(window.into(), |_, w, cx| {
        w.render_frame(cx);
        w.click(("note", 3_u64), cx);
    })
    .unwrap();
    runtime.block_on(async {
        services
            .db
            .writer()
            .await
            .unwrap()
            .execute(
                "UPDATE notes SET metadata='{\"created_by_ai\":true}' WHERE short_id=3",
                [],
            )
            .unwrap();
    });
    settle(cx, |cx| {
        cx.update(|cx| view.read(cx).model.rows.len() == 2 && !view.read(cx).detail_open)
    });
    cx.update_window(window.into(), |_, w, _| w.remove_window())
        .unwrap();
    drop(view);
    let (window, reopened) = append_tests::open(cx, services.clone());
    settle(cx, |cx| cx.update(|cx| reopened.read(cx).loaded));
    cx.update(|cx| {
        assert!(reopened.read(cx).source.human_only);
        assert_eq!(reopened.read(cx).composer.read(cx).value(), "next draft");
        assert_eq!(reopened.read(cx).composer.read(cx).selected_range(), 3..3);
    });
    cx.update_window(window.into(), |_, w, _| w.remove_window())
        .unwrap();
    services.cancel_operations();
    runtime
        .block_on(host.run_until(async { Ok(()) }, || {}))
        .unwrap();
}

#[gpui_kit::test]
#[allow(clippy::too_many_lines)] // One owned window follows saved choice across utility/search scopes.
fn utility_sources_hidden_search_unfiltered_and_saved_choice_restored(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let root = tempfile::tempdir().unwrap();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let host = append_tests::fixture(&runtime, root.path());
    runtime.block_on(async {
        let writer = host.db.writer().await.unwrap();
        writer.execute("UPDATE notes SET content='utilityword', status='ai_failed', project_id='utility-project', metadata=CASE short_id WHEN 1 THEN '{}' ELSE metadata END", []).unwrap();
        writer.execute("INSERT INTO projects(id,user_id,name,is_archived) VALUES('utility-project','append-owner','Utility project',0)", []).unwrap();
        writer.execute("INSERT INTO note_shares(id,user_id,token) SELECT id,user_id,'owned-token' FROM notes", []).unwrap();
    });
    let path = root.path().join("source.json");
    std::fs::write(&path, r#"{"append-owner":true}"#).unwrap();
    let services = services(&host, &runtime, path.clone());
    cx.update(|cx| {
        gpui_kit::init(cx);
        install_today_keys(cx);
    });
    let (window, view) = append_tests::open(cx, services.clone());
    settle(cx, |cx| {
        cx.update(|cx| view.read(cx).loaded && view.read(cx).model.rows.len() == 1)
    });
    // Force a truthful persistence failure, which must disappear with the entire group.
    std::fs::create_dir(path.with_extension("json.tmp")).unwrap();
    services.source.choose(true);
    settle(cx, |cx| {
        cx.update(|cx| view.read(cx).source.error.is_some())
    });
    let composer = cx.update(|cx| view.read(cx).composer.clone());
    cx.update_window(window.into(), |_, w, cx| {
        composer.update(cx, |i, cx| {
            i.set_value("retained utility draft", w, cx);
            i.set_selected_range(4..4, cx);
        });
    })
    .unwrap();
    for destination in [
        Destination::Failed,
        Destination::Shared,
        Destination::Archive,
    ] {
        if destination == Destination::Archive {
            runtime.block_on(async {
                let writer = host.db.writer().await.unwrap();
                writer.execute("UPDATE notes SET deleted_at='2026-01-01'", []).unwrap();
                for (id, metadata) in [(3, "{}"), (4, r#"{"created_by_ai":true}"#)] {
                    writer.execute("INSERT INTO notes(id,short_id,user_id,content,type,status,is_flagged,metadata,created_at) VALUES(?,CAST(? AS INTEGER),'append-owner','utilityword','normal','ready',0,?,strftime('%Y-%m-%dT%H:%M:%SZ','now'))", [format!("00000000-0000-4000-8000-{id:012}"), id.to_string(), metadata.to_owned()]).unwrap();
                }
            });
        }
        cx.update_window(window.into(), |_, w, cx| {
            view.update(cx, |v, cx| v.change_destination(destination.clone(), w, cx));
        })
        .unwrap();
        settle(cx, |cx| {
            cx.update(|cx| view.read(cx).loaded && view.read(cx).model.rows.len() == 2)
        });
        for mode in [
            gpui_kit::component::ThemeMode::Light,
            gpui_kit::component::ThemeMode::Dark,
        ] {
            cx.update_window(window.into(), |_, w, cx| {
                apply_theme(mode, cx);
                w.resize(size(px(760.), px(560.)));
                w.render_frame(cx);
                for id in ["source-control", "only-mine", "retry-source"] {
                    assert!(
                        w.try_find(id).is_none(),
                        "hidden group must have no rendered/AX/focus target"
                    );
                }
                assert_eq!(
                    w.find(("note", 2_u64)).bounds().top(),
                    w.find("destination-header").bounds().bottom()
                );
                if destination == Destination::Failed {
                    assert!(
                        w.within(("note", 2_u64))
                            .try_find(("retry-note", 2_u64))
                            .is_some()
                    );
                }
                let v = view.read(cx);
                assert!(v.source.human_only && v.source.error.is_some());
                assert!(!v.action_scope().1);
                assert_eq!(composer.read(cx).value(), "retained utility draft");
                assert_eq!(composer.read(cx).selected_range(), 4..4);
            })
            .unwrap();
        }
        cx.update_window(window.into(), |_, w, cx| {
            view.update(cx, |v, cx| v.select(2, w, cx));
        })
        .unwrap();
        let (epoch, scope, snapshot, reader) = cx.update(|cx| {
            let v = view.read(cx);
            (
                v.watch_epoch,
                v.action_scope(),
                flicknote_sync::today::Snapshot {
                    rows: v.canonical_rows.clone(),
                    projects: v.projects.clone(),
                    emission: 999,
                    elapsed_ms: 0.,
                    range: v.range,
                },
                v.detail.as_ref().unwrap().state.entity_id(),
            )
        });
        // A late save notification changes preference presentation, not the effective scope.
        cx.update_window(window.into(), |_, w, cx| {
            view.update(cx, |v, cx| {
                let mut state = v.source.clone();
                state.generation += 1;
                state.human_only = false;
                v.apply_source(state.clone(), w, cx);
                state.generation += 1;
                state.human_only = true;
                v.apply_source(state, w, cx);
                assert_eq!(v.watch_epoch, epoch);
                assert_eq!(v.action_scope(), scope);
                assert_eq!(v.detail.as_ref().unwrap().state.entity_id(), reader);
                v.receive_snapshot(
                    &(destination.clone(), true, epoch, v.period.clone()),
                    Err("stale filtered scope".into()),
                    w,
                    cx,
                );
                assert!(v.watch_error.is_none());
                v.receive_snapshot(
                    &(destination.clone(), false, epoch, v.period.clone()),
                    Ok(snapshot.clone()),
                    w,
                    cx,
                );
                assert_eq!(v.model.rows.len(), 2);
            });
            let input = view.read(cx).search_input.clone();
            input.update(cx, |i, cx| i.set_value("utilityword", w, cx));
            view.update(cx, |v, cx| v.search_changed(w, cx));
        })
        .unwrap();
        settle(cx, |cx| {
            cx.update(|cx| !view.read(cx).search.loading && view.read(cx).search.active())
        });
        cx.update_window(window.into(), |_, w, cx| {
            w.render_frame(cx);
            assert!(w.try_find("source-control").is_none());
            // Archive-origin lexical discovery still searches all active notes.
            assert_eq!(
                view.read(cx).model.rows.len(),
                2,
                "origin={destination:?} query={} error={:?} hits={:?}",
                view.read(cx).search.query,
                view.read(cx).search.error,
                view.read(cx).search.hits
            );
            view.update(cx, |v, cx| {
                let generation = v.search.generation;
                let scope = v.action_scope();
                let epoch = v.watch_epoch;
                let mut state = v.source.clone();
                state.generation += 1;
                state.human_only = false;
                v.apply_source(state.clone(), w, cx);
                state.generation += 1;
                state.human_only = true;
                v.apply_source(state, w, cx);
                v.toggle_source(w, cx);
                assert!(v.source.human_only);
                assert_eq!(v.search.generation, generation);
                assert_eq!(v.watch_epoch, epoch);
                assert_eq!(v.action_scope(), scope);
                v.receive_search(generation, true, Err("stale filtered search".into()), w, cx);
                assert!(v.search.error.is_none());
                assert!(!v.action_scope().1);
                v.exit_search(true, w, cx);
            });
        })
        .unwrap();
        settle(cx, |cx| {
            cx.update(|cx| !view.read(cx).search.active() && view.read(cx).model.rows.len() == 2)
        });
    }
    runtime.block_on(async {
        let writer = host.db.writer().await.unwrap();
        writer
            .execute("DELETE FROM notes WHERE short_id IN (3,4)", [])
            .unwrap();
        writer
            .execute("UPDATE notes SET deleted_at=NULL", [])
            .unwrap();
    });
    for destination in [
        Destination::Home,
        Destination::Project("utility-project".into()),
    ] {
        cx.update_window(window.into(), |_, w, cx| {
            view.update(cx, |v, cx| v.change_destination(destination, w, cx));
        })
        .unwrap();
        settle(cx, |cx| {
            cx.update(|cx| view.read(cx).loaded && view.read(cx).model.rows.len() == 1)
        });
        cx.update_window(window.into(), |_, w, cx| {
            w.render_frame(cx);
            assert_eq!(w.find("only-mine").checked(), Some(true));
            assert!(w.try_find("retry-source").is_some());
        })
        .unwrap();
    }
    assert_eq!(
        std::fs::read_to_string(path).unwrap(),
        r#"{"append-owner":true}"#
    );
    cx.update_window(window.into(), |_, w, _| w.remove_window())
        .unwrap();
    services.cancel_operations();
    runtime.block_on(host.shutdown());
}

#[gpui_kit::test]
fn utility_watch_does_not_wait_for_or_restart_on_late_saved_preference(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let root = tempfile::tempdir().unwrap();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .max_blocking_threads(1)
        .build()
        .unwrap();
    let host = append_tests::fixture(&runtime, root.path());
    runtime.block_on(async {
        host.db.writer().await.unwrap().execute("UPDATE notes SET status='source_failed', metadata=CASE short_id WHEN 1 THEN '{}' ELSE metadata END", []).unwrap();
    });
    let path = root.path().join("source.json");
    std::fs::write(&path, r#"{"append-owner":true}"#).unwrap();
    let (release, gate) = std::sync::mpsc::channel();
    let (started, waiting) = std::sync::mpsc::channel();
    let blocked = runtime.spawn_blocking(move || {
        started.send(()).unwrap();
        gate.recv().unwrap();
    });
    waiting.recv_timeout(Duration::from_secs(5)).unwrap();
    let services = services(&host, &runtime, path);
    *services.destination.lock().unwrap() = Destination::Failed;
    cx.update(|cx| {
        gpui_kit::init(cx);
        install_today_keys(cx);
    });
    let (window, view) = append_tests::open(cx, services.clone());
    settle(cx, |cx| {
        cx.update(|cx| view.read(cx).loaded && view.read(cx).model.rows.len() == 2)
    });
    let epoch = cx.update(|cx| {
        assert!(!view.read(cx).source.ready);
        view.read(cx).watch_epoch
    });
    release.send(()).unwrap();
    runtime.block_on(blocked).unwrap();
    settle(cx, |cx| cx.update(|cx| view.read(cx).source.ready));
    cx.update_window(window.into(), |_, w, cx| {
        w.render_frame(cx);
        assert!(view.read(cx).source.human_only);
        assert_eq!(view.read(cx).watch_epoch, epoch);
        assert_eq!(view.read(cx).model.rows.len(), 2);
        assert!(w.try_find("source-control").is_none());
        w.remove_window();
    })
    .unwrap();
    drop(view);
    let (window, reopened) = append_tests::open(cx, services.clone());
    settle(cx, |cx| {
        cx.update(|cx| reopened.read(cx).loaded && reopened.read(cx).model.rows.len() == 2)
    });
    cx.update(|cx| assert!(reopened.read(cx).source.human_only));
    cx.update_window(window.into(), |_, w, _| w.remove_window())
        .unwrap();
    services.cancel_operations();
    runtime.block_on(host.shutdown());
}
