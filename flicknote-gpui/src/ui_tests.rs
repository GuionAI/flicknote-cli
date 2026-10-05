//! GPUI test windows and simulated IME protocol, not native OS candidate-window evidence.
use super::*;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{Focusable, TestAppContext, point};

pub(super) fn settle(cx: &mut TestAppContext, predicate: impl Fn(&mut TestAppContext) -> bool) {
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
        destination: Mutex::default(),
        capture: Arc::default(),
        draft: std::sync::Mutex::default(),
        capture_changed: tokio::sync::watch::channel(()).0,
        user_id: flicknote_sync::spike::USER.into(),
        real_account: false,
        first_sync: std::sync::Mutex::default(),
    });
    cx.update(|cx| {
        gpui_kit::init(cx);
        install_today_keys(cx);
    });
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
            view.read(cx).model.capture().pending.is_empty(),
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
        assert_eq!(view.read(cx).model.capture().pending.len(), 1);
    });
    settle(cx, |cx| {
        cx.update(|cx| {
            view.read(cx).model.rows.len() == 6 && view.read(cx).model.capture().pending.is_empty()
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
    cx.update_window(window.into(), |_, window, cx| {
        assert_detail_dismissal_focus(window, cx, &view);
    })
    .unwrap();
    // Detail visibility is independent of selection and input; IME gets first Escape.
    cx.update_window(window.into(), |_, window, cx| {
        composer.update(cx, |input, cx| {
            input.set_value("keep this draft", window, cx);
            input.focus(window, cx);
        });
        window.render_frame(cx);
        window.click("close-detail", cx);
        assert!(!view.read(cx).detail_open);
        assert_eq!(view.read(cx).model.selected, Some(5));
        assert_eq!(composer.read(cx).value(), "keep this draft");
        // Open and replace through the exposed portion of the underlying rows.
        window.render_frame(cx);
        window.click_at(("note", 5_u64), point(px(4.), px(16.)), cx);
        assert!(view.read(cx).detail_open);
        window.render_frame(cx);
        window.click_at(("note", 4_u64), point(px(4.), px(16.)), cx);
        assert_eq!(view.read(cx).model.selected, Some(4));
        assert_eq!(
            view.read(cx).detail.read(cx).value(),
            view.read(cx)
                .model
                .rows
                .iter()
                .find(|r| r.id == 4)
                .unwrap()
                .content
        );
        composer.update(cx, |input, cx| {
            input.focus(window, cx);
            input.replace_and_mark_text_in_range(None, "ni", Some(2..2), window, cx);
        });
        window.render_frame(cx);
        window.press("escape", cx);
        assert!(
            view.read(cx).detail_open,
            "composition Escape must not dismiss detail"
        );
        assert!(composer.read(cx).focus_handle(cx).is_focused(window));
        window.press("escape", cx);
        assert!(!view.read(cx).detail_open);
        assert!(composer.read(cx).value().contains("keep this draft"));
        window.render_frame(cx);
        window.click_at(("note", 4_u64), point(px(4.), px(16.)), cx);
        window.render_frame(cx);
        // Exposed blank canvas closes only the reading surface.
        window.click_at("main-canvas", point(px(2.), px(280.)), cx);
        assert!(!view.read(cx).detail_open);
        assert_eq!(view.read(cx).model.selected, Some(4));
    })
    .unwrap();
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
        cx.update(|cx| view.read(cx).model.capture().pending.is_empty())
    });
    cx.update(|cx| {
        assert_eq!(composer.read(cx).value(), "new typing");
        assert_eq!(view.read(cx).model.capture().recovery, ["[fixture-fail]"]);
    });
    // Remove all but one through the actual host/watch, then remove the selected
    // final row while detail owns focus. A still-visible detail must keep focus.
    let ids = cx.update(|cx| {
        view.read(cx)
            .model
            .rows
            .iter()
            .map(|r| r.id)
            .collect::<Vec<_>>()
    });
    cx.update_window(window.into(), |_, window, cx| {
        view.update(cx, |this, cx| this.select(ids[0], window, cx));
        detail.update(cx, |input, cx| input.focus(window, cx));
        window.render_frame(cx);
    })
    .unwrap();
    for id in &ids[1..] {
        runtime
            .block_on(
                host.app
                    .handle(AppRequest::NoteArchive { id: id.to_string() }),
            )
            .unwrap();
    }
    settle(cx, |cx| cx.update(|cx| view.read(cx).model.rows.len() == 1));
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(detail.read(cx).focus_handle(cx).is_focused(window));
    })
    .unwrap();
    runtime
        .block_on(host.app.handle(AppRequest::NoteArchive {
            id: ids[0].to_string(),
        }))
        .unwrap();
    settle(cx, |cx| cx.update(|cx| view.read(cx).model.rows.is_empty()));
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("detail-surface").is_none());
        assert!(composer.read(cx).focus_handle(cx).is_focused(window));
        assert_eq!(composer.read(cx).value(), "new typing");
        assert_eq!(view.read(cx).model.selected, None);
        window.input(" continued", cx);
        // set_value retained the caret at the start; recovery must not move it.
        assert_eq!(composer.read(cx).value(), " continuednew typing");
        window.press("enter", cx);
    })
    .unwrap();
    settle(cx, |cx| {
        cx.update(|cx| {
            view.read(cx).model.rows.len() == 1 && view.read(cx).model.capture().pending.is_empty()
        })
    });
    cx.update_window(window.into(), |_, window, cx| {
        let id = view.read(cx).model.rows[0].id;
        view.update(cx, |this, cx| this.select(id, window, cx));
        detail.update(cx, |input, cx| input.focus(window, cx));
        window.render_frame(cx);
        tab_to_detail_button("archive", window, cx);
        window.press("enter", cx);
    })
    .unwrap();
    settle(cx, |cx| {
        cx.update(|cx| view.read(cx).model.rows.is_empty() && !view.read(cx).archive_busy)
    });
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("detail-surface").is_none());
        assert!(composer.read(cx).focus_handle(cx).is_focused(window));
        assert_eq!(view.read(cx).model.selected, None);
        window.input("after archive", cx);
        assert_eq!(composer.read(cx).value(), "after archive");
    })
    .unwrap();
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

fn assert_detail_dismissal_focus(window: &mut Window, cx: &mut App, view: &Entity<Today>) {
    let composer = view.read(cx).composer.clone();
    let detail = view.read(cx).detail.clone();
    for dismissal in [
        "button-close",
        "copy-escape",
        "copy-canvas",
        "close",
        "escape",
        "canvas",
        "home",
    ] {
        view.update(cx, |this, cx| this.select(5, window, cx));
        composer.update(cx, |input, cx| input.set_value("preserved", window, cx));
        detail.update(cx, |input, cx| input.focus(window, cx));
        window.render_frame(cx);
        assert!(detail.read(cx).focus_handle(cx).is_focused(window));
        match dismissal {
            "button-close" => {
                tab_to_detail_button("close-detail", window, cx);
                window.press("enter", cx);
            }
            "copy-escape" | "copy-canvas" => {
                tab_to_detail_button("copy-detail", window, cx);
                if dismissal == "copy-escape" {
                    window.press("escape", cx);
                } else {
                    window.click_at("main-canvas", point(px(2.), px(280.)), cx);
                }
            }
            "close" => window.click("close-detail", cx),
            "escape" => window.press("escape", cx),
            "home" => window.click("rail-label-Home", cx),
            _ => window.click_at("main-canvas", point(px(2.), px(280.)), cx),
        }
        window.render_frame(cx);
        assert!(
            !view.read(cx).detail_open,
            "{dismissal} did not close detail"
        );
        assert!(
            composer.read(cx).focus_handle(cx).is_focused(window),
            "{dismissal}"
        );
        assert_eq!(composer.read(cx).value(), "preserved");
        assert_eq!(view.read(cx).model.selected, Some(5));
        composer.update(cx, |input, cx| input.set_value("", window, cx));
        window.press("alt-j", cx);
        assert_eq!(view.read(cx).model.selected, Some(4));
        window.input("typing", cx);
        assert_eq!(composer.read(cx).value(), "typing");
    }
    view.update(cx, |this, cx| this.select(5, window, cx));
}

fn tab_to_detail_button(id: &'static str, window: &mut Window, cx: &mut App) {
    for _ in 0..8 {
        window.press("tab", cx);
        window.render_frame(cx);
        if window.find(id).focused() == Some(true) {
            return;
        }
    }
    panic!("Tab did not focus {id}");
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
        destination: Mutex::default(),
        capture: Arc::default(),
        draft: std::sync::Mutex::default(),
        capture_changed: tokio::sync::watch::channel(()).0,
        user_id: flicknote_sync::spike::USER.into(),
        real_account: false,
        first_sync: std::sync::Mutex::default(),
    });
    cx.update(gpui_kit::init);
    for width in [980., 760.] {
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
                            note_type: "normal".into(),
                            project_color: Some("05C7F7".into()),
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
                view.update(cx, |this, cx| this.close_detail(window, cx));
                window.render_frame(cx);
                let bounds = window.find(("note", id)).bounds();
                assert_eq!(bounds.size.width, viewport.size.width);
                assert_eq!(bounds.size.height, px(32.));
                assert_eq!(bounds.origin.x, viewport.origin.x);
                let row = window.within(("note", id));
                assert_eq!(row.find("type-glyph").bounds().size.width, px(17.));
                assert_eq!(row.find("project-dot").bounds().size, size(px(5.), px(5.)));
                // Click beyond short text at the viewport's right edge.
                window.click_at(("note", id), point(bounds.size.width - px(2.), px(16.)), cx);
                assert_eq!(window.find(("note", id)).selected(), Some(true));
                assert_eq!(
                    window.find(("note", id)).bounds().size.width,
                    window.find("today-notes").bounds().size.width
                );
            }
            view.update(cx, |this, cx| this.close_detail(window, cx));
            window.render_frame(cx);
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

/// Real temp host and rendered input/layout across supported viewports; no pixel claim.
#[gpui_kit::test]
#[allow(clippy::too_many_lines)] // Viewport/theme cases share one isolated host and interaction contract.
fn composer_detail_theme_and_final_row_remain_reachable(cx: &mut TestAppContext) {
    use gpui_kit::component::{Theme, ThemeMode};
    cx.executor().allow_parking();
    let root = tempfile::tempdir().unwrap();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let host = runtime
        .block_on(flicknote_sync::spike::SpikeHost::start(
            root.path(),
            0,
            80,
            Duration::ZERO,
        ))
        .unwrap();
    let services = Arc::new(Services {
        app: host.app.clone(),
        db: host.db.clone(),
        runtime: runtime.handle().clone(),
        operations: Mutex::new(vec![]),
        destination: Mutex::default(),
        capture: Arc::default(),
        draft: std::sync::Mutex::default(),
        capture_changed: tokio::sync::watch::channel(()).0,
        user_id: flicknote_sync::spike::USER.into(),
        real_account: false,
        first_sync: std::sync::Mutex::default(),
    });
    cx.update(gpui_kit::init);
    for (width, height) in [(980., 720.), (1440., 900.), (760., 560.)] {
        let (window, view) = cx.update(|cx| {
            let (window, view) = gpui_kit::open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(Bounds {
                        origin: point(px(0.), px(0.)),
                        size: size(px(width), px(height)),
                    })),
                    ..Default::default()
                },
                cx,
                |window, cx| cx.new(|cx| Today::new(services.clone(), window, cx)),
            )
            .unwrap();
            (window.downcast::<gpui_kit::base::Root>().unwrap(), view)
        });
        settle(cx, |cx| {
            cx.update(|cx| view.read(cx).model.rows.len() == 80)
        });
        for mode in [ThemeMode::Light, ThemeMode::Dark] {
            cx.update_window(window.into(), |_, window, cx| {
                apply_theme(mode, cx);
                view.read(cx)
                    .list_scroll
                    .scroll_to_item(0, gpui_kit::ScrollStrategy::Top);
                window.render_frame(cx);
                let composer = view.read(cx).composer.clone();
                let closed = window.find("composer-surface").bounds();
                let center = assert_closed_workbench(window, closed, width, height);
                assert_rail_alignment(window);
                let colors = Theme::global(cx).color_tokens();
                assert_painted_fill(
                    window,
                    window.find("rail-destinations").bounds(),
                    colors.secondary,
                );
                assert_neutral_composer_divider(window, colors.border);
                assert!(window.try_find("detail-surface").is_none());
                assert_eq!(Theme::global(cx).is_dark(), mode == ThemeMode::Dark);
                assert_eq!(
                    gpui_kit::base::Theme::global(cx).tokens,
                    Theme::global(cx).semantic_tokens()
                );
                assert_readable_rail_roles(Theme::global(cx).color_tokens());
                assert_workspace_theme_roles(mode, cx);
                window.hover(("note", 80_u64), cx);
                window.render_frame(cx);
                assert_painted_fill(
                    window,
                    window.find(("note", 80_u64)).bounds(),
                    Theme::global(cx).accent,
                );
                window.hover("destination-header", cx);
                window.render_frame(cx);
                composer.update(cx, |input, cx| {
                    input.set_value("First line\n第二行\nThird line", window, cx)
                });
                window.render_frame(cx);
                let grown = window.find("composer-surface").bounds();
                assert!(grown.size.height > closed.size.height);
                composer.update(cx, |input, cx| {
                    input.set_value("A long line that wraps ".repeat(100), window, cx)
                });
                window.render_frame(cx);
                window.render_frame(cx);
                let bounded = window.find("composer-surface").bounds();
                assert!(bounded.size.height >= grown.size.height);
                assert!(
                    bounded.size.height <= px(210.),
                    "six-line cap keeps composer bounded: {bounded:?}"
                );
                assert_eq!(bounded.bottom(), px(height));
                assert!(composer.read(cx).focus_handle(cx).is_focused(window));
                // The exact note list ends immediately above the dock, without filler rows.
                view.read(cx).list_scroll.scroll_to_bottom();
                window.render_frame(cx);
                let final_row = window.find(("note", 1_u64)).bounds();
                assert_eq!(final_row.bottom(), bounded.origin.y);
                assert_eq!(final_row.size.height, px(32.));
                assert_eq!(final_row.size.width, center.size.width);
                window.click_at(("note", 1_u64), point(px(4.), px(16.)), cx);
                window.render_frame(cx);
                assert_open_workbench(window, cx, &view, width, height);
                assert_detail_independent_of_composer(window, cx, &view);
                assert_bounded_workbench_feedback(window, cx, &view);
                window.click("archive", cx);
                assert!(!view.read(cx).archive_busy);
                assert_eq!(view.read(cx).model.rows.len(), 80);
                window.click("copy-detail", cx);
                assert_eq!(
                    cx.read_from_clipboard().unwrap().text().unwrap(),
                    view.read(cx).model.rows.last().unwrap().content
                );
                window.click("close-detail", cx);
                assert!(!view.read(cx).detail_open);
                assert!(composer.read(cx).value().len() > 100);
                // Inactive rail destinations never retarget selection/input.
                window.click_at("navigation-rail", point(px(100.), px(500.)), cx);
                assert_eq!(view.read(cx).model.selected, Some(1));
                assert!(composer.read(cx).focus_handle(cx).is_focused(window));
                composer.update(cx, |input, cx| input.set_value("", window, cx));
            })
            .unwrap();
        }
        cx.update_window(window.into(), |_, window, _| window.remove_window())
            .unwrap();
        cx.update(|cx| {
            view.update(cx, |this, _| {
                this.watch.take();
                this.watch_task.take();
            })
        });
    }
    runtime.block_on(host.shutdown());
}

fn assert_closed_workbench(
    window: &Window,
    closed: Bounds<gpui_kit::Pixels>,
    width: f32,
    height: f32,
) -> Bounds<gpui_kit::Pixels> {
    let center = window.find("center-pane").bounds();
    let rail = window.find("navigation-rail").bounds();
    assert_eq!(
        window.find("workspace-heading").bounds().bottom(),
        window.find("home").bounds().origin.y
    );
    assert_eq!(closed.size.width, center.size.width);
    assert_eq!(closed.origin.x, rail.right());
    assert_eq!(center.right(), px(width));
    assert_eq!(closed.bottom(), px(height));
    assert_eq!(rail.size.width, px(crate::workspace::RAIL_WIDTH));
    assert_eq!(rail.size.height, px(height));
    assert_eq!(
        window.find("destination-header").bounds().size.height,
        px(44.)
    );
    center
}

fn assert_open_workbench(
    window: &mut Window,
    cx: &mut App,
    view: &Entity<Today>,
    width: f32,
    height: f32,
) {
    let detail = window.find("detail-surface").bounds();
    let main = window.find("main-canvas").bounds();
    let dock = window.find("composer-surface").bounds();
    assert_eq!(detail.right(), px(width));
    assert_eq!(
        detail.size.width,
        px(crate::workspace::reading_width(width))
    );
    assert_eq!(detail.origin.x, main.right());
    assert_eq!(detail.origin.x, dock.right());
    assert_eq!(detail.bottom(), px(height));
    assert_eq!(detail.origin.y, px(0.));
    assert!(main.size.width >= px(272.));
    assert!(main.size.height >= px(200.));
    assert_eq!(dock.size.width, main.size.width);
    view.read(cx).list_scroll.scroll_to_bottom();
    window.render_frame(cx);
    let selected = window.find(("note", 1_u64)).bounds();
    assert_eq!(selected.size.width, main.size.width);
    assert_eq!(selected.bottom(), dock.origin.y);
    window.hover(("note", 1_u64), cx);
    window.render_frame(cx);
    assert_painted_fill(window, selected, Theme::global(cx).list_active);
}

fn assert_neutral_composer_divider(window: &Window, color: gpui_kit::Hsla) {
    let bounds = window
        .find("composer-surface")
        .bounds()
        .scale(window.scale_factor());
    let mut found = false;
    for quad in window.painted_quads() {
        if quad.bounds == bounds && quad.border_widths.top > px(0.).scale(window.scale_factor()) {
            assert_eq!(quad.border_color, color);
            found = true;
        }
    }
    assert!(found);
}

fn assert_painted_fill(window: &Window, bounds: Bounds<gpui_kit::Pixels>, color: gpui_kit::Hsla) {
    let bounds = bounds.scale(window.scale_factor());
    assert!(
        window.painted_quads().iter().any(|quad| {
            quad.bounds == bounds && quad.background == gpui_kit::solid_background(color)
        }),
        "expected semantic fill across the complete row"
    );
}

fn assert_rail_alignment(window: &Window) {
    let label_x = window.find("rail-label-Home").bounds().origin.x;
    for title in [
        "Ideas",
        "Reading",
        "Workspace",
        "Shared",
        "Archive",
        "Charts",
    ] {
        assert_eq!(
            window.find(format!("rail-label-{title}")).bounds().origin.x,
            label_x
        );
        assert_eq!(window.find(format!("rail-label-{title}")).focused(), None);
    }
}

fn assert_readable_rail_roles(colors: gpui_kit::base::ColorTokens) {
    let luminance = |color: gpui_kit::Hsla| {
        let rgb = color.to_rgb();
        let linear = |value: f32| {
            if value <= 0.04045 {
                value / 12.92
            } else {
                ((value + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * linear(rgb.r) + 0.7152 * linear(rgb.g) + 0.0722 * linear(rgb.b)
    };
    for background in [colors.background, colors.secondary, colors.surface] {
        let surface = luminance(background);
        for foreground in [colors.foreground, colors.secondary_foreground] {
            let text = luminance(foreground);
            assert!((text.max(surface) + 0.05) / (text.min(surface) + 0.05) >= 4.5);
        }
    }
}

fn assert_detail_independent_of_composer(window: &mut Window, cx: &mut App, view: &Entity<Today>) {
    let composer = view.read(cx).composer.clone();
    let detail = view.read(cx).detail.clone();
    detail.update(cx, |input, cx| input.focus(window, cx));
    window.render_frame(cx);
    assert!(!composer.read(cx).focus_handle(cx).is_focused(window));
    assert_neutral_composer_divider(window, Theme::global(cx).border);
    composer.update(cx, |input, cx| input.focus(window, cx));
    window.render_frame(cx);
    assert!(composer.read(cx).focus_handle(cx).is_focused(window));
    assert_neutral_composer_divider(window, Theme::global(cx).border);
    let long_detail = window.find("detail-surface").bounds();
    composer.update(cx, |input, cx| input.set_value("short", window, cx));
    window.render_frame(cx);
    window.render_frame(cx);
    let short_composer = window.find("composer-surface").bounds();
    let short_detail = window.find("detail-surface").bounds();
    assert_eq!(short_detail, long_detail);
    assert_eq!(short_detail.origin.x, short_composer.right());
    view.update(cx, |this, _| this.error = Some("Synthetic failure".into()));
    window.render_frame(cx);
    let feedback_composer = window.find("composer-surface").bounds();
    let feedback_detail = window.find("detail-surface").bounds();
    assert!(feedback_composer.size.height > short_composer.size.height);
    assert_eq!(feedback_detail, short_detail);
    assert_eq!(feedback_detail.origin.x, feedback_composer.right());
    for id in ["copy-detail", "archive", "close-detail"] {
        let action = window.find(id).bounds();
        assert!(action.origin.x >= feedback_detail.origin.x);
        assert!(action.right() <= feedback_detail.right());
        assert!(action.bottom() <= feedback_detail.bottom());
    }
    view.update(cx, |this, _| this.error = None);
    composer.update(cx, |input, cx| {
        input.set_value("A long line that wraps ".repeat(100), window, cx)
    });
    window.render_frame(cx);
    window.render_frame(cx);
}

#[gpui_kit::test]
fn app_local_shortcuts_preserve_input_and_follow_confirmed_selection(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let root = tempfile::tempdir().unwrap();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let host = runtime
        .block_on(flicknote_sync::spike::SpikeHost::start(
            root.path(),
            0,
            80,
            Duration::from_millis(100),
        ))
        .unwrap();
    let services = Arc::new(Services {
        app: host.app.clone(),
        db: host.db.clone(),
        runtime: runtime.handle().clone(),
        operations: Mutex::new(vec![]),
        destination: Mutex::default(),
        capture: Arc::default(),
        draft: std::sync::Mutex::default(),
        capture_changed: tokio::sync::watch::channel(()).0,
        user_id: flicknote_sync::spike::USER.into(),
        real_account: false,
        first_sync: std::sync::Mutex::default(),
    });
    cx.update(|cx| {
        gpui_kit::init(cx);
        install_today_keys(cx);
    });
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
    settle(cx, |cx| {
        cx.update(|cx| view.read(cx).model.rows.len() == 80)
    });
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert_initial_navigation(window, cx, &view);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(window.into(), |_, window, cx| {
        assert_navigation_shortcuts(window, cx, &view);
        assert_shortcut_input_guards(window, cx, &view);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(window.into(), |_, window, cx| {
        assert!(!view.read(cx).detail_open);
        assert!(view.read(cx).model.capture().pending.is_empty());
        let composer = view.read(cx).composer.clone();
        composer.update(cx, |input, cx| {
            input.unmark_text(window, cx);
            input.set_value("", window, cx);
        });
        window.press("alt-a", cx);
        assert!(view.read(cx).archive_busy);
    })
    .unwrap();
    settle(cx, |cx| {
        cx.update(|cx| !view.read(cx).archive_busy && view.read(cx).model.rows.len() == 79)
    });
    cx.update(|cx| assert_eq!(view.read(cx).model.selected, Some(2)));
    // A stale confirmed row after another actor archives it exercises guarded failure.
    cx.update(|cx| {
        view.update(cx, |this, _| {
            this.watch.take();
            this.watch_task.take();
        })
    });
    runtime
        .block_on(host.app.handle(AppRequest::NoteArchive { id: "2".into() }))
        .unwrap();
    cx.update_window(window.into(), |_, window, cx| window.press("alt-a", cx))
        .unwrap();
    settle(cx, |cx| cx.update(|cx| !view.read(cx).archive_busy));
    cx.update(|cx| {
        assert_eq!(view.read(cx).model.selected, Some(2));
        assert!(
            view.read(cx)
                .error
                .as_ref()
                .unwrap()
                .starts_with("Could not archive:")
        );
        assert_eq!(view.read(cx).model.rows.len(), 79);
    });
    cx.update_window(window.into(), |_, window, _| window.remove_window())
        .unwrap();
    runtime.block_on(host.shutdown());
}

fn assert_initial_navigation(window: &mut Window, cx: &mut App, view: &Entity<Today>) {
    use gpui_kit::InputHandler;
    let composer = view.read(cx).composer.clone();
    let mut native = crate::native_input::ComposerInput::new(composer.clone(), cx).unwrap();
    assert!(
        !native.prefers_ime_for_printable_keys(window, cx),
        "empty unmarked composer must offer Option bindings before an active Chinese IME"
    );
    assert_native_composer_contract(window, cx, &composer);
    // Replay the normalized US-layout macOS Option-J event, including the
    // printable character omitted by TestWindowExt::press("alt-j"). This
    // exercises GPUI dispatch, not AppKit's earlier input-context routing.
    window.dispatch_event(
        gpui_kit::PlatformInput::KeyDown(gpui_kit::KeyDownEvent {
            keystroke: gpui_kit::Keystroke::parse("alt-j->∆").unwrap(),
            is_held: false,
            prefer_character_input: false,
        }),
        cx,
    );
    assert_eq!(view.read(cx).model.selected, Some(80));
    view.update(cx, |this, _| this.model.selected = None);
    window.press("alt-k", cx);
    assert_eq!(view.read(cx).model.selected, Some(80));
    assert!(!view.read(cx).detail_open);
    window.press("alt-k", cx);
    assert_eq!(view.read(cx).model.selected, Some(80));
    view.update(cx, |this, _| this.model.selected = None);
    window.press("alt-j", cx);
    assert_eq!(view.read(cx).model.selected, Some(80));
    window.press("enter", cx);
}

fn assert_native_composer_contract(
    window: &mut Window,
    cx: &mut App,
    composer: &Entity<TextareaState>,
) {
    use gpui_kit::InputHandler;
    let bounds = composer.read(cx).text_bounds().unwrap();
    let mut native = crate::native_input::ComposerInput::new(composer.clone(), cx).unwrap();
    let mut kit = gpui_kit::ElementInputHandler::new(bounds, composer.clone());
    assert!(native.accepts_text_input(window, cx));
    assert_eq!(
        native.element_bounds(window, cx),
        kit.element_bounds(window, cx)
    );
    assert_eq!(
        native.bounds_for_range(0..0, window, cx),
        kit.bounds_for_range(0..0, window, cx)
    );
    // The pinned AppKit branch calls inputContext when callback propagation is
    // true. An unmatched first Chinese syllable letter must remain unconsumed.
    let result = window.dispatch_event(
        gpui_kit::PlatformInput::KeyDown(gpui_kit::KeyDownEvent {
            keystroke: gpui_kit::Keystroke::parse("n->n").unwrap(),
            is_held: false,
            prefer_character_input: false,
        }),
        cx,
    );
    assert!(result.propagate);
    assert!(composer.read(cx).value().is_empty());
    native.replace_and_mark_text_in_range(None, "ni", Some(2..2), window, cx);
    assert!(native.prefers_ime_for_printable_keys(window, cx));
    assert_eq!(
        native.marked_text_range(window, cx),
        kit.marked_text_range(window, cx)
    );
    native.replace_text_in_range(None, "你", window, cx);
    assert_eq!(composer.read(cx).value(), "你");
    assert!(native.prefers_ime_for_printable_keys(window, cx));
    assert_eq!(
        native.text_length_utf16(window, cx),
        kit.text_length_utf16(window, cx)
    );
    native.set_selected_text_range(0..1, window, cx);
    assert_eq!(
        native.selected_text_range(false, window, cx).unwrap().range,
        kit.selected_text_range(false, window, cx).unwrap().range
    );
    assert!(composer.read(cx).focus_handle(cx).is_focused(window));
    native.replace_text_in_range(Some(0..1), "", window, cx);
    native.unmark_text(window, cx);
    assert!(!native.prefers_ime_for_printable_keys(window, cx));
    window.render_frame(cx);
    let mut native = crate::native_input::ComposerInput::new(composer.clone(), cx).unwrap();
    let mut kit = gpui_kit::ElementInputHandler::new(
        composer.read(cx).text_bounds().unwrap(),
        composer.clone(),
    );
    assert_eq!(
        native.bounds_for_range(0..0, window, cx),
        kit.bounds_for_range(0..0, window, cx)
    );
}

fn assert_navigation_shortcuts(window: &mut Window, cx: &mut App, view: &Entity<Today>) {
    let composer = view.read(cx).composer.clone();
    assert!(view.read(cx).detail_open);
    window.press("alt-j", cx);
    assert_eq!(view.read(cx).model.selected, Some(79));
    assert_eq!(
        view.read(cx).detail.read(cx).value().as_ref(),
        view.read(cx).model.rows[1].content
    );
    assert!(composer.read(cx).focus_handle(cx).is_focused(window));
    window.press("escape", cx);
    for _ in 0..80 {
        window.press("alt-j", cx);
    }
    assert_eq!(view.read(cx).model.selected, Some(1));
    assert!(!view.read(cx).detail_open);
    window.render_frame(cx);
    let row = window.find(("note", 1_u64)).bounds();
    assert!(row.bottom() <= window.find("composer-surface").bounds().origin.y);
    assert!(row.origin.y >= window.find("today-notes").bounds().origin.y);
    assert!(composer.read(cx).focus_handle(cx).is_focused(window));
    view.update(cx, |this, cx| this.open_selected(window, cx));
    let detail = view.read(cx).detail.clone();
    detail.update(cx, |input, cx| input.focus(window, cx));
    window.press("alt-k", cx);
    assert_eq!(view.read(cx).model.selected, Some(2));
    assert!(detail.read(cx).focus_handle(cx).is_focused(window));
    window.press("cmd-a", cx);
    window.press("cmd-c", cx);
    assert_eq!(
        cx.read_from_clipboard().unwrap().text().unwrap(),
        detail.read(cx).value().as_ref()
    );
    window.press("alt-j", cx);
    assert_eq!(view.read(cx).model.selected, Some(1));
    view.update(cx, |this, cx| this.close_detail(window, cx));
}

fn assert_shortcut_input_guards(window: &mut Window, cx: &mut App, view: &Entity<Today>) {
    use gpui_kit::InputHandler;
    let composer = view.read(cx).composer.clone();
    composer.update(cx, |input, cx| input.set_value("draft", window, cx));
    assert!(
        crate::native_input::ComposerInput::new(composer.clone(), cx)
            .unwrap()
            .prefers_ime_for_printable_keys(window, cx)
    );
    for key in ["alt-j", "alt-k", "alt-a"] {
        window.press(key, cx);
    }
    assert_eq!(view.read(cx).model.selected, Some(1));
    assert!(!view.read(cx).archive_busy);
    window.press("cmd-a", cx);
    window.press("cmd-c", cx);
    assert_eq!(cx.read_from_clipboard().unwrap().text().unwrap(), "draft");
    window.press("cmd-x", cx);
    window.press("cmd-v", cx);
    assert_eq!(composer.read(cx).value(), "draft");
    composer.update(cx, |input, cx| {
        input.set_value("", window, cx);
        input.replace_and_mark_text_in_range(None, "ni", Some(2..2), window, cx);
    });
    assert!(
        crate::native_input::ComposerInput::new(composer.clone(), cx)
            .unwrap()
            .prefers_ime_for_printable_keys(window, cx)
    );
    for key in ["alt-j", "alt-k", "alt-a", "enter"] {
        window.press(key, cx);
    }
    assert_eq!(view.read(cx).model.selected, Some(1));
    assert!(!view.read(cx).detail_open);
    assert!(!view.read(cx).archive_busy);
    assert!(view.read(cx).model.capture().pending.is_empty());
    // Simulate native composition ending before the queued PressEnter callback.
    // Capture-phase composition must still prevent open or submission.
    composer.update(cx, |input, cx| {
        input.unmark_text(window, cx);
        input.set_value("", window, cx);
    });
}

/// Synthetic recovery/status stress at the same viewport/theme; no network effects.
fn assert_bounded_workbench_feedback(window: &mut Window, cx: &mut App, view: &Entity<Today>) {
    let composer = view.read(cx).composer.clone();
    let detail = window.find("detail-surface").bounds();
    view.update(cx, |this, cx| {
        this.error = Some("Synthetic capture failure\n".repeat(40));
        this.watch_error = Some("Synthetic watch failure".into());
        this.sync_progress = Some(crate::sync_progress::Progress::Percent(90));
        this.sync_message = Some("90% — Finishing sync…".into());
        let mut capture = this.model.capture();
        capture.recovery.push("Retained recovery".into());
        capture.uncertain.push((
            "Retained uncertain".into(),
            flicknote_client::WireError {
                code: "note_create_unknown".into(),
                message: "Synthetic unknown".into(),
                details: Some(serde_json::json!({"note_id": "owned-canonical-reference"})),
                retryable: false,
            },
        ));
        cx.notify();
    });
    window.render_frame(cx);
    window.render_frame(cx);
    let dock = window.find("composer-surface").bounds();
    let feedback = window.find("capture-feedback").bounds();
    let list = window.find("main-canvas").bounds();
    assert!(feedback.size.height <= px(120.));
    assert!(list.size.height >= px(150.));
    assert_eq!(list.bottom(), dock.origin.y);
    assert_eq!(dock.right(), detail.origin.x);
    assert_eq!(detail, window.find("detail-surface").bounds());
    assert!(window.find("list-status").bounds().size.height <= px(128.));
    assert!(window.find("first-sync-progress").bounds().size.height > px(0.));
    // Stabilized flex geometry must not feed scroll position back into sizing.
    view.read(cx).list_scroll.scroll_to_bottom();
    window.render_frame(cx);
    let last = window.find(("note", 1_u64)).bounds();
    assert_eq!(last.bottom(), dock.origin.y);
    window.render_frame(cx);
    assert_eq!(last, window.find(("note", 1_u64)).bounds());
    assert_eq!(dock, window.find("composer-surface").bounds());
    assert!(composer.read(cx).focus_handle(cx).is_focused(window));
    view.update(cx, |this, cx| {
        this.error = None;
        this.watch_error = None;
        this.sync_progress = None;
        this.sync_message = None;
        this.model.capture().recovery.clear();
        this.model.capture().uncertain.clear();
        cx.notify();
    });
    window.render_frame(cx);
}

fn assert_workspace_theme_roles(mode: gpui_kit::component::ThemeMode, cx: &App) {
    let theme = Theme::global(cx);
    let pair = |light, dark| gpui_kit::rgb(if mode.is_dark() { dark } else { light }).into();
    let colors = theme.color_tokens();
    assert_eq!(colors.background, pair(0xfafafc, 0x252830));
    assert_eq!(colors.secondary, pair(0xeceef2, 0x20232a));
    assert_eq!(colors.surface, pair(0xffffff, 0x282c34));
    assert_eq!(colors.accent, pair(0xe4e8ef, 0x303641));
    assert_eq!(colors.muted, colors.accent);
    assert_eq!(theme.list_active, pair(0xd6e3f5, 0x384963));
    assert_eq!(colors.border, pair(0xd6dae2, 0x3b414d));
    assert_ne!(colors.secondary, colors.surface);
    assert_ne!(colors.muted, colors.selection);
    assert_ne!(colors.accent, colors.selection);
    assert_eq!(colors.foreground, pair(0x242831, 0xe3e6ec));
    assert_eq!(colors.secondary_foreground, pair(0x505766, 0xaab2c0));
    assert_eq!(colors.muted_foreground, pair(0x606878, 0xa0a9b8));
    assert_eq!(colors.primary, pair(0x2864b4, 0x80adfa));
    assert_eq!(colors.primary_foreground, pair(0xffffff, 0x20232a));
    assert_eq!(theme.caret, colors.primary);
    assert_eq!(colors.ring, theme.caret);
    assert_eq!(colors.input, colors.border);
    assert_eq!(theme.radius, px(4.));
    assert_eq!(colors.destructive, pair(0xa12d35, 0xf4a0a5));
    assert_eq!(theme.button, colors.surface);
    assert_eq!(theme.button_hover, colors.accent);
    assert_eq!(theme.button_active, theme.list_active);
    assert_eq!(theme.button_foreground, colors.foreground);
}

#[gpui_kit::test]
#[allow(clippy::too_many_lines)]
fn rendered_real_account_uses_production_creation_and_reopens_fresh(cx: &mut TestAppContext) {
    use axum::{Json, Router, routing::post};
    use serde_json::{Value, json};
    cx.executor().allow_parking();
    let root = tempfile::tempdir().unwrap();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let listener = runtime
        .block_on(tokio::net::TcpListener::bind((
            std::net::Ipv4Addr::LOCALHOST,
            0,
        )))
        .unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let gate = Arc::new(tokio::sync::Semaphore::new(0));
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let requests = Arc::new(Mutex::new(Vec::<String>::new()));
    let server_requests = requests.clone();
    let server_gate = gate.clone();
    let server_calls = calls.clone();
    let server = runtime.spawn(async move {
        let router = Router::new()
            .route(
                "/rest/v1/notes",
                post(move |Json(mut note): Json<Value>| {
                    let gate = server_gate.clone();
                    let calls = server_calls.clone();
                    let requests = server_requests.clone();
                    async move {
                        let first = {
                            let mut requests = requests.lock().unwrap();
                            let id = note["id"].as_str().unwrap().to_owned();
                            let first = !requests.contains(&id);
                            requests.push(id);
                            first
                        };
                        if first {
                            calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        }
                        let content = note["content"].as_str().unwrap_or_default().to_owned();
                        if content.starts_with("closed-") {
                            if first {
                                gate.acquire().await.unwrap().forget();
                            }
                            if content == "closed-rejected" {
                                return Err(axum::http::StatusCode::BAD_REQUEST);
                            }
                            if content == "closed-unknown" {
                                return Err(axum::http::StatusCode::SERVICE_UNAVAILABLE);
                            }
                        }
                        note["short_id"] = if content == "closed-partial" {
                            Value::Null
                        } else {
                            json!(80)
                        };
                        note["is_flagged"] = json!(false);
                        note["summary"] = Value::Null;
                        note["source"] = Value::Null;
                        note["deleted_at"] = Value::Null;
                        Ok(Json(json!([note])))
                    }
                }),
            )
            .fallback(|| async { axum::http::StatusCode::SERVICE_UNAVAILABLE });
        axum::serve(listener, router).await.unwrap();
    });
    let mut config = flicknote_core::profile::load(root.path()).unwrap();
    config.supabase_url.clone_from(&origin);
    config.supabase_anon_key = "test-key".into();
    config.powersync_url.clone_from(&origin);
    config.api_url.clone_from(&origin);
    config.gateway_url = origin;
    flicknote_auth::session::save_session(
        &config.paths.session_file,
        &flicknote_auth::client::AuthSession {
            access_token: "test-access".into(),
            refresh_token: "test-refresh".into(),
            expires_at: Some(0),
            user: flicknote_auth::client::AuthUser {
                id: "real-test-account".into(),
                email: None,
            },
        },
    )
    .unwrap();
    let session_file = config.paths.session_file.clone();
    let (_cancel, receiver) = tokio::sync::watch::channel(false);
    let host = runtime
        .block_on(flicknote_sync::LocalHost::start(config, Some(0), receiver))
        .unwrap();
    let services = Arc::new(Services {
        app: host.app.clone(),
        db: host.db.clone(),
        runtime: runtime.handle().clone(),
        operations: Mutex::default(),
        destination: Mutex::default(),
        capture: Arc::default(),
        draft: std::sync::Mutex::default(),
        capture_changed: tokio::sync::watch::channel(()).0,
        user_id: host.user_id.clone(),
        real_account: true,
        first_sync: std::sync::Mutex::default(),
    });
    cx.update(gpui_kit::init);
    let view = cx.update(|cx| {
        gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
            cx.new(|cx| Today::new(services.clone(), window, cx))
        })
        .unwrap()
        .1
    });
    settle(cx, |cx| {
        cx.update(|cx| {
            view.read(cx)
                .watch
                .as_ref()
                .is_some_and(|w| w.receiver.borrow().is_some())
        })
    });
    cx.update(|cx| {
        assert!(view.read(cx).model.rows.is_empty());
        assert!(view.read(cx).projects.is_empty());
        assert!(!view.read(cx).first_synced);
    });
    // The fake refresh returns503: connector's "Auth error" is not proof of bad credentials.
    settle(cx, |cx| {
        cx.update(|cx| {
            view.read(cx)
                .sync_message
                .as_deref()
                .is_some_and(|s| s.starts_with("Sync unavailable."))
        })
    });
    cx.update(|cx| {
        assert_eq!(
            view.read(cx).sync_message.as_deref(),
            Some("Sync unavailable. Cached notes remain available; new notes need a connection.")
        )
    });
    // Restore this test-owned session's expiry without changing its account identity.
    let mut session = flicknote_auth::session::load_session(&session_file).unwrap();
    session.expires_at = Some(u64::MAX);
    flicknote_auth::session::save_session(&session_file, &session).unwrap();
    let window = cx.update(|cx| cx.windows()[0]);
    cx.update(|cx| {
        cx.update_window(window, |_, window, cx| {
            view.update(cx, |this, cx| {
                this.composer.update(cx, |input, cx| {
                    input.set_value("真实账户 capture", window, cx)
                });
                this.submit(window, cx);
                assert!(this.composer.read(cx).value().is_empty());
            })
        })
        .unwrap()
    });
    settle(cx, |cx| {
        cx.update(|cx| {
            view.read(cx).model.rows.len() == 1 && view.read(cx).model.capture().pending.is_empty()
        })
    });
    cx.update(|cx| {
        let row = &view.read(cx).model.rows[0];
        assert_eq!(row.id, 80);
        assert_eq!(row.content, "真实账户 capture");
    });
    cx.update(|cx| {
        cx.update_window(window, |_, window, cx| {
            view.read(cx).composer.clone().update(cx, |input, cx| {
                input.set_value("retained draft", window, cx);
                input.set_selected_range(4..4, cx);
            });
        })
        .unwrap();
    });
    cx.update(|cx| {
        cx.update_window(window, |_, window, _| window.remove_window())
            .unwrap()
    });
    drop(view);
    let client = flicknote_client::DaemonClient::new(&host.socket);
    assert!(runtime.block_on(client.health()).is_ok());
    runtime
        .block_on(client.app(AppRequest::NoteWrite {
            id: "80".into(),
            content: "Updated while closed".into(),
        }))
        .unwrap();
    let reopened = cx.update(|cx| {
        gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
            cx.new(|cx| Today::new(services.clone(), window, cx))
        })
        .unwrap()
        .1
    });
    settle(cx, |cx| {
        cx.update(|cx| {
            reopened
                .read(cx)
                .model
                .rows
                .first()
                .is_some_and(|r| r.content == "Updated while closed")
        })
    });
    cx.update(|cx| {
        let composer = reopened.read(cx).composer.read(cx);
        assert_eq!(composer.value().as_ref(), "retained draft");
        assert_eq!(composer.selected_range(), 4..4);
        assert!(!reopened.read(cx).detail_open);
        assert!(reopened.read(cx).model.selected.is_none());
    });
    let mut reopened = reopened;
    for (index, text) in ["closed-rejected", "closed-unknown", "closed-partial"]
        .into_iter()
        .enumerate()
    {
        let window = cx.update(|cx| cx.windows()[0]);
        cx.update(|cx| {
            cx.update_window(window, |_, window, cx| {
                reopened.update(cx, |this, cx| {
                    this.composer
                        .update(cx, |input, cx| input.set_value(text, window, cx));
                    this.submit(window, cx);
                });
                window.remove_window();
            })
            .unwrap();
        });
        drop(reopened);
        cx.run_until_parked();
        settle(cx, |_| {
            calls.load(std::sync::atomic::Ordering::SeqCst) == index + 2
        });
        gate.add_permits(1);
        // Wait for the production create task to finish while no window exists.
        settle(cx, |_| {
            services
                .operations
                .lock()
                .unwrap()
                .iter()
                .all(tokio::task::AbortHandle::is_finished)
        });
        assert!(runtime.block_on(client.health()).is_ok());
        reopened = cx.update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
                cx.new(|cx| Today::new(services.clone(), window, cx))
            })
            .unwrap()
            .1
        });
        cx.run_until_parked();
        cx.update(|cx| {
            let this = reopened.read(cx);
            if index == 0 {
                assert_eq!(this.composer.read(cx).value().as_ref(), text);
                assert!(
                    this.error
                        .as_deref()
                        .unwrap()
                        .starts_with("Could not save:")
                );
            } else {
                assert!(this.composer.read(cx).value().is_empty());
                assert!(this.model.capture().recovery.is_empty());
                assert_eq!(this.model.capture().uncertain.len(), index);
                let capture = this.model.capture();
                let (original, error) = capture.uncertain.last().unwrap();
                assert_eq!(original, text);
                assert_eq!(
                    error.code,
                    if index == 1 {
                        "note_create_unknown"
                    } else {
                        "note_create_partial"
                    }
                );
                assert_eq!(
                    error.details.as_ref().unwrap()["note_id"].as_str().unwrap(),
                    requests.lock().unwrap()[if index == 1 { 2 } else { 4 }]
                );
                assert!(
                    this.error
                        .as_deref()
                        .unwrap()
                        .contains("Do not submit this note again.")
                );
            }
        });
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), index + 2);
    }
    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 5); // Existing creator retries unknown once with the SAME UUID.
    assert_eq!(requests[2], requests[3]);
    assert_eq!(
        requests
            .iter()
            .collect::<std::collections::HashSet<_>>()
            .len(),
        4
    );
    drop(requests);
    services.cancel_operations();
    runtime.block_on(host.shutdown());
    server.abort();
}

#[gpui_kit::test]
#[allow(clippy::too_many_lines)] // One rendered window carries operation identity across watch swaps.
fn project_click_watch_swap_capture_and_fallback_preserve_composer(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let root = tempfile::tempdir().unwrap();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let host = runtime
        .block_on(flicknote_sync::spike::SpikeHost::start(
            root.path(),
            0,
            5,
            Duration::from_millis(150),
        ))
        .unwrap();
    let services = Arc::new(Services {
        app: host.app.clone(),
        db: host.db.clone(),
        runtime: runtime.handle().clone(),
        operations: Mutex::default(),
        destination: Mutex::default(),
        capture: Arc::default(),
        draft: std::sync::Mutex::default(),
        capture_changed: tokio::sync::watch::channel(()).0,
        user_id: flicknote_sync::spike::USER.into(),
        real_account: false,
        first_sync: std::sync::Mutex::default(),
    });
    cx.update(|cx| {
        gpui_kit::init(cx);
        install_today_keys(cx);
    });
    let (window, view) = cx.update(|cx| {
        let (window, view) = gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
            cx.new(|cx| Today::new(services.clone(), window, cx))
        })
        .unwrap();
        (window.downcast::<gpui_kit::base::Root>().unwrap(), view)
    });
    settle(cx, |cx| cx.update(|cx| view.read(cx).projects.len() == 3));
    let project = cx.update(|cx| view.read(cx).projects[0].id.clone());
    runtime.block_on(async {
        host.db.writer().await.unwrap().execute("INSERT INTO notes(id,user_id,short_id,project_id,content,type,created_at) VALUES('historical',?,500,?,'Historical canonical','normal','2020-01-01T12:00:00Z')", [flicknote_sync::spike::USER, project.as_str()]).unwrap();
    });
    cx.update_window(window.into(), |_, window, cx| {
        view.update(cx, |this, cx| {
            this.select(5, window, cx);
            this.composer.update(cx, |input, cx| {
                input.set_value("Draft to retain", window, cx);
                input.replace_text_in_range(Some(5..5), "!", window, cx);
            });
        });
        window.render_frame(cx);
        window.click(
            gpui_kit::SharedString::from(format!("project-{project}")),
            cx,
        );
        let this = view.read(cx);
        assert_eq!(this.destination, Destination::Project(project.clone()));
        assert!(this.model.rows.is_empty());
        assert!(this.model.selected.is_none());
        assert!(!this.detail_open);
        assert_eq!(this.composer.read(cx).value().as_ref(), "Draft! to retain");
    })
    .unwrap();
    settle(cx, |cx| {
        cx.update(|cx| {
            view.read(cx)
                .model
                .rows
                .first()
                .is_some_and(|r| r.id == 500)
        })
    });
    cx.update_window(window.into(), |_, window, cx| {
        assert_project_note_navigation(window, cx, &view);
        // A capture remains global while switching twice before its completion.
        view.update(cx, |this, cx| {
            this.submit(window, cx);
            this.change_destination(Destination::Home, window, cx);
            this.change_destination(Destination::Project(project.clone()), window, cx);
            assert_eq!(this.model.capture().pending.len(), 1);
        });
    })
    .unwrap();
    settle(cx, |cx| {
        cx.update(|cx| {
            view.read(cx)
                .model
                .capture()
                .pending
                .first()
                .is_some_and(|p| p.id.is_some())
        })
    });
    cx.update(|cx| {
        assert!(view.read(cx).model.rows.iter().all(|r| r.id != 501));
        assert_eq!(view.read(cx).model.rows[0].id, 500);
    });
    cx.update_window(window.into(), |_, window, cx| {
        view.update(cx, |this, cx| {
            // The production host tests establish these create failure DTOs;
            // here their completion is delivered while another surface is active.
            for (code, id) in [
                ("note_create_unknown", None),
                ("note_create_partial", Some(9000)),
            ] {
                let token = this.model.accept("uncertain capture".into());
                this.model.uncertain(
                    token,
                    flicknote_client::WireError {
                        code: code.into(),
                        message: "Do not create again".into(),
                        retryable: false,
                        details: Some(
                            serde_json::json!({"note_id":"uncertain-uuid","short_id":id}),
                        ),
                    },
                );
            }
            this.select(500, window, cx);
            assert_eq!(
                this.detail.read(cx).value().as_ref(),
                "Historical canonical"
            );
            this.change_destination(Destination::Home, window, cx);
            this.change_destination(Destination::Project(project.clone()), window, cx);
            assert!(!this.detail_open);
            assert!(this.model.selected.is_none());
            assert_eq!(this.model.capture().uncertain.len(), 2);
            assert!(this.model.capture().recovery.is_empty());
        });
    })
    .unwrap();
    runtime.block_on(async {
        host.db.writer().await.unwrap().execute("INSERT INTO notes(id,user_id,short_id,content,type,created_at) VALUES('uncertain-uuid',?,9000,'Acknowledged partial','normal',strftime('%Y-%m-%dT%H:%M:%SZ','now'))", [flicknote_sync::spike::USER]).unwrap();
    });
    // Archive the selected project while a composition is active: fallback must
    // not reject the watch-driven change or clear marked text.
    cx.update_window(window.into(), |_, window, cx| {
        view.update(cx, |this, cx| {
            this.composer.update(cx, |input, cx| {
                input.replace_and_mark_text_in_range(None, "ni", Some(2..2), window, cx)
            });
        });
    })
    .unwrap();
    runtime.block_on(async {
        host.db
            .writer()
            .await
            .unwrap()
            .execute("UPDATE projects SET is_archived=1 WHERE id=?", [&project])
            .unwrap();
    });
    settle(cx, |cx| {
        cx.update(|cx| {
            view.read(cx).destination == Destination::Home
                && view.read(cx).model.capture().pending.is_empty()
        })
    });
    cx.update_window(window.into(), |_, window, cx| {
        view.update(cx, |this, cx| {
            assert_eq!(this.composer.read(cx).value().as_ref(), "ni");
            assert!(this.composing(window, cx));
            assert!(this.model.rows.iter().all(|r| r.id != 500));
            assert!(!this.projects.iter().any(|p| p.id == project));
            assert_eq!(this.model.capture().uncertain.len(), 2);
            assert_eq!(
                this.model.capture().uncertain[1]
                    .1
                    .details
                    .as_ref()
                    .unwrap()["note_id"],
                "uncertain-uuid"
            );
            assert!(this.model.capture().recovery.is_empty());
        });
        window.remove_window();
    })
    .unwrap();
    drop(view);
    assert!(
        runtime
            .block_on(flicknote_client::DaemonClient::new(&host.socket).health())
            .is_ok()
    );
    runtime.block_on(host.shutdown());
}

#[gpui_kit::test]
#[allow(clippy::too_many_lines)] // One window verifies draft, marked, keyboard and reopen state together.
fn destination_numbers_and_option_bounds_follow_the_rendered_rail(cx: &mut TestAppContext) {
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
    runtime.block_on(async {
        let writer = host.db.writer().await.unwrap();
        for index in 0..7 {
            let id = format!("00000000-0000-4000-8000-{index:012}");
            let name = format!("A project {index}");
            writer
                .execute(
                    "INSERT INTO projects(id,user_id,name,is_archived) VALUES(?,?,?,0)",
                    [id.as_str(), flicknote_sync::spike::USER, name.as_str()],
                )
                .unwrap();
        }
    });
    let services = Arc::new(Services {
        app: host.app.clone(),
        db: host.db.clone(),
        runtime: runtime.handle().clone(),
        operations: Mutex::default(),
        destination: Mutex::default(),
        capture: Arc::default(),
        draft: std::sync::Mutex::default(),
        capture_changed: tokio::sync::watch::channel(()).0,
        user_id: flicknote_sync::spike::USER.into(),
        real_account: false,
        first_sync: std::sync::Mutex::default(),
    });
    cx.update(|cx| {
        gpui_kit::init(cx);
        install_today_keys(cx);
    });
    let (window, view) = cx.update(|cx| {
        let (window, view) = gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
            cx.new(|cx| Today::new(services.clone(), window, cx))
        })
        .unwrap();
        (window.downcast::<gpui_kit::base::Root>().unwrap(), view)
    });
    settle(cx, |cx| cx.update(|cx| view.read(cx).projects.len() == 10));
    let projects = cx.update(|cx| view.read(cx).projects.clone());
    cx.update_window(window.into(), |_, window, cx| {
        let composer = view.read(cx).composer.clone();
        composer.update(cx, |input, cx| {
            input.set_value("retained draft", window, cx);
            input.set_selected_range(3..3, cx);
        });
        for number in 2..=9 {
            window.render_frame(cx);
            window.press(&format!("cmd-{number}"), cx);
            assert_eq!(
                view.read(cx).destination,
                Destination::Project(projects[number - 2].id.clone())
            );
            assert_eq!(composer.read(cx).value().as_ref(), "retained draft");
            assert_eq!(composer.read(cx).selected_range(), 3..3);
            assert!(composer.read(cx).focus_handle(cx).is_focused(window));
        }
        window.render_frame(cx);
        window.press("alt-down", cx);
        assert_eq!(
            view.read(cx).destination,
            Destination::Project(projects[7].id.clone())
        );
        window.press("cmd-1", cx);
        assert_eq!(view.read(cx).destination, Destination::Home);
        composer.update(cx, |input, cx| {
            input.set_value("", window, cx);
            input.replace_and_mark_text_in_range(None, "ni", Some(2..2), window, cx);
        });
        window.render_frame(cx);
        window.press("cmd-2", cx);
        window.press("alt-down", cx);
        assert_eq!(view.read(cx).destination, Destination::Home);
        composer.update(cx, |input, cx| {
            input.unmark_text(window, cx);
            input.set_value("", window, cx);
        });
        window.render_frame(cx);
        window.press("alt-up", cx);
        assert_eq!(view.read(cx).destination, Destination::Home);
        for project in projects.iter() {
            window.render_frame(cx);
            window.press("alt-down", cx);
            assert_eq!(
                view.read(cx).destination,
                Destination::Project(project.id.clone())
            );
        }
        window.render_frame(cx);
        window.press("alt-down", cx);
        assert_eq!(
            view.read(cx).destination,
            Destination::Project(projects[9].id.clone())
        );
        for _ in 0..10 {
            window.render_frame(cx);
            window.press("alt-up", cx);
        }
        assert_eq!(view.read(cx).destination, Destination::Home);
        // Native editing continues to own ordinary draft movement and undo.
        composer.update(cx, |input, cx| input.set_value("abc", window, cx));
        window.render_frame(cx);
        window.press("cmd-a", cx);
        window.press("cmd-c", cx);
        assert_eq!(cx.read_from_clipboard().unwrap().text(), Some("abc".into()));
        window.press("left", cx);
        assert_eq!(composer.read(cx).selected_range(), 0..0);
        composer.update(cx, |input, cx| input.set_value("", window, cx));
        view.update(cx, |this, cx| {
            this.change_destination(Destination::Project(projects[0].id.clone()), window, cx)
        });
        window.remove_window();
    })
    .unwrap();
    drop(view);
    let (reopened_window, reopened) = cx.update(|cx| {
        let (window, view) = gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
            cx.new(|cx| Today::new(services.clone(), window, cx))
        })
        .unwrap();
        (window.downcast::<gpui_kit::base::Root>().unwrap(), view)
    });
    settle(cx, |cx| cx.update(|cx| reopened.read(cx).loaded));
    cx.update(|cx| {
        assert_eq!(
            reopened.read(cx).destination,
            Destination::Project(projects[0].id.clone())
        )
    });
    runtime.block_on(async {
        host.db
            .writer()
            .await
            .unwrap()
            .execute("UPDATE projects SET is_archived=1", [])
            .unwrap();
    });
    settle(cx, |cx| {
        cx.update(|cx| {
            reopened.read(cx).destination == Destination::Home
                && reopened.read(cx).projects.is_empty()
        })
    });
    cx.update_window(reopened_window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.press("cmd-9", cx);
        window.press("alt-down", cx);
        assert_eq!(reopened.read(cx).destination, Destination::Home);
        window.remove_window();
    })
    .unwrap();
    runtime.block_on(host.shutdown());
}

fn assert_project_note_navigation(window: &mut Window, cx: &mut App, view: &Entity<Today>) {
    let composer = view.read(cx).composer.clone();
    let draft = composer.read(cx).value().to_string();
    let caret = composer.read(cx).selected_range();
    composer.update(cx, |input, cx| input.set_value("", window, cx));
    let ids: Vec<_> = view.read(cx).model.rows.iter().map(|row| row.id).collect();
    assert!(!ids.is_empty());
    let second = ids.get(1).copied().unwrap_or(ids[0]);
    window.press("alt-j->∆", cx);
    assert_eq!(view.read(cx).model.selected, Some(ids[0]));
    view.update(cx, |this, cx| this.open_selected(window, cx));
    window.press("alt-j->∆", cx);
    assert_eq!(view.read(cx).model.selected, Some(second));
    assert!(view.read(cx).detail_open);
    let content = &view
        .read(cx)
        .model
        .rows
        .iter()
        .find(|row| row.id == second)
        .unwrap()
        .content;
    assert_eq!(view.read(cx).detail.read(cx).value().as_ref(), content);
    window.press("alt-k->˚", cx);
    assert_eq!(view.read(cx).model.selected, Some(ids[0]));
    window.press("alt-k->˚", cx);
    assert_eq!(view.read(cx).model.selected, Some(ids[0]));
    composer.update(cx, |input, cx| {
        input.set_value(draft, window, cx);
        input.set_selected_range(caret, cx);
    });
}
