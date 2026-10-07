//! Calendar navigation through the production owned host and real rendered controls.
use super::tests::settle;
use super::*;
use gpui_kit::{TestAppContext, test::TestWindowExt};

fn loaded(cx: &mut TestAppContext, view: &Entity<Today>) {
    settle(cx, |cx| cx.update(|cx| view.read(cx).loaded));
}
fn seed(host: &flicknote_sync::LocalHost, runtime: &tokio::runtime::Runtime) {
    let previous = Period::Day(None)
        .shifted(false, &chrono::Local::now())
        .unwrap();
    let range = previous.range(&chrono::Local::now()).unwrap().unwrap();
    runtime.block_on(async {
        let writer = host.db.writer().await.unwrap();
        writer.execute("UPDATE notes SET created_at=?, metadata='{}' WHERE short_id=1", [range.0.to_rfc3339()]).unwrap();
        writer.execute("INSERT INTO projects(id,user_id,name,is_archived) VALUES('calendar-project','append-owner','Calendar',0)", []).unwrap();
        writer.execute("UPDATE notes SET project_id='calendar-project'", []).unwrap();
    });
}

#[gpui_kit::test]
#[allow(clippy::too_many_lines)] // One rendered journey retains editor identity and per-UUID process memory.
fn date_week_header_aliases_draft_guards_stale_watch_and_process_memory(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let root = tempfile::tempdir().unwrap();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let host = append_tests::fixture(&runtime, root.path());
    seed(&host, &runtime);
    let services = append_tests::services(&host, &runtime);
    cx.update(|cx| {
        gpui_kit::init(cx);
        install_today_keys(cx);
    });
    let (window, view) = append_tests::open(cx, services.clone());
    loaded(cx, &view);
    let composer = cx.update(|cx| view.read(cx).composer.clone());
    cx.update_window(window.into(), |_, w, cx| {
        w.render_frame(cx);
        assert_eq!(
            view.read(cx)
                .model
                .rows
                .iter()
                .map(|r| r.id)
                .collect::<Vec<_>>(),
            [2]
        );
        w.click("period-forward", cx);
        assert!(view.read(cx).period.follows_clock());
        w.press("alt-l", cx);
        assert_eq!(view.read(cx).period, Period::Day(None));
        w.press("alt-h", cx);
        assert!(
            view.read(cx).model.rows.is_empty(),
            "old rows discarded synchronously"
        );
    })
    .unwrap();
    loaded(cx, &view);
    let historical = cx.update(|cx| view.read(cx).period.clone());
    let stale_scope = cx.update(|cx| {
        (
            Destination::Home,
            false,
            view.read(cx).watch_epoch,
            historical.clone(),
        )
    });
    let stale = cx.update(|cx| flicknote_sync::today::Snapshot {
        rows: view.read(cx).model.rows.clone(),
        projects: view.read(cx).projects.clone(),
        emission: 123,
        elapsed_ms: 0.,
        range: view.read(cx).range,
    });
    cx.update_window(window.into(), |_, w, cx| {
        w.render_frame(cx);
        assert_eq!(view.read(cx).model.rows[0].id, 1);
        w.click(("note", 1_u64), cx);
        composer.update(cx, |i, cx| {
            i.set_value("ordinary draft", w, cx);
            i.set_selected_range(3..3, cx);
        });
        w.render_frame(cx);
        for key in ["alt-h", "alt-left", "alt-l", "alt-right"] {
            w.press(key, cx);
        }
        assert_eq!(view.read(cx).period, historical);
        assert_eq!(composer.read(cx).value(), "ordinary draft");
        let caret = composer.read(cx).selected_range();
        w.click("period-forward", cx); // Pointer permits ordinary draft.
        assert_eq!(view.read(cx).period, Period::Day(None));
        assert!(!view.read(cx).detail_open);
        assert_eq!(view.read(cx).model.selected, None);
        assert_eq!(composer.read(cx).selected_range(), caret);
        view.update(cx, |v, cx| {
            v.receive_snapshot(&stale_scope, Ok(stale.clone()), w, cx);
            v.receive_snapshot(&stale_scope, Err("stale period".into()), w, cx);
            assert!(v.model.rows.is_empty());
            assert!(v.watch_error.is_none());
        });
        composer.update(cx, |i, cx| i.set_value("", w, cx));
    })
    .unwrap();
    loaded(cx, &view);
    cx.update_window(window.into(), |_, w, cx| {
        w.render_frame(cx);
        w.press("alt-left", cx);
    })
    .unwrap();
    loaded(cx, &view);
    cx.update_window(window.into(), |_, w, cx| {
        w.render_frame(cx);
        assert_eq!(view.read(cx).period, historical);
        w.press("alt-right", cx);
    })
    .unwrap();
    loaded(cx, &view);
    cx.update_window(window.into(), |_, w, cx| {
        w.render_frame(cx);
        composer.update(cx, |i, cx| {
            i.replace_and_mark_text_in_range(None, "ni", Some(2..2), w, cx)
        });
        w.render_frame(cx);
        w.click("period-back", cx);
        assert_eq!(view.read(cx).period, Period::Day(None));
        view.update(cx, |v, cx| v.move_period(false, w, cx));
        assert_eq!(view.read(cx).period, Period::Day(None));
        composer.update(cx, |i, cx| {
            i.unmark_text(w, cx);
            i.set_value("", w, cx);
        });
        view.update(cx, |v, cx| v.edit(project_editor::Kind::Add, w, cx));
        view.update(cx, |v, cx| {
            v.mouse_period(false, w, cx);
            v.move_period(false, w, cx);
        });
        assert_eq!(view.read(cx).period, Period::Day(None));
        view.update(cx, |v, cx| v.cancel_editor(w, cx));
        w.render_frame(cx);
        w.press("cmd-2", cx);
    })
    .unwrap();
    loaded(cx, &view);
    cx.update_window(window.into(), |_, w, cx| {
        w.render_frame(cx);
        assert_eq!(
            view.read(cx).period,
            Period::All,
            "first project visit remains All"
        );
        assert_eq!(view.read(cx).model.rows.len(), 2);
        w.press("alt-h", cx);
        w.press("alt-right", cx);
        assert_eq!(view.read(cx).period, Period::All);
        // Use the actual tab focus path and Space, not a direct handler call.
        let mut focused = false;
        for _ in 0..16 {
            w.press("tab", cx);
            w.render_frame(cx);
            if w.find("scope-week").focused() == Some(true) {
                focused = true;
                break;
            }
        }
        assert!(focused, "Week must be reachable from the keyboard");
        w.press("space", cx);
    })
    .unwrap();
    loaded(cx, &view);
    for mode in [
        gpui_kit::component::ThemeMode::Light,
        gpui_kit::component::ThemeMode::Dark,
    ] {
        for width in [980., 760.] {
            cx.update_window(window.into(), |_, w, cx| {
                apply_theme(mode, cx);
                w.resize(size(px(width), px(560.)));
                w.render_frame(cx);
                w.click(("note", 2_u64), cx);
                w.render_frame(cx);
                let bounds = w.find("period-header").bounds();
                for id in [
                    "scope-all",
                    "scope-week",
                    "period-back",
                    "period-forward",
                    "period-current",
                ] {
                    let control = w.find(id);
                    assert!(control.bounds().left() >= bounds.left(), "{id}");
                    assert!(control.bounds().right() <= bounds.right(), "{id}");
                    assert!(control.bounds().size.height >= px(20.));
                }
                assert!(w.find("period-range").bounds().size.width > px(100.));
                w.click("period-forward", cx);
                assert!(view.read(cx).period.follows_clock());
                w.click("close-detail", cx);
            })
            .unwrap();
        }
    }
    cx.update_window(window.into(), |_, w, cx| {
        w.render_frame(cx);
        w.click("period-back", cx);
    })
    .unwrap();
    loaded(cx, &view);
    let week = cx.update(|cx| view.read(cx).period.clone());
    cx.update_window(window.into(), |_, w, cx| {
        w.render_frame(cx);
        w.click("scope-all", cx);
    })
    .unwrap();
    loaded(cx, &view);
    cx.update_window(window.into(), |_, w, cx| {
        w.render_frame(cx);
        w.click("scope-week", cx);
    })
    .unwrap();
    loaded(cx, &view);
    cx.update_window(window.into(), |_, w, cx| {
        assert_eq!(view.read(cx).period, week);
        w.render_frame(cx);
        w.click("rail-label-Home", cx);
    })
    .unwrap();
    loaded(cx, &view);
    cx.update_window(window.into(), |_, w, cx| {
        w.render_frame(cx);
        w.press("alt-h", cx);
    })
    .unwrap();
    loaded(cx, &view);
    cx.update_window(window.into(), |_, w, cx| {
        assert_eq!(view.read(cx).period, historical);
        view.update(cx, |v, cx| v.change_destination(Destination::Shared, w, cx));
    })
    .unwrap();
    loaded(cx, &view);
    cx.update_window(window.into(), |_, w, cx| {
        w.render_frame(cx);
        w.press("alt-h", cx);
        w.press("alt-right", cx);
        assert_eq!(view.read(cx).period, Period::All);
        w.render_frame(cx);
        w.click("rail-label-Home", cx);
    })
    .unwrap();
    loaded(cx, &view);
    cx.update_window(window.into(), |_, w, cx| {
        assert_eq!(view.read(cx).period, historical);
        w.remove_window();
    })
    .unwrap();
    cx.run_until_parked();
    let (window, view) = append_tests::open(cx, services.clone());
    loaded(cx, &view);
    cx.update_window(window.into(), |_, w, cx| {
        assert_eq!(view.read(cx).period, historical);
        w.render_frame(cx);
        w.press("cmd-2", cx);
    })
    .unwrap();
    loaded(cx, &view);
    cx.update_window(window.into(), |_, w, cx| {
        assert_eq!(view.read(cx).period, week);
        w.render_frame(cx);
        w.click("period-current", cx);
    })
    .unwrap();
    loaded(cx, &view);
    cx.update_window(window.into(), |_, w, cx| {
        assert_eq!(view.read(cx).period, Period::Week(None));
        w.render_frame(cx);
        w.press("cmd-1", cx);
    })
    .unwrap();
    loaded(cx, &view);
    cx.update_window(window.into(), |_, w, cx| {
        assert_eq!(view.read(cx).period, Period::Day(None));
        w.render_frame(cx);
        w.press("cmd-2", cx);
    })
    .unwrap();
    loaded(cx, &view);
    runtime.block_on(async {
        host.db
            .writer()
            .await
            .unwrap()
            .execute("UPDATE projects SET is_archived=1", [])
            .unwrap();
    });
    settle(cx, |cx| {
        cx.update(|cx| view.read(cx).destination == Destination::Home && view.read(cx).loaded)
    });
    cx.update_window(window.into(), |_, w, cx| {
        assert_eq!(view.read(cx).period, Period::Day(None));
        w.remove_window();
    })
    .unwrap();
    services.cancel_operations();
    runtime.block_on(host.shutdown());
}
