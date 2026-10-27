//! Synthetic PowerSync host and rendered window; no live data or native app.
use super::tests::settle;
use super::*;
use gpui_kit::point;
use gpui_kit::test::TestWindowExt;

fn wheel(window: &mut Window, cx: &mut App, position: gpui_kit::Point<gpui_kit::Pixels>) {
    use gpui_kit::InputEvent as _;
    window.dispatch_event(
        gpui_kit::ScrollWheelEvent {
            position,
            delta: gpui_kit::ScrollDelta::Pixels(point(px(0.), px(-120.))),
            ..Default::default()
        }
        .to_platform_input(),
        cx,
    );
    window.render_frame(cx);
}

#[gpui_kit::test]
#[allow(clippy::too_many_lines)] // One owned window follows the complete modal journey.
fn saved_history_dialog_reads_one_snapshot_and_preserves_detail(cx: &mut gpui_kit::TestAppContext) {
    cx.executor().allow_parking();
    let root = tempfile::tempdir().unwrap();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let host = runtime
        .block_on(flicknote_sync::spike::SpikeHost::start(
            root.path(),
            0,
            2,
            Duration::ZERO,
        ))
        .unwrap();
    let encoded = "KLUv/SBV3QEAxAJ7IjE3OTE4MCI6IiMgT2xkXG5cbk9sZCBzYXZlZCBib2R5IiwxTmV3TmV3fQUAgAewA8ilqe0ywAU=";
    runtime.block_on(async {
        host.db
            .writer()
            .await
            .unwrap()
            .execute("UPDATE notes SET history=? WHERE short_id=2", [encoded])
            .unwrap();
        host.db
            .writer()
            .await
            .unwrap()
            .execute("UPDATE notes SET updated_at=NULL WHERE short_id=1", [])
            .unwrap();
    });
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
        source: crate::source::Control::default(),
        capture_changed: tokio::sync::watch::channel(()).0,
    });
    cx.update(|cx| {
        gpui_kit::init(cx);
        install_today_keys(cx);
    });
    let (window, view) = cx.update(|cx| {
        gpui_kit::open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds {
                    origin: point(px(0.), px(0.)),
                    size: size(px(760.), px(560.)),
                })),
                ..Default::default()
            },
            cx,
            |window, cx| cx.new(|cx| Today::new(services, window, cx)),
        )
        .unwrap()
    });
    let window = window.downcast::<gpui_kit::base::Root>().unwrap();
    settle(cx, |cx| cx.update(|cx| view.read(cx).model.rows.len() == 2));
    cx.update_window(window.into(), |_, w, cx| {
        view.update(cx, |this, cx| this.select(2, w, cx));
    })
    .unwrap();
    cx.update_window(window.into(), |_, w, cx| {
        let composer = view.read(cx).composer.clone();
        composer.update(cx, |input, cx| {
            input.set_value("Unsent draft", w, cx);
            input.set_selected_range(2..6, cx);
        });
        w.render_frame(cx);
        w.click("history-detail", cx);
        assert!(view.read(cx).history.as_ref().unwrap().loading());
    })
    .unwrap();
    settle(cx, |cx| {
        cx.update(|cx| {
            view.read(cx)
                .history
                .as_ref()
                .is_some_and(history::HistoryDialog::ready)
        })
    });
    for theme in [
        gpui_kit::component::ThemeMode::Light,
        gpui_kit::component::ThemeMode::Dark,
    ] {
        cx.update_window(window.into(), |_, w, cx| {
            apply_theme(theme, cx);
            w.render_frame(cx);
            assert!(w.try_find(("history-version", 0_u64)).is_some());
            assert!(w.try_find(("history-version", 2_u64)).is_some());
            assert!(w.try_find(("history-version", 1_u64)).is_some());
            let current = w.find(("history-version", 0_u64));
            let newest = w.find(("history-version", 2_u64));
            assert!(current.bounds().top() < newest.bounds().top());
            let older = w.find(("history-version", 1_u64));
            assert!(newest.bounds().top() < older.bounds().top());
            let expected = chrono::DateTime::parse_from_rfc3339("2026-10-12T10:13:21.000Z")
                .unwrap()
                .with_timezone(&chrono::Local)
                .format("%b %-d, %Y %H:%M")
                .to_string();
            assert_eq!(newest.label(), Some(format!("v2, {expected}").as_str()));
            let pane = w.find("history-pane").bounds();
            assert!(pane.size.width <= px(760.) && pane.size.height <= px(560.));
        })
        .unwrap();
    }
    settle(cx, |cx| {
        cx.update(|cx| {
            view.read(cx)
                .history
                .as_ref()
                .unwrap()
                .text(cx)
                .contains("https://example.com/synthetic-reading")
        })
    });
    cx.update_window(window.into(), |_, w, cx| {
        w.render_frame(cx);
        assert!(
            view.read(cx).history.as_ref().unwrap().list_focused(w),
            "version list should receive focus"
        );
        w.press("down", cx);
        w.press("down", cx);
    })
    .unwrap();
    settle(cx, |cx| {
        cx.update(|cx| {
            view.read(cx)
                .history
                .as_ref()
                .unwrap()
                .text(cx)
                .contains("Old saved body")
        })
    });
    cx.update_window(window.into(), |_, w, cx| {
        w.press("up", cx);
    })
    .unwrap();
    cx.update_window(window.into(), |_, w, cx| {
        w.click(("history-version", 1_u64), cx);
    })
    .unwrap();
    settle(cx, |cx| {
        cx.update(|cx| {
            view.read(cx)
                .history
                .as_ref()
                .unwrap()
                .text(cx)
                .contains("Old saved body")
        })
    });
    cx.update_window(window.into(), |_, w, cx| {
        let reader = view.read(cx).history.as_ref().unwrap().reader().unwrap();
        w.focus(&reader.read(cx).focus_handle().clone(), cx);
        w.render_frame(cx);
        w.press("cmd-a", cx);
        w.press("cmd-c", cx);
        assert!(view.read(cx).history.is_some());
        assert!(
            cx.read_from_clipboard()
                .unwrap()
                .text()
                .unwrap()
                .contains("Old saved body")
        );
    })
    .unwrap();
    cx.update_window(window.into(), |_, w, cx| {
        w.press("escape", cx);
        w.render_frame(cx);
        assert!(view.read(cx).history.is_none());
        assert_eq!(view.read(cx).model.selected, Some(2));
        assert!(view.read(cx).detail.is_some());
        assert_eq!(view.read(cx).composer.read(cx).value(), "Unsent draft");
        assert_eq!(view.read(cx).composer.read(cx).selected_range(), 2..6);
        w.click("history-detail", cx);
        view.update(cx, |this, cx| this.select(1, w, cx));
        assert!(view.read(cx).history.is_none());
    })
    .unwrap();
    cx.update_window(window.into(), |_, w, cx| {
        w.render_frame(cx);
        w.click("history-detail", cx);
    })
    .unwrap();
    settle(cx, |cx| {
        cx.update(|cx| {
            view.read(cx)
                .history
                .as_ref()
                .is_some_and(history::HistoryDialog::empty)
        })
    });
    cx.update_window(window.into(), |_, w, cx| {
        w.render_frame(cx);
        assert!(w.try_find(("history-version", 0_u64)).is_some());
        assert_eq!(w.find(("history-version", 0_u64)).label(), Some("Current"));
        assert!(!view.read(cx).history.as_ref().unwrap().text(cx).is_empty());
        let previous = view.read(cx).history.as_ref().unwrap().previous_focus();
        w.click_at("history-backdrop", point(px(5.), px(5.)), cx);
        assert!(view.read(cx).history.is_none());
        assert_eq!(w.focused(cx), previous);
        assert_eq!(view.read(cx).model.selected, Some(1));
        assert_eq!(view.read(cx).composer.read(cx).value(), "Unsent draft");
        assert_eq!(view.read(cx).composer.read(cx).selected_range(), 2..6);
    })
    .unwrap();
    runtime.block_on(async {
        host.db
            .writer()
            .await
            .unwrap()
            .execute("UPDATE notes SET history='bad!' WHERE short_id=1", [])
            .unwrap();
    });
    cx.update_window(window.into(), |_, w, cx| {
        w.render_frame(cx);
        w.click("history-detail", cx);
    })
    .unwrap();
    settle(cx, |cx| {
        cx.update(|cx| {
            view.read(cx)
                .history
                .as_ref()
                .is_some_and(history::HistoryDialog::failed)
        })
    });
    cx.update_window(window.into(), |_, w, cx| {
        w.render_frame(cx);
        w.click("close-history", cx);
    })
    .unwrap();
    let long = "KLUv/aB/YgEArQYA8gYXG3BpNQauQCWNIolVi27w3TGLJTL7/xTirf//zY3/////R6+lVErLoihJgiDH8YATaOreW2iICbfJxIIlkj3EoRCz2WTqXjbdlwNllEGp6yJkimy6hCM4SL3ShxEpIIAzyio9l3EU62X6QsMotmfTEhpFsf5Ms9AgivUyfaExFNuzaQkNoVh/plloBMV6mb7QAIrt2bSExk+sB38XGj6xnk0aaPQyudDoWSKhsb+EhYZekguNPUskNPSXsNDQS3KhoWeJhEZ/PTHeQFtztID7dkMc";
    runtime.block_on(async {
        host.db
            .writer()
            .await
            .unwrap()
            .execute("UPDATE notes SET history=? WHERE short_id=1", [long])
            .unwrap();
    });
    cx.update_window(window.into(), |_, w, cx| {
        w.render_frame(cx);
        w.click("history-detail", cx);
    })
    .unwrap();
    settle(cx, |cx| {
        cx.update(|cx| {
            view.read(cx)
                .history
                .as_ref()
                .is_some_and(history::HistoryDialog::ready)
        })
    });
    cx.update_window(window.into(), |_, w, cx| {
        w.render_frame(cx);
        w.click(("history-version", 20_u64), cx);
    })
    .unwrap();
    settle(cx, |cx| {
        cx.update(|cx| {
            view.read(cx)
                .history
                .as_ref()
                .unwrap()
                .text(cx)
                .contains("Long version 19")
        })
    });
    cx.update_window(window.into(), |_, w, cx| {
        w.render_frame(cx);
        let background = view.read(cx).list_scroll.0.borrow().base_handle.offset();
        let list_position = w.find("history-list").bounds().center();
        let reading_position = w.find("history-reading").bounds().center();
        wheel(w, cx, list_position);
        wheel(w, cx, reading_position);
        assert!(view.read(cx).history.is_some());
        let (list, reading) = view.read(cx).history.as_ref().unwrap().offsets();
        assert!(list < px(0.) && reading < px(0.));
        assert_eq!(
            view.read(cx).list_scroll.0.borrow().base_handle.offset(),
            background
        );
    })
    .unwrap();
    runtime.block_on(host.shutdown());
}
