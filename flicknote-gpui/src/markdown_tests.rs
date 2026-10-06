//! Owned host/rendered evidence, without OS windows, personal profiles or URL opening.
#![allow(clippy::print_stderr)] // Owned timing evidence, never personal content.
use super::tests::settle;
use super::*;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{ClipboardItem, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, point};
use gpui_kit::{Focusable as _, InputEvent as _};

const MARKDOWN: &str = "# 中文 Heading\n\n**bold** and *emphasis* with `inline`\n\n> quotation\n\n- first\n- second\n\n| Column | Value | Third | Fourth | Fifth | Sixth |\n| --- | --- | --- | --- | --- | --- |\n| 中文 | English | very-wide-content | very-wide-content | very-wide-content | very-wide-content |\n\n```rust\nlet x = 1;\n```\n\n[link](https://example.invalid)\n\n$E=mc^2$\n\n```mermaid\ngraph TD; A-->B\n```\n";

#[gpui_kit::test]
#[allow(clippy::too_many_lines)] // One owned window checks state across real watched changes.
fn markdown_watch_reading_selection_identity_and_layout(cx: &mut gpui_kit::TestAppContext) {
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
    let services = Arc::new(Services {
        app: host.app.clone(),
        db: host.db.clone(),
        runtime: runtime.handle().clone(),
        operations: Mutex::default(),
        user_id: flicknote_sync::spike::USER.into(),
        real_account: false,
        first_sync: Mutex::default(),
        destination: Mutex::default(),
        capture: Arc::default(),
        draft: Mutex::default(),
        organization: Mutex::default(),
        capture_changed: tokio::sync::watch::channel(()).0,
    });
    let write = |sql: &str, values: &[&str]| {
        runtime.block_on(async {
            let writer = host.db.writer().await.unwrap();
            match values {
                [] => writer.execute(sql, []).unwrap(),
                [a] => writer.execute(sql, [a]).unwrap(),
                [a, b] => writer.execute(sql, [a, b]).unwrap(),
                _ => panic!("bounded owned fixture parameters"),
            };
        })
    };
    write(
        "UPDATE notes SET content=?,title=? WHERE short_id=2",
        &[
            MARKDOWN,
            "Real title 中文 wraps across the narrow reading pane",
        ],
    );
    write(
        "UPDATE projects SET is_archived=1 WHERE id='fixture-work'",
        &[],
    );
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
    cx.update(|cx| {
        assert!(
            view.read(cx).detail.is_none(),
            "closed list never parses documents"
        )
    });
    cx.update_window(window.into(), |_, window, cx| {
        view.update(cx, |this, cx| this.select(2, window, cx))
    })
    .unwrap();
    settle(cx, |cx| {
        cx.update(|cx| {
            view.read(cx)
                .detail
                .as_ref()
                .unwrap()
                .state
                .read(cx)
                .rendered_text()
                .as_str()
                .contains("graph TD")
        })
    });
    for mode in [
        gpui_kit::component::ThemeMode::Light,
        gpui_kit::component::ThemeMode::Dark,
    ] {
        cx.update_window(window.into(), |_, window, cx| {
            apply_theme(mode, cx);
            view.update(cx, |this, cx| this.select(2, window, cx));
            window.render_frame(cx);
            assert!(window.find("detail-title").bounds().size.height > px(24.));
            assert!(window.try_find("detail-project").is_some());
            assert!(
                !view
                    .read(cx)
                    .projects
                    .iter()
                    .any(|p| p.id == "fixture-work")
            );
            let pane = window.find("detail-surface").bounds();
            assert_eq!(pane.size.width, px(272.));
            assert!(window.find("detail-id").bounds().bottom() <= px(44.));
            for action in ["copy-detail", "archive", "close-detail"] {
                assert!(window.find(action).bounds().right() <= pane.right());
            }
            let state = view.read(cx).detail.as_ref().unwrap().state.clone();
            eprintln!(
                "Markdown rendered bounds {:?}; scroll max {:?}",
                state.read(cx).bounds(),
                view.read(cx).detail.as_ref().unwrap().scroll.max_offset()
            );
            assert!(state.read(cx).bounds().size.height > px(0.));
            assert!(view.read(cx).detail.as_ref().unwrap().scroll.max_offset().y > px(0.));
            gpui_kit::base::TextSelection::clear(window, cx);
            window.focus(&state.read(cx).focus_handle().clone(), cx);
            window.render_frame(cx);
            window.press("cmd-a", cx);
            window.press("cmd-c", cx);
            let text = cx.read_from_clipboard().unwrap().text().unwrap();
            for rendered in [
                "中文 Heading",
                "bold",
                "quotation",
                "English",
                "let x = 1;",
                "$E=mc^2$",
                "graph TD; A-->B",
            ] {
                assert!(text.contains(rendered), "{text:?}");
            }
            assert!(!text.contains("**bold**"));
            assert!(!text.contains("```rust"));
            window.click("copy-detail", cx);
            assert_eq!(cx.read_from_clipboard().unwrap().text().unwrap(), MARKDOWN);
            // Actual pointer selection of the first heading, followed by native copy dispatch.
            state.update(cx, gpui_kit::base::TextViewState::clear_selection);
            let origin = state.read(cx).bounds().origin;
            drag_text(
                window,
                cx,
                origin + point(px(2.), px(10.)),
                origin + point(px(90.), px(10.)),
            );
            window.press("cmd-c", cx);
            let selected = state.read(cx).selected_text();
            assert!(!selected.is_empty());
            assert!(!selected.contains('#'));
            assert_eq!(cx.read_from_clipboard().unwrap().text().unwrap(), selected);
            window.resize(size(px(980.), px(720.)));
            window.bounds_changed(cx);
            window.render_frame(cx);
            let pane = window.find("detail-surface").bounds();
            assert_eq!(pane.size.width, px(crate::workspace::reading_width(980.)));
            assert!(state.read(cx).bounds().right() <= pane.right());
            assert!(window.find("close-detail").bounds().right() <= pane.right());
            window.resize(size(px(760.), px(560.)));
            window.bounds_changed(cx);
            window.render_frame(cx);
        })
        .unwrap();
    }
    let (state, revision, selection) = cx
        .update_window(window.into(), |_, window, cx| {
            view.read(cx)
                .detail
                .as_ref()
                .unwrap()
                .scroll
                .set_offset(point(px(0.), px(-100.)));
            window.render_frame(cx);
            let state = view.read(cx).detail.as_ref().unwrap().state.clone();
            let origin = state.read(cx).bounds().origin;
            drag_text(
                window,
                cx,
                origin + point(px(2.), px(10.)),
                origin + point(px(90.), px(10.)),
            );
            assert!(!state.read(cx).selected_text().is_empty());
            let reading = view.read(cx).detail.as_ref().unwrap();
            (
                reading.state.clone(),
                reading.state.read(cx).rendered_text(),
                reading.state.read(cx).selected_text(),
            )
        })
        .unwrap();
    write(
        "UPDATE notes SET title='Metadata only' WHERE short_id=2",
        &[],
    );
    settle(cx, |cx| {
        cx.update(|cx| view.read(cx).model.rows[0].title.as_deref() == Some("Metadata only"))
    });
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        let reading = view.read(cx).detail.as_ref().unwrap();
        assert_eq!(reading.state, state);
        assert_eq!(reading.state.read(cx).rendered_text(), revision);
        assert_eq!(reading.state.read(cx).selected_text(), selection);
        assert_eq!(reading.scroll.offset().y, px(-100.));
        assert!(state.read(cx).focus_handle().is_focused(window));
    })
    .unwrap();
    let long = MARKDOWN.repeat(160);
    let started = Instant::now();
    write("UPDATE notes SET content=? WHERE short_id=2", &[&long]);
    settle(cx, |cx| {
        cx.update(|cx| view.read(cx).detail.as_ref().unwrap().source == long)
    });
    // Switch while the large update can still be parsing; late old results stay in old entity.
    cx.update_window(window.into(), |_, window, cx| {
        view.update(cx, |this, cx| this.select(1, window, cx))
    })
    .unwrap();
    settle(cx, |cx| {
        cx.update(|cx| {
            !view
                .read(cx)
                .detail
                .as_ref()
                .unwrap()
                .state
                .read(cx)
                .rendered_text()
                .is_empty()
        })
    });
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        let reading = view.read(cx).detail.as_ref().unwrap();
        assert_ne!(reading.state, state);
        assert_eq!(reading.scroll.offset(), point(px(0.), px(0.)));
        assert!(reading.state.read(cx).selected_text().is_empty());
        assert!(
            !reading
                .state
                .read(cx)
                .rendered_text()
                .as_str()
                .contains("中文 Heading")
        );
        view.update(cx, |this, cx| this.select(2, window, cx));
    })
    .unwrap();
    settle(cx, |cx| {
        cx.update(|cx| {
            view.read(cx)
                .detail
                .as_ref()
                .unwrap()
                .state
                .read(cx)
                .rendered_text()
                .as_str()
                .contains("graph TD")
        })
    });
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        let state = view.read(cx).detail.as_ref().unwrap().state.clone();
        gpui_kit::base::TextSelection::clear(window, cx);
        window.focus(&state.read(cx).focus_handle().clone(), cx);
        window.render_frame(cx);
        window.press("cmd-a", cx);
        window.press("cmd-c", cx);
        let text = cx.read_from_clipboard().unwrap().text().unwrap();
        assert_eq!(text.matches("graph TD; A-->B").count(), 160);
        window.click("copy-detail", cx);
        assert_eq!(cx.read_from_clipboard().unwrap().text().unwrap(), long);
    })
    .unwrap();
    eprintln!(
        "Markdown owned long-document open/watch/switch/reopen: {} bytes, {:.3}ms",
        long.len(),
        started.elapsed().as_secs_f64() * 1000.
    );
    let code = "```rust\n    first();  \n\tsecond(); \n```\n";
    write(
        "UPDATE notes SET content=?,title='' WHERE short_id=2",
        &[code],
    );
    settle(cx, |cx| {
        cx.update(|cx| {
            view.read(cx)
                .detail
                .as_ref()
                .unwrap()
                .state
                .read(cx)
                .rendered_text()
                .as_str()
                .contains("first();")
        })
    });
    cx.update_window(window.into(), |_, window, cx| {
        let state = view.read(cx).detail.as_ref().unwrap().state.clone();
        view.read(cx)
            .detail
            .as_ref()
            .unwrap()
            .scroll
            .set_offset(point(px(0.), px(0.)));
        gpui_kit::base::TextSelection::clear(window, cx);
        window.focus(&state.read(cx).focus_handle().clone(), cx);
        window.render_frame(cx);
        window.press("cmd-a", cx);
        let selected = state.read(cx).selected_text();
        assert!(
            selected.starts_with("    first();  \n\tsecond(); "),
            "{selected:?}"
        );
        assert_ne!(selected.as_str(), selected.trim());
        window.press("cmd-c", cx);
        assert_eq!(cx.read_from_clipboard().unwrap().text().unwrap(), selected);
        window.click("copy-detail", cx);
        assert_eq!(cx.read_from_clipboard().unwrap().text().unwrap(), code);
        state.update(cx, gpui_kit::base::TextViewState::clear_selection);
        gpui_kit::base::TextSelection::clear(window, cx);
        let origin = state.read(cx).bounds().origin;
        drag_text(
            window,
            cx,
            origin + point(px(13.), px(44.)),
            origin + point(px(35.), px(44.)),
        );
        let indentation = state.read(cx).selected_text();
        assert!(!indentation.is_empty(), "{indentation:?}");
        assert!(
            indentation.chars().all(char::is_whitespace),
            "{indentation:?}"
        );
        cx.write_to_clipboard(ClipboardItem::new_string("indentation sentinel".into()));
        window.press("cmd-c", cx);
        assert_eq!(
            cx.read_from_clipboard().unwrap().text().unwrap(),
            indentation
        );
        state.update(cx, gpui_kit::base::TextViewState::clear_selection);
        gpui_kit::base::TextSelection::clear(window, cx);
        window.focus(&state.read(cx).focus_handle().clone(), cx);
        cx.write_to_clipboard(ClipboardItem::new_string("no selection".into()));
        window.press("cmd-c", cx);
        assert_eq!(
            cx.read_from_clipboard().unwrap().text().unwrap(),
            "no selection"
        );
    })
    .unwrap();
    // Real host changes, two block-scoped actions, note switch and body update.
    for (id, first, second) in [
        ("2", "    first();  \n", "\tsecond(); \n"),
        ("1", "  switched(); \n", "\tother();  \n"),
        ("1", "    updated();  \n", "\tupdated_other(); \n"),
    ] {
        let second = format!("{second}    {}  \n", "wide_code_".repeat(12));
        let source = format!("```rust\n{first}\n```\n\n```text\n{second}\n```\n");
        write(
            "UPDATE notes SET content=?,title='' WHERE short_id=?",
            &[&source, id],
        );
        settle(cx, |cx| {
            cx.update(|cx| {
                view.read(cx)
                    .model
                    .rows
                    .iter()
                    .any(|r| r.id.to_string() == id && r.content == source)
            })
        });
        cx.update_window(window.into(), |_, window, cx| {
            view.update(cx, |this, cx| this.select(id.parse().unwrap(), window, cx))
        })
        .unwrap();
        settle(cx, |cx| {
            cx.update(|cx| {
                view.read(cx)
                    .detail
                    .as_ref()
                    .unwrap()
                    .state
                    .read(cx)
                    .rendered_text()
                    .as_str()
                    .contains(first.trim())
            })
        });
        for mode in [
            gpui_kit::component::ThemeMode::Light,
            gpui_kit::component::ThemeMode::Dark,
        ] {
            cx.update_window(window.into(), |_, window, cx| {
                apply_theme(mode, cx);
                view.read(cx)
                    .detail
                    .as_ref()
                    .unwrap()
                    .scroll
                    .set_offset(point(px(0.), px(0.)));
                window.render_frame(cx);
                let pane = window.find("detail-surface").bounds();
                let state = view.read(cx).detail.as_ref().unwrap().state.clone();
                window.focus(&state.read(cx).focus_handle().clone(), cx);
                for _ in 0..12 {
                    window.press("tab", cx);
                    window.render_frame(cx);
                    if window.find("copy-code-0").focused() == Some(true) {
                        break;
                    }
                }
                assert_eq!(window.find("copy-code-0").focused(), Some(true));
                window.press("enter", cx);
                assert_eq!(cx.read_from_clipboard().unwrap().text().unwrap(), first);
                for (start, expected) in [
                    (0, first),
                    (source.find("```text").unwrap(), second.as_str()),
                ] {
                    let selector = format!("copy-code-{start}");
                    let bounds = window
                        .find(gpui_kit::SharedString::from(selector.clone()))
                        .bounds();
                    assert!(bounds.right() <= pane.right());
                    assert!(bounds.bottom() <= px(560.));
                    window.click(gpui_kit::SharedString::from(selector), cx);
                    assert_eq!(cx.read_from_clipboard().unwrap().text().unwrap(), expected);
                }
                window.click("copy-detail", cx);
                assert_eq!(cx.read_from_clipboard().unwrap().text().unwrap(), source);
                window.click("copy-code-0", cx);
                window.press("escape", cx);
                assert!(view.read(cx).detail.is_none());
                assert!(
                    view.read(cx)
                        .composer
                        .read(cx)
                        .focus_handle(cx)
                        .is_focused(window)
                );
                view.update(cx, |this, cx| this.select(id.parse().unwrap(), window, cx));
            })
            .unwrap();
            settle(cx, |cx| {
                cx.update(|cx| {
                    view.read(cx)
                        .detail
                        .as_ref()
                        .unwrap()
                        .state
                        .read(cx)
                        .rendered_text()
                        .as_str()
                        .contains(first.trim())
                })
            });
        }
    }
    cx.update_window(window.into(), |_, window, cx| {
        view.update(cx, |this, cx| this.select(2, window, cx))
    })
    .unwrap();
    write(
        "UPDATE notes SET content='',title='  ',project_id=NULL WHERE short_id=2",
        &[],
    );
    settle(cx, |cx| {
        cx.update(|cx| view.read(cx).detail.as_ref().unwrap().source.is_empty())
    });
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("detail-title").is_none());
        assert!(window.try_find("detail-project").is_none());
        window.click("copy-detail", cx);
        assert_eq!(
            cx.read_from_clipboard().unwrap().text().unwrap_or_default(),
            ""
        );
        let old = view.read(cx).detail.as_ref().unwrap().state.clone();
        window.click("close-detail", cx);
        assert!(view.read(cx).detail.is_none());
        view.update(cx, |this, cx| this.select(2, window, cx));
        assert_ne!(view.read(cx).detail.as_ref().unwrap().state, old);
        cx.write_to_clipboard(ClipboardItem::new_string("owned cleanup".into()));
        window.remove_window();
    })
    .unwrap();
    runtime.block_on(host.shutdown());
}

fn drag_text(
    window: &mut Window,
    cx: &mut App,
    start: gpui_kit::Point<gpui_kit::Pixels>,
    end: gpui_kit::Point<gpui_kit::Pixels>,
) {
    window.dispatch_event(
        MouseDownEvent {
            button: MouseButton::Left,
            position: start,
            modifiers: Default::default(),
            click_count: 1,
            first_mouse: false,
        }
        .to_platform_input(),
        cx,
    );
    window.render_frame(cx);
    window.dispatch_event(
        MouseMoveEvent {
            position: end,
            pressed_button: Some(MouseButton::Left),
            modifiers: Default::default(),
        }
        .to_platform_input(),
        cx,
    );
    window.render_frame(cx);
    window.dispatch_event(
        MouseUpEvent {
            button: MouseButton::Left,
            position: end,
            modifiers: Default::default(),
            click_count: 1,
        }
        .to_platform_input(),
        cx,
    );
    window.render_frame(cx);
}
