use super::*;
use gpui_kit::{Focusable, TestAppContext, point, test::TestWindowExt};
use project_editor::Kind;

fn wheel(w: &mut Window, cx: &mut App, position: gpui_kit::Point<gpui_kit::Pixels>, y: f32) {
    use gpui_kit::InputEvent as _;
    w.dispatch_event(
        gpui_kit::ScrollWheelEvent {
            position,
            delta: gpui_kit::ScrollDelta::Pixels(point(px(0.), px(y))),
            ..Default::default()
        }
        .to_platform_input(),
        cx,
    );
    w.render_frame(cx);
}

fn editor_focus(view: &Entity<Today>, w: &mut Window, cx: &mut App) {
    let editor = view.read(cx).editor.as_ref().unwrap();
    let focus = editor.input.read(cx).focus_handle(cx);
    assert!(focus.is_focused(w), "opening focuses the native input");
    for _ in 0..12 {
        w.press("tab", cx);
        w.render_frame(cx);
        assert!(gpui_kit::base::active_focus_trap(w, cx).is_some());
        assert!(
            !view
                .read(cx)
                .composer
                .read(cx)
                .focus_handle(cx)
                .is_focused(w)
        );
        assert_ne!(w.find("add-project").focused(), Some(true));
    }
    w.focus(&focus, cx);
}

#[gpui_kit::test]
#[allow(clippy::too_many_lines)] // One owned window verifies description geometry and real modal wheel routing.
fn description_editor_wheel_isolates_background(cx: &mut TestAppContext) {
    use flicknote_client::dto::{Patch, ProjectAddInput, ProjectModifyInput};
    cx.executor().allow_parking();
    let root = tempfile::tempdir().unwrap();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let host = append_tests::fixture(&runtime, root.path());
    let services = append_tests::services(&host, &runtime);
    let AppResponse::Project(project) = runtime
        .block_on(host.app.handle(AppRequest::ProjectAdd(ProjectAddInput {
            name: "Wheel fixture".into(),
            color: None,
        })))
        .unwrap()
    else {
        panic!("project")
    };
    runtime.block_on(async {
        let writer = host.db.writer().await.unwrap();
        for id in 3..33 {
            writer.execute("INSERT INTO notes(id,short_id,user_id,content,type,status,project_id,metadata,created_at) VALUES(?,?,'append-owner','Wheel row','normal','ready',?,'{}',strftime('%Y-%m-%dT%H:%M:%SZ','now'))", [format!("00000000-0000-4000-8000-{id:012}"), id.to_string(), project.id.clone()]).unwrap();
        }
    });
    cx.update(|cx| {
        gpui_kit::init(cx);
        install_today_keys(cx);
    });
    let (window, view) = append_tests::open(cx, services.clone());
    tests::settle(cx, |cx| cx.update(|cx| view.read(cx).loaded));
    cx.update_window(window.into(), |_, w, cx| {
        view.update(cx, |v, cx| {
            v.set_destination(Destination::Project(project.id.clone()), w, cx)
        });
    })
    .unwrap();
    tests::settle(cx, |cx| {
        cx.update(|cx| view.read(cx).loaded && view.read(cx).model.rows.len() == 30)
    });
    for description in [
        "",
        "Short description",
        &"Long description line\n".repeat(100),
    ] {
        runtime
            .block_on(
                host.app
                    .handle(AppRequest::ProjectModify(ProjectModifyInput {
                        id: project.id.clone(),
                        color: Patch::Missing,
                        description: if description.is_empty() {
                            Patch::Null
                        } else {
                            Patch::Value(description.into())
                        },
                    })),
            )
            .unwrap();
        tests::settle(cx, |cx| {
            cx.update(|cx| {
                view.read(cx).projects.iter().any(|p| {
                    p.id == project.id
                        && p.description.as_deref().unwrap_or_default() == description
                })
            })
        });
        cx.update_window(window.into(), |_, w, cx| {
            for mode in [
                gpui_kit::component::ThemeMode::Light,
                gpui_kit::component::ThemeMode::Dark,
            ] {
                apply_theme(mode, cx);
                for (width, height) in [(980., 720.), (760., 560.)] {
                    w.resize(size(px(width), px(height)));
                    w.bounds_changed(cx);
                    w.render_frame(cx);
                    let region = w.find("project-description-region").bounds();
                    let header = w.find("destination-header").bounds();
                    let controls = w.find("period-header").bounds();
                    let list = w.find("today-notes").bounds();
                    assert_eq!(region.size.height, px(80.));
                    assert_eq!(region.top(), header.bottom());
                    assert_eq!(controls.top(), region.bottom());
                    assert_eq!(controls.size.height, px(52.));
                    assert_eq!(list.top(), controls.bottom());
                    assert!(list.bottom() <= w.find("composer-surface").bounds().top());
                    w.click("scope-week", cx);
                    w.render_frame(cx);
                    assert_eq!(w.find("project-description-region").bounds(), region);
                    assert_eq!(w.find("today-notes").bounds().top(), list.top());
                    w.click("period-back", cx);
                    w.render_frame(cx);
                    assert_eq!(w.find("today-notes").bounds().top(), list.top());
                    w.click("scope-all", cx);
                    w.render_frame(cx);
                    if description.len() > 100 {
                        let before = w.find("project-description-text").bounds().top();
                        let background = view.read(cx).list_scroll.0.borrow().base_handle.clone();
                        let offset = background.offset();
                        let position = w.find("project-description").bounds().center();
                        wheel(w, cx, position, -40.);
                        assert!(w.find("project-description-text").bounds().top() < before);
                        for delta in [-10000., -80., 80., 10000., 80.] {
                            wheel(w, cx, position, delta);
                            assert_eq!(background.offset(), offset);
                        }
                        assert_eq!(w.find("project-description-text").bounds().top(), before);
                    }
                }
            }
        })
        .unwrap();
    }
    tests::settle(cx, |cx| {
        cx.update(|cx| view.read(cx).loaded && view.read(cx).model.rows.len() == 30)
    });
    cx.update_window(window.into(), |_, w, cx| {
        view.update(cx, |v, cx| v.select(32, w, cx));
        for mode in [
            gpui_kit::component::ThemeMode::Light,
            gpui_kit::component::ThemeMode::Dark,
        ] {
            apply_theme(mode, cx);
            for (width, height) in [(980., 720.), (760., 560.)] {
                w.resize(size(px(width), px(height)));
                w.bounds_changed(cx);
                w.render_frame(cx);
                let description = w.find("project-description-region").bounds();
                let list = w.find("today-notes").bounds();
                let reader = w.find("detail-surface").bounds();
                assert_eq!(description.size.height, px(80.));
                assert!(description.right() <= reader.left());
                assert!(list.right() <= reader.left());
                assert!(list.bottom() <= w.find("composer-surface").bounds().top());
            }
        }
        view.update(cx, |v, cx| v.close_detail(w, cx));
    })
    .unwrap();
    cx.update_window(window.into(), |_, w, cx| {
        w.resize(size(px(980.), px(720.)));
        w.bounds_changed(cx);
        w.render_frame(cx);
        let list = view.read(cx).list_scroll.0.borrow().base_handle.clone();
        assert!(list.max_offset().y > px(0.));
        let background_before = list.offset();
        let description_position = w.find("project-description").bounds().center();
        let text_top = w.find("project-description-text").bounds().top();
        wheel(w, cx, description_position, -80.);
        assert!(w.find("project-description-text").bounds().top() < text_top);
        for delta in [-10000., -80., 80., 10000., 80.] {
            wheel(w, cx, description_position, delta);
            assert_eq!(list.offset(), background_before);
        }
        assert_eq!(w.find("project-description-text").bounds().top(), text_top);
        view.update(cx, |v, cx| {
            v.edit(Kind::Description(project.id.clone()), w, cx);
            v.editor.as_ref().unwrap().input.update(cx, |i, cx| {
                i.set_value("Long description line\n".repeat(100), w, cx);
                i.set_selected_range(0..0, cx);
            });
        });
        w.render_frame(cx);
        let input = view.read(cx).editor.as_ref().unwrap().input.clone();
        editor_focus(&view, w, cx);
        let position = input.read(cx).text_bounds().unwrap().center();
        let editor_before = input.read(cx).scroll_offset();
        wheel(w, cx, position, -80.);
        assert!(input.read(cx).scroll_offset().y < editor_before.y);
        wheel(w, cx, position, 40.);
        assert_eq!(input.read(cx).scroll_offset().y, px(-40.));
        for _ in 0..40 {
            wheel(w, cx, position, -80.);
        }
        let editor_after = input.read(cx).scroll_offset();
        assert!(editor_after.y < editor_before.y, "inner editor must scroll");
        assert_eq!(
            list.offset(),
            background_before,
            "wheel over editor moved background list"
        );
        for delta in [80., 10000., 80., -10000., -80.] {
            wheel(w, cx, position, delta);
            assert_eq!(
                list.offset(),
                background_before,
                "editor bounds isolate background"
            );
        }
        let pane = w.find("editor-pane").bounds();
        let backdrop = w.find("dialog-0").bounds();
        for position in [
            point(pane.left() + px(4.), pane.top() + px(4.)),
            point(backdrop.right() - px(8.), backdrop.center().y),
        ] {
            for delta in [-80., 80.] {
                wheel(w, cx, position, delta);
                assert_eq!(
                    list.offset(),
                    background_before,
                    "whole backdrop isolates background"
                );
            }
        }
    })
    .unwrap();
    // Kit's standard textarea still owns selection, exact copy and composing dismissal.
    cx.update_window(window.into(), |_, w, cx| {
        let input = view.read(cx).editor.as_ref().unwrap().input.clone();
        let list = view.read(cx).list_scroll.0.borrow().base_handle.clone();
        let background_before = list.offset();
        input.update(cx, |i, cx| {
            i.set_value("copy  exact", w, cx);
            i.set_selected_range(0..11, cx);
        });
        w.render_frame(cx);
        w.press("cmd-c", cx);
        assert_eq!(
            cx.read_from_clipboard().and_then(|c| c.text()),
            Some("copy  exact".into())
        );
        let text = input.read(cx).text_bounds().unwrap();
        let start = point(text.left() + px(2.), text.top() + px(8.));
        let end = point(text.left() + px(45.), start.y);
        markdown_tests::drag_text(w, cx, start, end);
        let released = input.read(cx).selected_range();
        assert!(!released.is_empty());
        use gpui_kit::InputEvent as _;
        w.dispatch_event(
            gpui_kit::MouseMoveEvent {
                position: point(end.x + px(40.), end.y),
                pressed_button: None,
                modifiers: Default::default(),
            }
            .to_platform_input(),
            cx,
        );
        w.render_frame(cx);
        assert_eq!(
            input.read(cx).selected_range(),
            released,
            "release freezes textarea selection"
        );
        w.press("cmd-c", cx);
        assert_eq!(
            cx.read_from_clipboard().and_then(|c| c.text()),
            Some("copy  exact"[released].into())
        );
        input.update(cx, |i, cx| {
            i.replace_and_mark_text_in_range(None, "ni", Some(2..2), w, cx)
        });
        w.render_frame(cx);
        w.press("escape", cx);
        assert!(view.read(cx).editor.is_some());
        input.update(cx, |i, cx| {
            i.replace_and_mark_text_in_range(None, "ni", Some(2..2), w, cx)
        });
        w.dispatch_action(Box::new(gpui_kit::base::actions::Cancel), cx);
        assert!(view.read(cx).editor.is_some());
        w.click("cancel-editor", cx);
        assert!(view.read(cx).editor.is_some());
        input.update(cx, |i, cx| i.unmark_text(w, cx));
        w.press("tab", cx);
        w.press("escape", cx);
        w.render_frame(cx);
        assert!(view.read(cx).editor.is_none());
        assert!(
            view.read(cx)
                .composer
                .read(cx)
                .focus_handle(cx)
                .is_focused(w)
        );
        wheel(w, cx, w.find("today-notes").bounds().center(), -80.);
        assert!(
            list.offset().y < background_before.y,
            "list wheel works after dismissal"
        );
        view.update(cx, |v, cx| v.set_destination(Destination::Home, w, cx));
        w.render_frame(cx);
        assert!(w.try_find("project-description-region").is_none());
        assert_eq!(
            w.find("period-header").bounds().top(),
            w.find("destination-header").bounds().bottom()
        );
        w.remove_window();
    })
    .unwrap();
    runtime
        .block_on(host.run_until(async { Ok(()) }, || services.cancel_operations()))
        .unwrap();
}
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
        w.render_frame(cx);
        editor_focus(&view, w, cx);
        view.update(cx, |v, _| v.editor.as_mut().unwrap().busy = true);
        w.render_frame(cx);
        w.press("escape", cx);
        w.press("cmd-enter", cx);
        w.click("cancel-editor", cx);
        assert!(view.read(cx).editor.as_ref().unwrap().busy);
        view.update(cx, |v, cx| {
            v.editor.as_mut().unwrap().busy = false;
            cx.notify();
        });
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
            v.edit(Kind::Description(pid.clone()), w, cx);
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
                .any(|p| p.id == pid && p.description.as_deref() == Some("Rust\nSystems"))
                && view.read(cx).editor.is_none()
        })
    });
    cx.update_window(window.into(), |_, w, cx| {
        view.update(cx, |v, cx| {
            v.edit(Kind::Description(pid.clone()), w, cx);
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
                .any(|p| p.id == pid && p.description.as_deref() == Some("Rust\nSystems"))
        )
    });
    cx.update_window(window.into(), |_, w, cx| {
        view.update(cx, |v, cx| {
            v.edit(Kind::Description(pid.clone()), w, cx);
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
                .any(|p| p.id == pid && p.description.is_none())
                && view.read(cx).editor.is_none()
        })
    });
    // Composition in the description owns Return/Escape; a deleted target produces recoverable save text.
    cx.update_window(window.into(), |_, w, cx| {
        view.update(cx, |v, cx| v.edit(Kind::Description(pid.clone()), w, cx));
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
                i.set_value("recovery description", w, cx);
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
            "recovery description"
        )
    });
    cx.update_window(window.into(), |_, w, _cx| w.remove_window())
        .unwrap();
    services.cancel_operations();
    runtime.block_on(host.run_until(async { Ok(()) })).unwrap();
}
fn window_save(view: &Entity<Today>, w: &mut Window, cx: &mut App) {
    view.update(cx, |v, cx| v.save_editor(w, cx));
}
