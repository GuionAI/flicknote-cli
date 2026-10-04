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
        cx.update(|cx| view.read(cx).model.pending.is_empty())
    });
    cx.update(|cx| {
        assert_eq!(composer.read(cx).value(), "new typing");
        assert_eq!(view.read(cx).model.recovery, ["[fixture-fail]"]);
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
            view.read(cx).model.rows.len() == 1 && view.read(cx).model.pending.is_empty()
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
                let bounds = window.find(("note", id)).bounds();
                assert_eq!(bounds.size.width, viewport.size.width);
                assert_eq!(bounds.size.height, px(32.));
                assert_eq!(bounds.origin.x, viewport.origin.x);
                let row = window.within(("note", id));
                assert_eq!(row.find("type-glyph").bounds().size.width, px(17.));
                assert_eq!(row.find("project-dot").bounds().size, size(px(5.), px(5.)));
                view.update(cx, |this, cx| this.close_detail(window, cx));
                window.render_frame(cx);
                // Click beyond short text at the viewport's right edge.
                window.click_at(("note", id), point(bounds.size.width - px(2.), px(16.)), cx);
                assert_eq!(window.find(("note", id)).selected(), Some(true));
                assert_eq!(
                    window.find(("note", id)).bounds().size.width,
                    viewport.size.width
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
                window.render_frame(cx);
                let composer = view.read(cx).composer.clone();
                let closed = window.find("composer-surface").bounds();
                assert_eq!(closed.size.width, px(620.));
                assert_eq!(closed.origin.x + closed.size.width / 2., px(width / 2.));
                assert_eq!(closed.bottom(), px(height - 28.));
                assert_eq!(window.find("navigation-rail").bounds().size.width, px(252.));
                assert_rail_alignment(window);
                assert!(window.try_find("detail-surface").is_none());
                assert_eq!(Theme::global(cx).is_dark(), mode == ThemeMode::Dark);
                assert_eq!(
                    gpui_kit::base::Theme::global(cx).tokens,
                    Theme::global(cx).semantic_tokens()
                );
                assert_readable_rail_roles(Theme::global(cx).color_tokens());
                assert_workspace_theme_roles(mode, cx);
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
                assert_eq!(bounded.bottom(), px(height - 28.));
                assert!(composer.read(cx).focus_handle(cx).is_focused(window));
                // A scrollable tail provides reachability above the bounded composer.
                view.read(cx).list_scroll.scroll_to_bottom();
                window.render_frame(cx);
                let final_row = window.find(("note", 1_u64)).bounds();
                assert!(
                    final_row.bottom() <= bounded.origin.y,
                    "last note must clear composer"
                );
                window.click_at(("note", 1_u64), point(px(4.), px(16.)), cx);
                window.render_frame(cx);
                let detail = window.find("detail-surface").bounds();
                let main = window.find("main-canvas").bounds();
                assert_eq!(detail.right(), main.right() - px(16.));
                assert_eq!(detail.size.width, px(520_f32.min(width - 394.)));
                assert!(detail.origin.x >= main.origin.x + px(48.));
                assert_eq!(detail.bottom(), bounded.origin.y - px(16.));
                assert_detail_tracks_composer(window, cx, &view);
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
                window.click_at("navigation-rail", point(px(100.), px(200.)), cx);
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

fn assert_detail_tracks_composer(window: &mut Window, cx: &mut App, view: &Entity<Today>) {
    let composer = view.read(cx).composer.clone();
    let long_detail = window.find("detail-surface").bounds();
    composer.update(cx, |input, cx| input.set_value("short", window, cx));
    window.render_frame(cx);
    window.render_frame(cx);
    let short_composer = window.find("composer-surface").bounds();
    let short_detail = window.find("detail-surface").bounds();
    assert_eq!(short_detail.bottom(), short_composer.origin.y - px(16.));
    assert!(short_detail.size.height > long_detail.size.height);
    view.update(cx, |this, _| this.error = Some("Synthetic failure".into()));
    window.render_frame(cx);
    let feedback_composer = window.find("composer-surface").bounds();
    let feedback_detail = window.find("detail-surface").bounds();
    assert!(feedback_composer.size.height > short_composer.size.height);
    assert_eq!(
        feedback_detail.bottom(),
        feedback_composer.origin.y - px(16.)
    );
    assert!(feedback_detail.size.height < short_detail.size.height);
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
        assert!(view.read(cx).model.pending.is_empty());
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
}

fn assert_shortcut_input_guards(window: &mut Window, cx: &mut App, view: &Entity<Today>) {
    let composer = view.read(cx).composer.clone();
    composer.update(cx, |input, cx| input.set_value("draft", window, cx));
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
    for key in ["alt-j", "alt-k", "alt-a", "enter"] {
        window.press(key, cx);
    }
    assert_eq!(view.read(cx).model.selected, Some(1));
    assert!(!view.read(cx).detail_open);
    assert!(!view.read(cx).archive_busy);
    assert!(view.read(cx).model.pending.is_empty());
    // Simulate native composition ending before the queued PressEnter callback.
    // Capture-phase composition must still prevent open or submission.
    composer.update(cx, |input, cx| {
        input.unmark_text(window, cx);
        input.set_value("", window, cx);
    });
}

fn assert_workspace_theme_roles(mode: gpui_kit::component::ThemeMode, cx: &App) {
    let theme = Theme::global(cx);
    let pair = |light, dark| gpui_kit::rgb(if mode.is_dark() { dark } else { light }).into();
    let colors = theme.color_tokens();
    assert_eq!(colors.background, pair(0xffffff, 0x0d0d0d));
    assert_eq!(colors.secondary, pair(0xfafafa, 0x141414));
    assert_eq!(colors.surface, pair(0xffffff, 0x1c1c1c));
    assert_eq!(colors.accent, pair(0xf5f5f5, 0x202020));
    assert_eq!(colors.muted, colors.accent);
    assert_eq!(colors.selection, pair(0xebebeb, 0x2b2b2b));
    assert_eq!(colors.border, pair(0xe8e8e8, 0x303030));
    assert_ne!(colors.secondary, colors.surface);
    assert_ne!(colors.muted, colors.selection);
    assert_ne!(colors.accent, colors.selection);
    assert_eq!(colors.foreground, pair(0x171717, 0xededed));
    assert_eq!(colors.secondary_foreground, pair(0x525252, 0xa6a6a6));
    assert_eq!(colors.muted_foreground, pair(0x737373, 0x808080));
    assert_eq!(colors.primary, pair(0x2e2e2e, 0xc6c6c6));
    assert_eq!(colors.primary_foreground, pair(0xe2e2e2, 0x222222));
    assert_eq!(theme.caret, gpui_kit::rgb(0x05c7f7).into());
    assert_eq!(colors.ring, theme.caret);
    assert_eq!(colors.input, colors.border);
    assert_eq!(theme.button, colors.surface);
    assert_eq!(theme.button_hover, colors.accent);
    assert_eq!(theme.button_active, colors.selection);
    assert_eq!(theme.button_foreground, colors.foreground);
}
