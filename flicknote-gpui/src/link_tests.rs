//! Owned LocalHost/rendered copy and source evidence; no native pasteboard or URL opening.
use super::tests::settle;
use super::*;
use gpui_kit::{Focusable as _, TestAppContext, test::TestWindowExt};

const URL: &str = "https://example.invalid/articles/a-long-original-source?query=1#part";

fn toast(window: &mut Window, cx: &mut App, visible: bool) {
    window.render_frame(cx);
    assert_eq!(window.try_find("copy-toast").is_some(), visible);
    if visible {
        assert_eq!(window.find("copy-toast").role(), Some(Role::Alert));
    }
}

#[gpui_kit::test]
#[allow(clippy::too_many_lines)] // One production-host journey checks creation, append and watched source states.
fn link_enter_source_processing_search_archive_and_geometry(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let root = tempfile::tempdir().unwrap();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let (host, _) = note_action_tests::fixture(&runtime, root.path());
    let services = append_tests::services(&host, &runtime);
    cx.update(|cx| {
        gpui_kit::init(cx);
        install_today_keys(cx);
    });
    let (window, view) = append_tests::open(cx, services.clone());
    settle(cx, |cx| cx.update(|cx| view.read(cx).loaded));
    let composer = cx.update(|cx| view.read(cx).composer.clone());
    for (input, kind) in [
        (format!(" \n{URL}\t"), "link"),
        (format!("{URL} with prose"), "normal"),
        (format!("{URL}\nhttps://second.invalid"), "normal"),
        ("example.invalid/page".into(), "normal"),
        ("HTTPS://example.invalid/page".into(), "normal"),
    ] {
        cx.update_window(window.into(), |_, w, cx| {
            composer.update(cx, |input_state, cx| {
                input_state.set_value(input.clone(), w, cx);
                input_state.focus(w, cx);
            });
            w.render_frame(cx);
            w.press("enter", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update(|cx| assert!(composer.read(cx).value().is_empty()));
        settle(cx, |cx| {
            cx.update(|cx| {
                view.read(cx).model.rows.iter().any(|r| r.id == 5)
                    && view.read(cx).model.capture().pending.is_empty()
            })
        });
        let (stored_kind, status, body, metadata, assigned) = runtime.block_on(async {
            host.db
                .reader()
                .await
                .unwrap()
                .query_row(
                    "SELECT type,status,content,metadata,project_id FROM notes WHERE short_id=5",
                    [],
                    |r| {
                        Ok((
                            r.get::<_, String>(0)?,
                            r.get::<_, String>(1)?,
                            r.get::<_, Option<String>>(2)?,
                            r.get::<_, Option<String>>(3)?,
                            r.get::<_, Option<String>>(4)?,
                        ))
                    },
                )
                .unwrap()
        });
        assert_eq!(stored_kind, kind);
        let metadata: serde_json::Value =
            serde_json::from_str(metadata.as_deref().unwrap_or("null")).unwrap();
        assert!(metadata.get("created_by_ai").is_none());
        assert!(assigned.is_none());
        if kind == "link" {
            assert_eq!(status, "source_queued");
            assert!(body.is_none());
            assert_eq!(metadata["link"]["url"], URL);
            cx.update_window(window.into(), |_, w, cx| {
                view.update(cx, |this, cx| this.select(5, w, cx));
                w.render_frame(cx);
                assert_eq!(view.read(cx).model.rows[0].preview, URL);
                assert_eq!(view.read(cx).detail.as_ref().unwrap().source, "");
                assert_eq!(w.find("source-bar").bounds().size.height, px(52.));
                w.click("copy-source", cx);
                assert_eq!(cx.read_from_clipboard().unwrap().text().unwrap(), URL);
                toast(w, cx, true);
                composer.update(cx, |i, cx| {
                    i.set_value("https://append.invalid", w, cx);
                    i.focus(w, cx);
                });
                w.press("enter", cx);
            })
            .unwrap();
            cx.run_until_parked();
            settle(cx, |cx| {
                cx.update(|cx| view.read(cx).model.capture().appends.pending.is_empty())
            });
            runtime.block_on(async {
                let reader = host.db.reader().await.unwrap();
                let values: (String, String, String) = reader
                    .query_row(
                        "SELECT type,status,content FROM notes WHERE short_id=5",
                        [],
                        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                    )
                    .unwrap();
                assert_eq!(
                    values,
                    (
                        "link".into(),
                        "source_queued".into(),
                        "https://append.invalid".into()
                    )
                );
            });
            cx.update_window(window.into(), |_, w, cx| {
                view.update(cx, |this, cx| this.close_detail(w, cx))
            })
            .unwrap();
        } else {
            assert_eq!(status, "ai_queued");
            assert_eq!(body.as_deref(), Some(input.as_str()));
        }
        runtime.block_on(async {
            host.db
                .writer()
                .await
                .unwrap()
                .execute("DELETE FROM notes WHERE short_id=5", [])
                .unwrap();
        });
        settle(cx, |cx| {
            cx.update(|cx| !view.read(cx).model.rows.iter().any(|r| r.id == 5))
        });
    }
    // Every processing and collection context uses the same source, independently of body.
    runtime.block_on(async {
        host.db
            .writer()
            .await
            .unwrap()
            .execute(
                "UPDATE notes SET type='link',content='',title=NULL,metadata=? WHERE short_id=4",
                [serde_json::json!({"link":{"url":URL}}).to_string()],
            )
            .unwrap();
    });
    for (status, title, body) in [
        ("source_queued", "", ""),
        ("source_failed", "Saved page", ""),
        ("ready", "Processed title", "Canonical body"),
    ] {
        runtime.block_on(async {
            host.db
                .writer()
                .await
                .unwrap()
                .execute(
                    "UPDATE notes SET status=?,title=?,content=? WHERE short_id=4",
                    [status, title, body],
                )
                .unwrap();
        });
        settle(cx, |cx| {
            cx.update(|cx| {
                view.read(cx)
                    .model
                    .rows
                    .iter()
                    .any(|r| r.id == 4 && r.content == body && r.title.as_deref() == Some(title))
            })
        });
        for mode in [
            gpui_kit::component::ThemeMode::Light,
            gpui_kit::component::ThemeMode::Dark,
        ] {
            cx.update_window(window.into(), |_, w, cx| {
                apply_theme(mode, cx);
                w.resize(size(px(760.), px(560.)));
                view.update(cx, |this, cx| this.select(4, w, cx));
                w.render_frame(cx);
                let row = &view.read(cx).model.rows[0];
                assert_eq!(row.source_url.as_deref(), Some(URL));
                assert_eq!(
                    row.preview,
                    if !body.is_empty() {
                        body
                    } else if !title.is_empty() {
                        title
                    } else {
                        URL
                    }
                );
                let bar = w.find("source-bar").bounds();
                let pane = w.find("detail-surface").bounds();
                assert!(bar.right() <= pane.right());
                for action in ["open-source", "copy-source"] {
                    let b = w.find(action).bounds();
                    assert!(b.size.width > px(20.) && b.right() <= bar.right());
                }
                assert!(w.find("detail-project").bounds().bottom() <= bar.top());
                w.click("copy-detail", cx);
                assert_eq!(
                    cx.read_from_clipboard()
                        .and_then(|item| item.text())
                        .unwrap_or_default(),
                    body
                );
                toast(w, cx, true);
            })
            .unwrap();
        }
    }
    runtime.block_on(async {
        let writer=host.db.writer().await.unwrap();
        writer.execute("UPDATE notes SET status='source_failed' WHERE short_id=4",[]).unwrap();
        writer.execute("INSERT INTO note_shares(id,user_id,token,created_at) SELECT id,user_id,'owned-source-share','2026-01-01T00:00:00Z' FROM notes WHERE short_id=4",[]).unwrap();
    });
    for destination in [Destination::Failed, Destination::Shared] {
        cx.update_window(window.into(), |_, w, cx| {
            view.update(cx, |this, cx| {
                this.change_destination(destination.clone(), w, cx)
            })
        })
        .unwrap();
        settle(cx, |cx| {
            cx.update(|cx| {
                view.read(cx).loaded && view.read(cx).model.rows.iter().any(|r| r.id == 4)
            })
        });
        cx.update_window(window.into(), |_, w, cx| {
            view.update(cx, |this, cx| this.select(4, w, cx));
            w.render_frame(cx);
            assert!(w.try_find("source-bar").is_some());
            w.click("copy-source", cx);
            toast(w, cx, true);
        })
        .unwrap();
    }
    cx.update_window(window.into(), |_, w, cx| {
        view.read(cx)
            .search_input
            .clone()
            .update(cx, |input, cx| input.set_value("#4", w, cx));
    })
    .unwrap();
    settle(cx, |cx| {
        cx.update(|cx| {
            view.read(cx).search.active()
                && !view.read(cx).search.loading
                && view.read(cx).model.rows.iter().any(|r| r.id == 4)
        })
    });
    cx.update_window(window.into(), |_, w, cx| {
        view.update(cx, |this, cx| this.select(4, w, cx))
    })
    .unwrap();
    settle(cx, |cx| {
        cx.update(|cx| view.read(cx).search.detail == Some(4))
    });
    cx.update_window(window.into(), |_, w, cx| {
        w.render_frame(cx);
        assert!(w.try_find("source-bar").is_some());
        w.click("copy-source", cx);
        toast(w, cx, true);
        assert_eq!(
            view.read(cx).detail.as_ref().unwrap().source,
            "Canonical body"
        );
    })
    .unwrap();
    let results = runtime
        .block_on(flicknote_sync::workspace_search::read(
            &host.app,
            &host.db,
            "append-owner",
            "#4",
            false,
            Some(4),
        ))
        .unwrap();
    assert_eq!(results.rows[0].source_url.as_deref(), Some(URL));
    assert_eq!(results.rows[0].content, "Canonical body");
    runtime.block_on(async {
        let writer = host.db.writer().await.unwrap();
        writer
            .execute(
                "UPDATE notes SET deleted_at='2026-01-01T00:00:00Z' WHERE short_id=4",
                [],
            )
            .unwrap();
    });
    cx.update_window(window.into(), |_, w, cx| {
        view.update(cx, |this, cx| {
            this.change_destination(Destination::Archive, w, cx)
        })
    })
    .unwrap();
    settle(cx, |cx| {
        cx.update(|cx| {
            view.read(cx)
                .model
                .rows
                .iter()
                .any(|r| r.id == 4 && r.archived)
        })
    });
    cx.update_window(window.into(), |_, w, cx| {
        view.update(cx, |this, cx| this.select(4, w, cx));
        w.render_frame(cx);
        w.click("copy-source", cx);
        toast(w, cx, true);
        assert_eq!(cx.read_from_clipboard().unwrap().text().unwrap(), URL);
        assert!(view.read(cx).model.capture().appends.pending.is_empty());
    })
    .unwrap();
    for metadata in [
        serde_json::json!({"link":{"url":false}}),
        serde_json::json!({"link":{"url":"javascript:alert(1)"}}),
        serde_json::json!({"link":{"url":"https:///missing-host"}}),
    ] {
        let expected = metadata["link"]["url"].as_str().map(str::to_owned);
        runtime.block_on(async {
            host.db
                .writer()
                .await
                .unwrap()
                .execute(
                    "UPDATE notes SET metadata=? WHERE short_id=4",
                    [metadata.to_string()],
                )
                .unwrap();
        });
        settle(cx, |cx| {
            cx.update(|cx| {
                view.read(cx)
                    .model
                    .rows
                    .iter()
                    .any(|r| r.id == 4 && r.source_url == expected)
            })
        });
        cx.update_window(window.into(), |_, w, cx| {
            w.render_frame(cx);
            assert!(w.try_find("source-bar").is_none());
            assert!(w.try_find("open-source").is_none());
        })
        .unwrap();
    }
    cx.update_window(window.into(), |_, w, _| {
        w.remove_window();
    })
    .unwrap();
    drop(view);
    let (window, view) = append_tests::open(cx, services);
    settle(cx, |cx| cx.update(|cx| view.read(cx).loaded));
    cx.update_window(window.into(), |_, w, cx| {
        toast(w, cx, false);
        w.remove_window();
    })
    .unwrap();
    runtime
        .block_on(host.run_until(async { Ok(()) }, || {}))
        .unwrap();
}

#[gpui_kit::test]
#[allow(clippy::too_many_lines)] // One window keeps the clock, focus and recovery journey observable.
fn copy_refreshes_one_second_deadline_preserves_focus_draft_and_errors(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let root = tempfile::tempdir().unwrap();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let (host, _) = note_action_tests::fixture(&runtime, root.path());
    let services = append_tests::services(&host, &runtime);
    cx.update(gpui_kit::init);
    let (window, view) = append_tests::open(cx, services);
    settle(cx, |cx| cx.update(|cx| view.read(cx).loaded));
    cx.update_window(window.into(), |_, w, cx| {
        view.update(cx, |this, cx| {
            this.select(4, w, cx);
            this.composer.update(cx, |i, cx| {
                i.set_value("keep draft", w, cx);
                i.set_selected_range(2..5, cx);
            });
            this.model.capture().error = Some("Retained error".into());
            this.error = Some("Retained error".into());
        });
        w.render_frame(cx);
        w.click("copy-detail", cx);
        toast(w, cx, true);
        assert!(
            view.read(cx)
                .composer
                .read(cx)
                .focus_handle(cx)
                .is_focused(w)
        );
    })
    .unwrap();
    cx.run_until_parked();
    cx.executor().advance_clock(Duration::from_millis(700));
    cx.run_until_parked();
    cx.update_window(window.into(), |_, w, cx| {
        toast(w, cx, true);
        w.click("copy-detail", cx);
        toast(w, cx, true);
    })
    .unwrap();
    cx.run_until_parked();
    cx.executor().advance_clock(Duration::from_millis(400));
    cx.run_until_parked();
    cx.update_window(window.into(), |_, w, cx| {
        toast(w, cx, true);
        assert_eq!(view.read(cx).composer.read(cx).value(), "keep draft");
        assert_eq!(view.read(cx).composer.read(cx).selected_range(), 2..5);
        assert_eq!(view.read(cx).error.as_deref(), Some("Retained error"));
    })
    .unwrap();
    cx.executor().advance_clock(Duration::from_millis(601));
    cx.run_until_parked();
    cx.update_window(window.into(), |_, w, cx| {
        toast(w, cx, false);
        let before = cx.read_from_clipboard().and_then(|item| item.text());
        view.update(cx, |this, cx| {
            this.model
                .capture()
                .note_actions
                .links
                .extend([String::new(), "  ".into()]);
            this.refresh_capture(w, cx);
        });
        toast(w, cx, false);
        assert_eq!(
            cx.read_from_clipboard().and_then(|item| item.text()),
            before
        );
        // Feed retained results through the real capture/append state, then activate their buttons.
        view.update(cx, |this, _| {
            let token = this.model.accept("captured uncertain text".into());
            let error = flicknote_client::WireError {
                code: "note_create_unknown".into(),
                message: "Check first".into(),
                retryable: false,
                details: None,
            };
            this.model.capture().uncertain(token, error.clone());
            let row = this.model.rows[0].clone();
            let mut capture = this.model.capture();
            let token = capture
                .appends
                .accept(&row, "submitted recovery text".into())
                .unwrap();
            capture.appends.complete(token, Err(error));
        });
        w.render_frame(cx);
        for (action, text) in [
            ("copy-uncertain", "captured uncertain text"),
            ("copy-append-1", "submitted recovery text"),
        ] {
            w.click(action, cx);
            assert_eq!(cx.read_from_clipboard().unwrap().text().unwrap(), text);
            toast(w, cx, true);
            assert_eq!(view.read(cx).composer.read(cx).value(), "keep draft");
            assert_eq!(view.read(cx).composer.read(cx).selected_range(), 2..5);
        }
        w.remove_window();
    })
    .unwrap();
    runtime
        .block_on(host.run_until(async { Ok(()) }, || {}))
        .unwrap();
}
