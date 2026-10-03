//! GPUI test windows and simulated IME protocol, not native OS candidate-window evidence.
use super::*;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{Focusable, TestAppContext, point};

fn settle(cx: &mut TestAppContext, predicate: impl Fn(&mut TestAppContext) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        cx.run_until_parked();
        if predicate(cx) {
            return;
        }
        assert!(Instant::now() < deadline, "UI operation deadline");
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[gpui_kit::test]
#[allow(clippy::too_many_lines)] // One real test window preserves focus/state across the interaction sequence.
fn rendered_creation_ime_multiline_selection_archive_and_recovery(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let root = tempfile::tempdir().unwrap();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let host = runtime
        .block_on(flicknote_sync::spike::SpikeHost::start(
            root.path(),
            0,
            5,
            Duration::from_millis(120),
        ))
        .unwrap();
    let services = Arc::new(Services {
        app: host.app.clone(),
        db: host.db.clone(),
        runtime: runtime.handle().clone(),
        operations: Mutex::new(vec![]),
    });
    cx.update(gpui_kit::init);
    let (window, view) = cx.update(|cx| {
        let (window, view) = gpui_kit::open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds {
                    origin: point(px(0.), px(0.)),
                    size: size(px(980.), px(720.)),
                })),
                ..Default::default()
            },
            cx,
            |window, cx| cx.new(|cx| Today::new(services, window, cx)),
        )
        .unwrap();
        (window.downcast::<gpui_kit::base::Root>().unwrap(), view)
    });
    settle(cx, |cx| cx.update(|cx| view.read(cx).model.rows.len() == 5));
    let composer = cx.update(|cx| view.read(cx).composer.clone());
    // Drive retained component through public IME protocol, then Return action.
    cx.update_window(window.into(), |_, window, cx| {
        composer.update(cx, |input, cx| {
            input.replace_and_mark_text_in_range(None, "ni", Some(2..2), window, cx)
        });
        window.render_frame(cx);
        window.press("enter", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        assert!(
            view.read(cx).model.pending.is_empty(),
            "composing Return must not submit"
        )
    });
    cx.update_window(window.into(), |_, window, cx| {
        composer.update(cx, |input, cx| {
            input.replace_text_in_range(None, "中文", window, cx)
        });
        window.press("shift-enter", cx);
        window.input("second line", cx);
    })
    .unwrap();
    cx.update(|cx| assert_eq!(composer.read(cx).value(), "中文\nsecond line"));
    cx.update_window(window.into(), |_, window, cx| window.press("enter", cx))
        .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        assert!(composer.read(cx).value().is_empty());
        assert_eq!(view.read(cx).model.pending.len(), 1);
    });
    settle(cx, |cx| {
        cx.update(|cx| {
            view.read(cx).model.rows.len() == 6 && view.read(cx).model.pending.is_empty()
        })
    });
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        let row = window.find(("note", 6_u64));
        assert_eq!(row.bounds().size.height, px(32.));
        window.click(("note", 6_u64), cx);
        assert_eq!(window.find(("note", 6_u64)).selected(), Some(true));
        assert!(composer.read(cx).focus_handle(cx).is_focused(window));
    })
    .unwrap();
    cx.run_until_parked();
    let detail = cx.update(|cx| {
        assert_eq!(view.read(cx).model.selected, Some(6));

        view.read(cx).detail.clone()
    });
    cx.update_window(window.into(), |_, window, cx| {
        detail.update(cx, |input, cx| input.focus(window, cx));
        window.press("cmd-a", cx);
        window.press("cmd-c", cx);
        assert_eq!(
            cx.read_from_clipboard().unwrap().text().unwrap(),
            "中文\nsecond line"
        );
        composer.update(cx, |input, cx| input.set_value("unsubmitted", window, cx));
        view.update(cx, |this, cx| this.archive(window, cx));
    })
    .unwrap();
    cx.update(|cx| assert!(!view.read(cx).archive_busy));
    cx.update_window(window.into(), |_, window, cx| {
        composer.update(cx, |input, cx| input.set_value("", window, cx));
        view.update(cx, |this, cx| this.archive(window, cx));
    })
    .unwrap();
    settle(cx, |cx| {
        cx.update(|cx| !view.read(cx).archive_busy && view.read(cx).model.rows.len() == 5)
    });
    cx.update(|cx| assert_eq!(view.read(cx).model.selected, Some(5)));
    // A failure never overwrites new typing; the recovery action retains it.
    cx.update_window(window.into(), |_, window, cx| {
        composer.update(cx, |input, cx| {
            input.set_value("[fixture-fail]", window, cx);
            input.focus(window, cx);
        });
        view.update(cx, |this, cx| this.submit(window, cx));
        composer.update(cx, |input, cx| input.set_value("new typing", window, cx));
    })
    .unwrap();
    settle(cx, |cx| {
        cx.update(|cx| view.read(cx).model.pending.is_empty())
    });
    cx.update(|cx| {
        assert_eq!(composer.read(cx).value(), "new typing");
        assert_eq!(view.read(cx).model.recovery, ["[fixture-fail]"]);
    });
    cx.update_window(window.into(), |_, window, _| window.remove_window())
        .unwrap();
    cx.update(|cx| {
        view.update(cx, |this, _| {
            this.watch.take();
            this.watch_task.take();
        })
    });
    runtime.block_on(host.shutdown());
}

/// Rendered bounds and real hit testing on GPUI's test platform; not native pixel evidence.
#[gpui_kit::test]
fn rows_fill_viewport_for_short_long_and_pending_previews(cx: &mut TestAppContext) {
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
        operations: Mutex::new(vec![]),
    });
    cx.update(gpui_kit::init);
    for width in [980., 360.] {
        let (window, view) = cx.update(|cx| {
            let (window, view) = gpui_kit::open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(Bounds {
                        origin: point(px(0.), px(0.)),
                        size: size(px(width), px(720.)),
                    })),
                    ..Default::default()
                },
                cx,
                |window, cx| cx.new(|cx| Today::new(services.clone(), window, cx)),
            )
            .unwrap();
            (window.downcast::<gpui_kit::base::Root>().unwrap(), view)
        });
        // Stop the empty fixture watch before installing only synthetic layout inputs.
        cx.update(|cx| {
            view.update(cx, |this, cx| {
                this.watch.take();
                this.watch_task.take();
                this.model.snapshot(Arc::new(
                    ["x".to_owned(), "Long preview ".repeat(100)]
                        .into_iter()
                        .enumerate()
                        .map(|(index, content)| flicknote_sync::spike::today::TodayRow {
                            id: index as i64 + 1,
                            uuid: format!("layout-{index}"),
                            preview: content.clone(),
                            content,
                        })
                        .collect(),
                ));
                this.model.accept("Pending preview ".repeat(100));
                cx.notify();
            })
        });
        cx.update_window(window.into(), |_, window, cx| {
            window.render_frame(cx);
            let viewport = window.find("today-notes").bounds();
            assert!(viewport.size.width <= px(width - 32.));
            for id in [1_u64, 2] {
                let bounds = window.find(("note", id)).bounds();
                assert_eq!(bounds.size.width, viewport.size.width);
                assert_eq!(bounds.size.height, px(32.));
                assert_eq!(bounds.origin.x, viewport.origin.x);
                // Click beyond short text at the viewport's right edge.
                window.click_at(("note", id), point(bounds.size.width - px(2.), px(16.)), cx);
                assert_eq!(window.find(("note", id)).selected(), Some(true));
                assert_eq!(
                    window.find(("note", id)).bounds().size.width,
                    viewport.size.width
                );
            }
            let pending = window.find(("pending", 1_u64)).bounds();
            assert_eq!(pending.size.width, viewport.size.width);
            assert_eq!(pending.size.height, px(32.));
            window.click_at(
                ("pending", 1_u64),
                point(pending.size.width - px(2.), px(16.)),
                cx,
            );
            assert_eq!(view.read(cx).model.selected, Some(2));
            window.remove_window();
        })
        .unwrap();
    }
    runtime.block_on(host.shutdown());
}
