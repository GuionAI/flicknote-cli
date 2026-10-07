//! Actual production LocalHost/FTS and rendered Kit input over owned fake HTTP and port0.
use super::tests::settle;
use super::*;
use flicknote_sync::workspace_search::{self, Results};
use gpui_kit::{Focusable, TestAppContext, test::TestWindowExt};

#[gpui_kit::test]
fn hello_search_uses_same_today_rows(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let root = tempfile::tempdir().unwrap();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let host = append_tests::fixture(&runtime, root.path());
    seed(&runtime, &host);
    runtime.block_on(async {
        let writer = host.db.writer().await.unwrap();
        for (id, content, title, kind, project) in [
            (3, format!("{} hello excerpt", "界".repeat(200)), Some("标题  保留\n下一行"), "meeting", Some("search-project")),
            (4, format!("{} hello", "x".repeat(600)), None, "normal", None),
            (5, format!("{} hello", "x".repeat(600)), Some(""), "flash", Some("search-project")),
            (6, "hello\n\n短文  保留".into(), Some("Unused title"), "link", Some("search-project")),
        ] {
            writer.execute("INSERT OR REPLACE INTO notes(id,short_id,user_id,content,type,status,is_flagged,title,metadata,created_at,project_id) VALUES(?,CAST(? AS INTEGER),'append-owner',?,?,'ready',0,?,'{}',strftime('%Y-%m-%dT%H:%M:%SZ','now'),?)", [Some(format!("00000000-0000-4000-8000-{id:012}")), Some(id.to_string()), Some(content), Some(kind.to_owned()), title.map(str::to_owned), project.map(str::to_owned)]).unwrap();
        }
    });
    let services = append_tests::services(&host, &runtime);
    cx.update(|cx| {
        gpui_kit::init(cx);
        install_today_keys(cx);
    });
    let (window, view) = append_tests::open(cx, services.clone());
    settle(cx, |cx| {
        cx.update(|cx| view.read(cx).loaded && view.read(cx).model.rows.len() == 6)
    });
    for (mode, width) in [
        (gpui_kit::component::ThemeMode::Light, 760.),
        (gpui_kit::component::ThemeMode::Light, 980.),
        (gpui_kit::component::ThemeMode::Dark, 760.),
        (gpui_kit::component::ThemeMode::Dark, 980.),
    ] {
        let ordinary = cx
            .update_window(window.into(), |_, w, cx| {
                apply_theme(mode, cx);
                w.resize(size(px(width), px(560.)));
                w.render_frame(cx);
                let rows = [3, 4, 5, 6].map(|id| row_presentation(w, id));
                assert_eq!(rows[0].0, "标题  保留 下一行");
                assert_eq!(rows[1].0, "Untitled note");
                assert_eq!(rows[2].0, "");
                assert_eq!(rows[3].0, "hello 短文  保留");
                w.press("cmd-f", cx);
                w.input("hello", cx);
                rows
            })
            .unwrap();
        settle(cx, |cx| {
            cx.update(|cx| view.read(cx).search.active() && !view.read(cx).search.loading)
        });
        cx.update_window(window.into(), |_, w, cx| {
            w.render_frame(cx);
            assert_eq!(view.read(cx).search.query, "hello");
            assert_eq!(view.read(cx).search.error, None);
            assert_eq!(view.read(cx).model.rows.len(), 4);
            assert_eq!(
                (
                    w.find(("note", 3_u64)).bounds().size.height,
                    w.try_find("search-status").is_some(),
                    w.try_find(("match-excerpt", 3_u64)).is_some()
                ),
                (px(32.), false, false),
                "hello must use the Today row with no successful strip or excerpt"
            );
            assert_eq!([3, 4, 5, 6].map(|id| row_presentation(w, id)), ordinary);
            assert_eq!(
                w.find(("note", view.read(cx).model.rows[0].id as u64))
                    .bounds()
                    .top(),
                w.find("destination-header").bounds().bottom()
            );
            assert!(w.try_find("only-mine").is_some());
            assert_eq!(
                view.read(cx)
                    .model
                    .rows
                    .iter()
                    .map(|r| r.id)
                    .collect::<Vec<_>>(),
                find(&runtime, &host, "hello", false, None)
                    .rows
                    .iter()
                    .map(|r| r.id)
                    .collect::<Vec<_>>()
            );
            w.press("escape", cx);
        })
        .unwrap();
        settle(cx, |cx| {
            cx.update(|cx| view.read(cx).loaded && view.read(cx).model.rows.len() == 6)
        });
    }
    cx.update_window(window.into(), |_, w, _| w.remove_window())
        .unwrap();
    services.cancel_operations();
    runtime.block_on(host.shutdown());
}

fn row_presentation(
    w: &mut gpui_kit::Window,
    id: u64,
) -> (String, Vec<gpui_kit::Bounds<gpui_kit::Pixels>>) {
    let row = w.find(("note", id)).bounds();
    let scope = w.within(("note", id));
    let text = scope.find("note-preview");
    let mut bounds = vec![row, text.bounds(), scope.find("type-glyph").bounds()];
    if let Some(dot) = scope.try_find("project-dot") {
        bounds.push(dot.bounds());
    }
    for bound in &mut bounds {
        bound.origin -= row.origin;
    }
    (text.label().unwrap().to_owned(), bounds)
}

#[gpui_kit::test]
fn empty_search_escape_returns_composer(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let root = tempfile::tempdir().unwrap();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let host = append_tests::fixture(&runtime, root.path());
    let services = append_tests::services(&host, &runtime);
    cx.update(|cx| {
        gpui_kit::init(cx);
        install_today_keys(cx);
    });
    let (window, view) = append_tests::open(cx, services.clone());
    settle(cx, |cx| cx.update(|cx| view.read(cx).loaded));
    cx.update_window(window.into(), |_, w, cx| {
        let composer = view.read(cx).composer.clone();
        let input = view.read(cx).search_input.clone();
        w.render_frame(cx);
        for draft in ["", "kept draft"] {
            if !draft.is_empty() {
                w.click(("note", 2_u64), cx);
            }
            composer.update(cx, |i, cx| {
                i.set_value(draft, w, cx);
                i.set_selected_range(if draft.is_empty() { 0..0 } else { 2..5 }, cx);
                i.focus(w, cx);
            });
            let before = (
                view.read(cx).destination.clone(),
                view.read(cx).period.clone(),
                view.read(cx).model.selected,
                view.read(cx).list_scroll.0.borrow().base_handle.offset(),
            );
            w.render_frame(cx);
            w.press("cmd-f", cx);
            w.render_frame(cx);
            assert!(
                input.read(cx).focus_handle(cx).is_focused(w),
                "search focus"
            );
            w.press("cmd-f", cx);
            assert!(input.read(cx).value().is_empty());
            w.press("escape", cx);
            w.render_frame(cx);
            assert!(
                composer.read(cx).focus_handle(cx).is_focused(w),
                "composer focus"
            );
            assert_eq!(composer.read(cx).value(), draft);
            assert_eq!(
                composer.read(cx).selected_range(),
                if draft.is_empty() { 0..0 } else { 2..5 }
            );
            assert_eq!(
                before,
                (
                    view.read(cx).destination.clone(),
                    view.read(cx).period.clone(),
                    view.read(cx).model.selected,
                    view.read(cx).list_scroll.0.borrow().base_handle.offset()
                )
            );
            assert_eq!(view.read(cx).detail_open, !draft.is_empty());
            w.input("X", cx);
            assert_eq!(
                composer.read(cx).value(),
                if draft.is_empty() { "X" } else { "keXdraft" }
            );
        }
        composer.update(cx, |i, cx| {
            i.focus(w, cx);
            i.replace_and_mark_text_in_range(None, "ni", Some(2..2), w, cx);
        });
        w.render_frame(cx);
        w.press("cmd-f", cx);
        assert!(composer.read(cx).focus_handle(cx).is_focused(w));
        composer.update(cx, |i, cx| i.unmark_text(w, cx));
        composer.update(cx, |i, cx| {
            i.set_value("native caret", w, cx);
            i.set_selected_range(3..3, cx);
        });
        w.render_frame(cx);
        for key in ["ctrl-f", "ctrl-b"] {
            w.press(key, cx);
            assert!(composer.read(cx).focus_handle(cx).is_focused(w));
            assert_eq!(composer.read(cx).value(), "native caret");
        }
        // Ordinary arrow caret motion is a separate Kit editing contract.
        composer.update(cx, |i, cx| i.set_selected_range(3..3, cx));
        w.press("right", cx);
        assert_eq!(composer.read(cx).selected_range(), 4..4);
        w.press("left", cx);
        assert_eq!(composer.read(cx).selected_range(), 3..3);

        view.update(cx, |v, cx| v.edit(project_editor::Kind::Add, w, cx));
        w.render_frame(cx);
        let editor = view.read(cx).editor.as_ref().unwrap().input.clone();
        w.press("cmd-f", cx);
        assert!(editor.read(cx).focus_handle(cx).is_focused(w));
        w.remove_window();
    })
    .unwrap();
    services.cancel_operations();
    runtime.block_on(host.shutdown());
}

#[gpui_kit::test]
fn escape_invalidates_pending_and_failed_search(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let root = tempfile::tempdir().unwrap();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let host = append_tests::fixture(&runtime, root.path());
    seed(&runtime, &host);
    let services = append_tests::services(&host, &runtime);
    cx.update(|cx| {
        gpui_kit::init(cx);
        install_today_keys(cx);
    });
    let (window, view) = append_tests::open(cx, services.clone());
    settle(cx, |cx| cx.update(|cx| view.read(cx).loaded));
    let result = find(&runtime, &host, "buriedneedle", false, None);
    for failed in [false, true] {
        cx.update_window(window.into(), |_, w, cx| {
            let composer = view.read(cx).composer.clone();
            composer.update(cx, |i, cx| {
                i.set_value("preserved", w, cx);
                i.set_selected_range(3..3, cx);
            });
            query(&view, "buriedneedle", w, cx);
            let generation = view.read(cx).search.generation;
            if failed {
                view.update(cx, |v, cx| {
                    v.receive_search(generation, false, Err("owned failure".into()), w, cx)
                });
            }
            w.render_frame(cx);
            w.press("escape", cx);
            assert!(!view.read(cx).search.active());
            assert!(composer.read(cx).focus_handle(cx).is_focused(w));
            view.update(cx, |v, cx| {
                v.receive_search(generation, false, Ok(result.clone()), w, cx)
            });
            assert!(!view.read(cx).search.active());
            assert_eq!(view.read(cx).model.selected, None);
            assert!(view.read(cx).search_input.read(cx).value().is_empty());
            assert_eq!(composer.read(cx).value(), "preserved");
            assert_eq!(composer.read(cx).selected_range(), 3..3);
        })
        .unwrap();
        settle(cx, |cx| {
            cx.update(|cx| view.read(cx).loaded && view.read(cx).model.rows.len() == 2)
        });
    }
    cx.update_window(window.into(), |_, w, _| w.remove_window())
        .unwrap();
    services.cancel_operations();
    runtime.block_on(host.shutdown());
}

#[gpui_kit::test]
fn review_capture_refresh_cannot_revive_previous_query_rows(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let root = tempfile::tempdir().unwrap();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let host = append_tests::fixture(&runtime, root.path());
    seed(&runtime, &host);
    let services = append_tests::services(&host, &runtime);
    cx.update(|cx| {
        gpui_kit::init(cx);
        install_today_keys(cx);
    });
    let (window, view) = append_tests::open(cx, services.clone());
    settle(cx, |cx| cx.update(|cx| view.read(cx).loaded));
    let previous = find(&runtime, &host, "buriedneedle", false, None);
    cx.update_window(window.into(), |_, window, cx| {
        query(&view, "buriedneedle", window, cx);
        view.update(cx, |view, cx| {
            view.receive_search(view.search.generation, false, Ok(previous), window, cx);
        });
        assert_eq!(view.read(cx).model.rows[0].id, 3);
        view.update(cx, |view, cx| view.refresh_capture(window, cx));
        assert_eq!(view.read(cx).model.rows[0].id, 3);
        view.update(cx, |view, cx| {
            view.receive_search(
                view.search.generation,
                false,
                Err("offline".into()),
                window,
                cx,
            );
            view.refresh_capture(window, cx);
        });
        assert!(view.read(cx).search.error.is_some());
        assert!(view.read(cx).model.rows.is_empty());
        let previous = find(&runtime, &host, "buriedneedle", false, None);
        view.update(cx, |view, cx| {
            view.receive_search(view.search.generation, false, Ok(previous), window, cx);
        });
        assert_eq!(view.read(cx).model.rows[0].id, 3);
        query(&view, "unmatchable", window, cx);
        assert!(view.read(cx).model.rows.is_empty());
        view.update(cx, |view, cx| view.refresh_capture(window, cx));
        assert_eq!(view.read(cx).search.query, "unmatchable");
        assert!(view.read(cx).search.loading);
        assert!(
            view.read(cx).model.rows.is_empty(),
            "a capture/action completion must not restore the previous query's canonical rows"
        );
    })
    .unwrap();
    cx.update_window(window.into(), |_, window, _| window.remove_window())
        .unwrap();
    drop(view);
    services.cancel_operations();
    runtime.block_on(host.shutdown());
}

fn seed(runtime: &tokio::runtime::Runtime, host: &flicknote_sync::LocalHost) {
    runtime.block_on(async {
        let writer = host.db.writer().await.unwrap();
        writer.execute("INSERT INTO projects(id,user_id,name,color,is_archived) VALUES('search-project','append-owner','Research','#2864b4',0)", []).unwrap();
        writer.execute("INSERT INTO notes(id,short_id,user_id,content,type,status,is_flagged,title,metadata,created_at,project_id) VALUES('00000000-0000-4000-8000-000000000003',3,'append-owner',?,'normal','ready',0,'Unrelated title','{}','2020-01-02T12:00:00Z','search-project')", [format!("# Canonical\n\n{}\n\nburiedneedle\n\n```rust\n  exact whitespace  \n```", "x".repeat(600))]).unwrap();
    });
}
fn find(
    runtime: &tokio::runtime::Runtime,
    host: &flicknote_sync::LocalHost,
    query: &str,
    human: bool,
    selected: Option<i64>,
) -> Results {
    runtime
        .block_on(workspace_search::read(
            &host.app,
            &host.db,
            &host.user_id,
            query,
            human,
            selected,
        ))
        .unwrap()
}

fn seed_large(runtime: &tokio::runtime::Runtime, host: &flicknote_sync::LocalHost) {
    runtime.block_on(async {
        let writer = host.db.writer().await.unwrap();
        writer.execute_batch("WITH RECURSIVE ids(id) AS (SELECT 100 UNION ALL SELECT id+1 FROM ids WHERE id<10100) INSERT INTO notes(id,short_id,user_id,content,type,status,is_flagged,metadata,created_at) SELECT 'bulk-'||id,id,'append-owner','boundedneedle','normal','ready',0,'{}',strftime('%Y-%m-%dT%H:%M:%SZ','now') FROM ids;").unwrap();
        for (id, metadata, status, deleted) in [
            (20,"{}","ready",None), (21,"{\"created_by_ai\":false}","ready",None),
            (22,"{\"created_by_ai\":true}","ready",None), (23,"{\"created_by_ai\":null}","ready",None),
            (24,"{\"created_by_ai\":\"true\"}","ready",None), (25,"{\"created_by_ai\":1}","ready",None),
            (26,"{}","draft",None), (27,"{}","ready",Some("2020-01-01")),
        ] {
            writer.execute("INSERT INTO notes(id,short_id,user_id,content,type,status,is_flagged,metadata,created_at,deleted_at) VALUES(?,CAST(? AS INTEGER),'append-owner','matrixneedle','normal',?,0,?,strftime('%Y-%m-%dT%H:%M:%SZ','now'),?)", [Some(format!("00000000-0000-4000-8000-{id:012}")),Some(id.to_string()),Some(status.into()),Some(metadata.into()),deleted.map(str::to_owned)]).unwrap();
        }
        writer.execute("UPDATE notes SET metadata='{\"created_by_ai\":true}' WHERE short_id>=10050", []).unwrap();
    });
}

fn assert_backend_rank(
    runtime: &tokio::runtime::Runtime,
    host: &flicknote_sync::LocalHost,
    bounded: &Results,
) {
    let AppResponse::SearchHits(raw) = runtime
        .block_on(
            host.app
                .handle(AppRequest::NoteFind(flicknote_client::dto::NoteFindInput {
                    keywords: vec!["boundedneedle".into()],
                    extractions: vec![],
                    project: None,
                    created_after: None,
                    created_before: None,
                    human: true,
                    archived: false,
                    limit: 50,
                })),
        )
        .unwrap()
    else {
        panic!("search response")
    };
    assert_eq!(
        bounded.hits, raw,
        "backend rank and highlighting remain authoritative"
    );
}

fn assert_exact_fallback(runtime: &tokio::runtime::Runtime, host: &flicknote_sync::LocalHost) {
    assert_eq!(
        find(runtime, host, "99999999", true, None).rows[0].id,
        20,
        "missing exact ID still discovers lexical matches"
    );
    let dedup = find(runtime, host, "22", false, None);
    assert_eq!(dedup.rows.iter().filter(|r| r.id == 22).count(), 1);
    assert!(dedup.hits[0].snippet.segments.iter().any(|s| s.highlighted));
    assert!(find(runtime, host, "987654321", true, None).rows.is_empty());
}

#[test]
fn actual_fts_global_ranking_bounds_provenance_and_exact_access() {
    let root = tempfile::tempdir().unwrap();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let host = append_tests::fixture(&runtime, root.path());
    seed(&runtime, &host);
    seed_large(&runtime, &host);
    runtime.block_on(async {
        let writer = host.db.writer().await.unwrap();
        writer
            .execute(
                "UPDATE notes SET content=content||' 22' WHERE short_id=22",
                [],
            )
            .unwrap();
        writer
            .execute(
                "UPDATE notes SET content=content||' 99999999' WHERE short_id=20",
                [],
            )
            .unwrap();
    });
    let body = find(&runtime, &host, "buriedneedle", false, None);
    assert_eq!(body.rows.len(), 1);
    assert_eq!(body.rows[0].uuid, "00000000-0000-4000-8000-000000000003");
    assert_eq!(body.rows[0].preview, "Unrelated title");
    assert!(
        body.rows[0].content.is_empty(),
        "discovery never preloads bodies"
    );
    assert!(
        body.hits[0]
            .snippet
            .segments
            .iter()
            .any(|s| s.highlighted && s.text.contains("buriedneedle"))
    );
    runtime.block_on(async {
        host.db
            .writer()
            .await
            .unwrap()
            .execute(
                "UPDATE notes SET created_at=strftime('%Y-%m-%dT%H:%M:%SZ','now') WHERE short_id=3",
                [],
            )
            .unwrap();
        let watcher = flicknote_sync::today::TodayWatch::start_for_user(
            host.db.clone(),
            host.user_id.clone(),
        );
        let mut receive = watcher.receiver.clone();
        let rows = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if let Some(result) = receive.borrow_and_update().clone() {
                    break result.unwrap().rows;
                }
                receive.changed().await.unwrap();
            }
        })
        .await
        .unwrap();
        assert_eq!(rows.len(), 10_000);
        assert!(
            !rows.iter().any(|r| r.id == 3),
            "FTS finds a body match outside the bounded loaded slice"
        );
    });
    let selected = find(&runtime, &host, "buriedneedle", false, Some(3));
    assert_eq!(selected.detail, Some(3));
    assert!(selected.rows[0].content.starts_with("# Canonical"));
    let matrix = find(&runtime, &host, "matrixneedle", true, None);
    assert_eq!(
        std::collections::BTreeSet::from_iter(matrix.rows.iter().map(|r| r.id)),
        std::collections::BTreeSet::from([20, 21, 23, 24, 25])
    );
    let bounded = find(&runtime, &host, "boundedneedle", true, None);
    assert!(bounded.bounded);
    assert_eq!(bounded.rows.len(), 50);
    assert!(bounded.rows.iter().all(|r| r.id < 10050));
    assert_backend_rank(&runtime, &host, &bounded);
    for (query, id) in [("#22", 22), ("26", 26), ("#27", 27)] {
        let exact = find(&runtime, &host, query, true, Some(id));
        assert_eq!(exact.rows[0].id, id);
        assert_eq!(exact.detail, Some(id));
        assert_eq!(exact.rows.iter().filter(|r| r.id == id).count(), 1);
    }
    assert!(
        find(&runtime, &host, "matrixneedle", false, None)
            .rows
            .iter()
            .all(|r| !r.draft && !r.archived)
    );
    assert_exact_fallback(&runtime, &host);
    runtime.block_on(host.shutdown());
}

#[gpui_kit::test]
#[allow(clippy::too_many_lines)] // A continuous real-host window checks retained input and canonical actions.
fn native_search_input_outside_watch_reader_append_escape_and_rail(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let root = tempfile::tempdir().unwrap();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let host = append_tests::fixture(&runtime, root.path());
    seed(&runtime, &host);
    let services = append_tests::services(&host, &runtime);
    cx.update(|cx| {
        gpui_kit::init(cx);
        install_today_keys(cx);
    });
    let (window, view) = append_tests::open(cx, services.clone());
    settle(cx, |cx| cx.update(|cx| view.read(cx).loaded));
    let composer = cx.update(|cx| view.read(cx).composer.clone());
    let input = cx.update(|cx| view.read(cx).search_input.clone());
    cx.update_window(window.into(), |_, w, cx| {
        w.resize(size(px(980.), px(720.)));
        w.render_frame(cx);
        w.click(("note", 2_u64), cx);
        composer.update(cx, |i, cx| {
            i.set_value("kept draft", w, cx);
            i.set_selected_range(2..5, cx);
        });
        w.press("cmd-f", cx);
        w.render_frame(cx);
        assert!(input.read(cx).focus_handle(cx).is_focused(w));
        let header = w.find("destination-header").bounds();
        assert_eq!(w.find("period-header").bounds().top(), header.bottom());
        assert_eq!(
            view.read(cx).model.rows.len(),
            2,
            "empty search keeps browsing"
        );
        w.input("buriedneedle", cx);
    })
    .unwrap();
    settle(cx, |cx| {
        cx.update(|cx| !view.read(cx).search.loading && view.read(cx).model.rows.len() == 1)
    });
    cx.update_window(window.into(), |_, w, cx| {
        assert_eq!(composer.read(cx).value(), "kept draft");
        assert_eq!(composer.read(cx).selected_range(), 2..5);
        w.render_frame(cx);
        w.press("down", cx);
    })
    .unwrap();
    settle(cx, |cx| {
        cx.update(|cx| {
            view.read(cx)
                .detail
                .as_ref()
                .is_some_and(|r| r.uuid == "00000000-0000-4000-8000-000000000003")
        })
    });
    for (mode, width) in [
        (gpui_kit::component::ThemeMode::Light, 760.),
        (gpui_kit::component::ThemeMode::Light, 980.),
        (gpui_kit::component::ThemeMode::Dark, 760.),
        (gpui_kit::component::ThemeMode::Dark, 980.),
    ] {
        cx.update_window(window.into(), |_, w, cx| {
            apply_theme(mode, cx);
            w.resize(size(px(width), px(560.)));
            w.press("cmd-f", cx);
            w.render_frame(cx);
            let center = w.find("center-pane").bounds();
            let row = w.find(("note", 3_u64)).bounds();
            assert_eq!(row.size.height, px(32.));
            assert!(row.right() <= center.right());
            let rail = w.find("navigation-rail").bounds();
            let search = w.find("workspace-search").bounds();
            let field = w.find(("input", input.entity_id()));
            assert_eq!(field.label(), Some("Search notes"));
            assert_eq!(field.bounds().size, search.size);
            assert_eq!(field.bounds().origin, search.origin);
            let cell = search.scale(w.scale_factor());
            let zero = px(0.).scale(w.scale_factor());
            for quad in w.painted_quads().into_iter().filter(|q| {
                q.bounds.top() >= cell.top()
                    && q.bounds.bottom() <= cell.bottom()
                    && q.bounds.left() >= cell.left()
                    && q.bounds.right() <= cell.right()
                    && q.bounds.size.width > px(100.).scale(w.scale_factor())
            }) {
                assert_eq!(
                    (
                        quad.border_widths.top,
                        quad.border_widths.left,
                        quad.border_widths.right
                    ),
                    (zero, zero, zero)
                );
                assert_eq!(quad.corner_radii, Default::default(), "no inset capsule");
            }
            let text = input.read(cx).text_bounds().unwrap();
            let clean = w.find("clean").bounds();
            assert_eq!(search.top(), rail.top());
            assert_eq!(search.size.height, px(44.));
            assert!(search.right() <= rail.right() && search.right() <= center.left());
            assert!(text.left() > search.left() && text.right() < clean.left());
            assert!(clean.right() < rail.right() && clean.bottom() <= search.bottom());
            assert_eq!(
                w.find("today-notes").bounds().top(),
                w.find("destination-header").bounds().bottom()
            );
            assert!(input.read(cx).focus_handle(cx).is_focused(w));
            let reader = view.read(cx).detail.as_ref().unwrap().state.clone();
            reader.read(cx).focus_handle().clone().focus(w, cx);
            w.render_frame(cx);
            w.press("cmd-f", cx);
            assert!(input.read(cx).focus_handle(cx).is_focused(w));
            w.press("tab", cx);
            w.render_frame(cx);
            assert!(!input.read(cx).focus_handle(cx).is_focused(w));
            w.press("cmd-f", cx);
            assert!(input.read(cx).focus_handle(cx).is_focused(w));
            assert_eq!(input.read(cx).value(), "buriedneedle");
            search_transient_escape(&input, w, cx);
            assert!(view.read(cx).search.active());
            search_selection_release(&input, w, cx);
            assert!(view.read(cx).search.active() && view.read(cx).detail_open);
            w.click("copy-detail", cx);
            assert!(
                cx.read_from_clipboard()
                    .unwrap()
                    .text()
                    .unwrap()
                    .contains("  exact whitespace  ")
            );
        })
        .unwrap();
    }
    cx.update_window(window.into(), |_, w, cx| {
        let reader = view.read(cx).detail.as_ref().unwrap().state.clone();
        reader.read(cx).focus_handle().clone().focus(w, cx);
        w.render_frame(cx);
        w.press("escape", cx);
        assert!(view.read(cx).search.active() && !view.read(cx).detail_open);
        w.press("cmd-f", cx);
        w.press("enter", cx);
    })
    .unwrap();
    settle(cx, |cx| cx.update(|cx| view.read(cx).detail_open));
    cx.update_window(window.into(), |_, w, cx| {
        composer.update(cx, |i, cx| {
            i.set_value("appended through search", w, cx);
            i.focus(w, cx);
        });
        w.press("enter", cx);
    })
    .unwrap();
    settle(cx, |cx| {
        cx.update(|cx| view.read(cx).model.capture().appends.pending.is_empty())
    });
    let canonical = find(&runtime, &host, "#3", false, Some(3));
    assert!(
        canonical.rows[0]
            .content
            .ends_with("\n\nappended through search")
    );
    cx.update_window(window.into(), |_, w, cx| {
        w.press("cmd-f", cx);
        w.press("escape", cx);
    })
    .unwrap();
    settle(cx, |cx| {
        cx.update(|cx| {
            !view.read(cx).search.active()
                && view.read(cx).loaded
                && view.read(cx).model.rows.len() == 2
        })
    });
    cx.update_window(window.into(), |_, w, cx| {
        assert_eq!(view.read(cx).model.selected, Some(2));
        assert!(composer.read(cx).focus_handle(cx).is_focused(w));
        composer.update(cx, |i, cx| {
            i.set_value("rail draft", w, cx);
            i.set_selected_range(2..5, cx);
        });
        w.press("cmd-f", cx);
        w.input("buriedneedle", cx);
    })
    .unwrap();
    settle(cx, |cx| {
        cx.update(|cx| view.read(cx).search.active() && !view.read(cx).search.loading)
    });
    cx.update_window(window.into(), |_, w, cx| {
        w.render_frame(cx);
        w.press("enter", cx);
    })
    .unwrap();
    settle(cx, |cx| cx.update(|cx| view.read(cx).detail_open));
    cx.update_window(window.into(), |_, w, cx| {
        w.render_frame(cx);
        w.click("clean", cx);
    })
    .unwrap();
    settle(cx, |cx| {
        cx.update(|cx| !view.read(cx).search.active() && view.read(cx).loaded)
    });
    cx.update_window(window.into(), |_, w, cx| {
        assert_eq!(view.read(cx).model.selected, Some(2));
        assert_eq!(composer.read(cx).value(), "rail draft");
        assert_eq!(composer.read(cx).selected_range(), 2..5);
        w.press("cmd-f", cx);
        w.input("buriedneedle", cx);
    })
    .unwrap();
    settle(cx, |cx| cx.update(|cx| view.read(cx).search.active()));
    cx.update_window(window.into(), |_, w, cx| {
        w.render_frame(cx);
        w.click("home", cx);
        w.press("escape", cx);
        assert!(!view.read(cx).search.active());
        assert!(input.read(cx).value().is_empty());
        w.remove_window();
    })
    .unwrap();
    drop(view);
    services.cancel_operations();
    runtime.block_on(host.shutdown());
}

fn search_transient_escape(input: &Entity<InputState>, w: &mut Window, cx: &mut App) {
    use gpui_kit::InputEvent as _;
    let at = input.read(cx).text_bounds().unwrap().origin + gpui_kit::point(px(4.), px(8.));
    w.dispatch_event(
        gpui_kit::MouseMoveEvent {
            position: at,
            pressed_button: None,
            modifiers: Default::default(),
        }
        .to_platform_input(),
        cx,
    );
    w.render_frame(cx);
    for phase in [gpui_kit::TouchPhase::Started, gpui_kit::TouchPhase::Ended] {
        w.dispatch_event(
            gpui_kit::LongPressEvent {
                phase,
                start_position: at,
                position: at,
            }
            .to_platform_input(),
            cx,
        );
        w.render_frame(cx);
    }
    assert!(input.read(cx).touch_selection().is_some());
    w.press("escape", cx);
    w.render_frame(cx);
    assert!(input.read(cx).touch_selection().is_none());
    assert!(input.read(cx).focus_handle(cx).is_focused(w));
}

fn search_selection_release(input: &Entity<InputState>, w: &mut Window, cx: &mut App) {
    use gpui_kit::InputEvent as _;
    let text = input.read(cx).text_bounds().unwrap();
    let start = text.origin + gpui_kit::point(px(2.), text.size.height / 2.);
    let end = start + gpui_kit::point(px(45.), px(0.));
    super::markdown_tests::drag_text(w, cx, start, end);
    let selected = input.read(cx).selected_range();
    assert!(!selected.is_empty());
    w.dispatch_event(
        gpui_kit::MouseMoveEvent {
            position: start,
            pressed_button: None,
            modifiers: Default::default(),
        }
        .to_platform_input(),
        cx,
    );
    w.render_frame(cx);
    assert_eq!(input.read(cx).selected_range(), selected);
    w.press("cmd-c", cx);
    assert_eq!(
        cx.read_from_clipboard().unwrap().text().unwrap(),
        &input.read(cx).value()[selected]
    );
}

pub(super) fn query(view: &Entity<Today>, text: &str, w: &mut Window, cx: &mut App) {
    view.update(cx, |this, cx| {
        this.search_input.update(cx, |i, cx| {
            i.set_value(text, w, cx);
            i.focus(w, cx);
        });
        this.search_changed(w, cx);
    });
}
#[gpui_kit::test]
#[allow(clippy::too_many_lines)] // Preserve one owned window's generations, periods, composition and recovery.
fn query_generations_source_ime_errors_origin_period_and_close_reopen(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let root = tempfile::tempdir().unwrap();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let host = append_tests::fixture(&runtime, root.path());
    seed(&runtime, &host);
    let previous = Period::Day(None)
        .shifted(false, &chrono::Local::now())
        .unwrap();
    let range = previous.range(&chrono::Local::now()).unwrap().unwrap();
    runtime.block_on(async {
        let writer=host.db.writer().await.unwrap();
        writer.execute("UPDATE notes SET created_at=?, metadata='{}',project_id='search-project' WHERE short_id=1",[range.0.to_rfc3339()]).unwrap();
    });
    let services = append_tests::services(&host, &runtime);
    cx.update(|cx| {
        gpui_kit::init(cx);
        install_today_keys(cx);
    });
    let (window, view) = append_tests::open(cx, services.clone());
    settle(cx, |cx| cx.update(|cx| view.read(cx).loaded));
    let composer = cx.update(|cx| view.read(cx).composer.clone());
    cx.update_window(window.into(), |_, w, cx| {
        view.update(cx, |v, cx| v.mouse_period(false, w, cx));
    })
    .unwrap();
    settle(cx, |cx| {
        cx.update(|cx| view.read(cx).loaded && view.read(cx).model.rows.len() == 1)
    });
    cx.update_window(window.into(), |_, w, cx| {
        view.update(cx, |v, cx| v.select(1, w, cx));
        query(&view, "buriedneedle", w, cx);
    })
    .unwrap();
    let old_generation = cx.update(|cx| view.read(cx).search.generation);
    let old = find(&runtime, &host, "buriedneedle", false, None);
    cx.update_window(window.into(), |_, w, cx| {
        query(&view, "unmatchable", w, cx);
        w.render_frame(cx);
        assert_eq!(w.find("search-status").label(), Some("Searching…"));
    })
    .unwrap();
    cx.update_window(window.into(), |_, w, cx| {
        view.update(cx, |v, cx| {
            v.receive_search(old_generation, false, Ok(old.clone()), w, cx)
        });
        assert!(view.read(cx).model.rows.is_empty());
    })
    .unwrap();
    settle(cx, |cx| cx.update(|cx| !view.read(cx).search.loading));
    cx.update_window(window.into(), |_, w, cx| {
        w.render_frame(cx);
        assert_eq!(w.find("search-status").label(), Some("No matching notes"));
        view.update(cx, |v, cx| {
            v.receive_search(
                v.search.generation,
                false,
                Err("owned failure".into()),
                w,
                cx,
            )
        });
        w.render_frame(cx);
        assert_eq!(w.find("search-status").label(), Some("Search unavailable"));
        assert!(w.try_find("retry-search").is_some());
        w.click("retry-search", cx);
    })
    .unwrap();
    settle(cx, |cx| {
        cx.update(|cx| !view.read(cx).search.loading && view.read(cx).search.error.is_none())
    });
    cx.update_window(window.into(), |_, w, cx| {
        let input = view.read(cx).search_input.clone();
        input.update(cx, |i, cx| {
            i.replace_and_mark_text_in_range(None, "ni", Some(2..2), w, cx)
        });
        view.update(cx, |v, cx| v.search_changed(w, cx));
        w.render_frame(cx);
        w.press("cmd-f", cx);
        assert!(input.read(cx).focus_handle(cx).is_focused(w));
        w.press("enter", cx);
        w.press("escape", cx);
        assert!(view.read(cx).search.active());
        assert!(view.read(cx).search.job.is_none());
        input.update(cx, |i, cx| {
            i.unmark_text(w, cx);
            i.set_value("#2", w, cx);
        });
        view.update(cx, |v, cx| v.search_changed(w, cx));
    })
    .unwrap();
    settle(cx, |cx| {
        cx.update(|cx| {
            !view.read(cx).search.loading
                && view.read(cx).model.rows.first().is_some_and(|r| r.id == 2)
        })
    });
    let before_source = cx.update(|cx| view.read(cx).search.generation);
    cx.update_window(window.into(), |_, w, cx| {
        w.render_frame(cx);
        w.click("only-mine", cx);
        view.update(cx, |v, cx| {
            v.receive_search(before_source, false, Ok(old.clone()), w, cx)
        });
        assert!(view.read(cx).search.generation > before_source);
    })
    .unwrap();
    settle(cx, |cx| {
        cx.update(|cx| !view.read(cx).search.loading && view.read(cx).source.human_only)
    });
    cx.update(|cx| {
        assert_eq!(
            view.read(cx).model.rows[0].id,
            2,
            "exact ID bypasses discovery source"
        )
    });
    cx.update_window(window.into(), |_, w, cx| {
        w.press("cmd-f", cx);
        w.press("cmd-a", cx);
        w.press("backspace", cx);
    })
    .unwrap();
    settle(cx, |cx| {
        cx.update(|cx| {
            view.read(cx).loaded && view.read(cx).model.rows.first().is_some_and(|r| r.id == 1)
        })
    });
    cx.update(|cx| {
        assert_eq!(view.read(cx).period, previous);
        assert_eq!(view.read(cx).model.selected, Some(1));
    });
    // Restore per-project Week and retain its origin when closing disposable search.
    cx.update_window(window.into(), |_, w, cx| {
        view.update(cx, |v, cx| {
            v.change_destination(Destination::Project("search-project".into()), w, cx)
        });
    })
    .unwrap();
    settle(cx, |cx| cx.update(|cx| view.read(cx).loaded));
    cx.update_window(window.into(), |_, w, cx| {
        w.render_frame(cx);
        w.click("scope-week", cx);
        w.click("period-back", cx);
        composer.update(cx, |i, cx| {
            i.set_value("retained on close", w, cx);
            i.set_selected_range(4..7, cx);
        });
        query(&view, "buriedneedle", w, cx);
    })
    .unwrap();
    let project_period = cx.update(|cx| view.read(cx).period.clone());
    cx.update_window(window.into(), |_, w, _| w.remove_window())
        .unwrap();
    drop(view);
    let (second, reopened) = append_tests::open(cx, services.clone());
    settle(cx, |cx| cx.update(|cx| reopened.read(cx).loaded));
    cx.update(|cx| {
        let v = reopened.read(cx);
        assert!(!v.search.active());
        assert_eq!(v.period, project_period);
        assert_eq!(v.destination, Destination::Project("search-project".into()));
        assert_eq!(v.composer.read(cx).value(), "retained on close");
        assert_eq!(v.composer.read(cx).selected_range(), 4..7);
    });
    cx.update_window(second.into(), |_, w, cx| {
        query(&reopened, "buriedneedle", w, cx);
    })
    .unwrap();
    runtime.block_on(async {
        host.db
            .writer()
            .await
            .unwrap()
            .execute(
                "UPDATE projects SET is_archived=1 WHERE id='search-project'",
                [],
            )
            .unwrap();
    });
    settle(cx, |cx| {
        cx.update(|cx| reopened.read(cx).projects.is_empty())
    });
    cx.update_window(second.into(), |_, w, cx| {
        query(&reopened, "", w, cx);
    })
    .unwrap();
    settle(cx, |cx| {
        cx.update(|cx| {
            reopened.read(cx).loaded && reopened.read(cx).destination == Destination::Home
        })
    });
    cx.update(|cx| assert_eq!(reopened.read(cx).period, Period::Day(None)));
    cx.update_window(second.into(), |_, w, _| w.remove_window())
        .unwrap();
    services.cancel_operations();
    runtime.block_on(host.shutdown());
}

#[gpui_kit::test]
fn origin_scroll_anchor_survives_new_rows_and_current_source(cx: &mut TestAppContext) {
    use gpui_kit::InputEvent as _;
    cx.executor().allow_parking();
    let root = tempfile::tempdir().unwrap();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let host = append_tests::fixture(&runtime, root.path());
    runtime.block_on(async {
        host.db.writer().await.unwrap().execute_batch("WITH RECURSIVE ids(id) AS (SELECT 10 UNION ALL SELECT id+1 FROM ids WHERE id<80) INSERT INTO notes(id,short_id,user_id,content,type,status,metadata,created_at) SELECT printf('00000000-0000-4000-8000-%012d',id),id,'append-owner','scrollmatch','normal','ready','{}',strftime('%Y-%m-%dT%H:%M:%SZ','now') FROM ids;").unwrap();
    });
    let services = append_tests::services(&host, &runtime);
    cx.update(|cx| {
        gpui_kit::init(cx);
        install_today_keys(cx);
    });
    let (window, view) = append_tests::open(cx, services.clone());
    settle(cx, |cx| {
        cx.update(|cx| view.read(cx).loaded && view.read(cx).model.rows.len() == 73)
    });
    let (anchor, offset) = cx
        .update_window(window.into(), |_, w, cx| {
            w.resize(size(px(760.), px(560.)));
            w.render_frame(cx);
            view.update(cx, |v, cx| v.select(60, w, cx));
            let center = w.find("today-notes").bounds().center();
            w.dispatch_event(
                gpui_kit::ScrollWheelEvent {
                    position: center,
                    delta: gpui_kit::ScrollDelta::Pixels(gpui_kit::point(px(0.), px(-192.))),
                    modifiers: Default::default(),
                    touch_phase: gpui_kit::TouchPhase::Moved,
                }
                .to_platform_input(),
                cx,
            );
            w.render_frame(cx);
            let v = view.read(cx);
            let offset = f32::from(v.list_scroll.0.borrow().base_handle.offset().y);
            assert!(offset < 0.);
            let anchor = v.model.rows[(-offset / 32.).floor() as usize].uuid.clone();
            (anchor, offset)
        })
        .unwrap();
    cx.update_window(window.into(), |_, w, cx| query(&view, "scrollmatch", w, cx))
        .unwrap();
    settle(cx, |cx| {
        cx.update(|cx| !view.read(cx).search.loading && view.read(cx).search.bounded)
    });
    runtime.block_on(async {
        host.db.writer().await.unwrap().execute("INSERT INTO notes(id,short_id,user_id,content,type,status,metadata,created_at) VALUES('00000000-0000-4000-8000-000000000081',81,'append-owner','scrollmatch','normal','ready','{}',strftime('%Y-%m-%dT%H:%M:%SZ','now'))",[]).unwrap();
    });
    cx.update_window(window.into(), |_, w, cx| {
        w.render_frame(cx);
        w.click("only-mine", cx);
        query(&view, "", w, cx);
    })
    .unwrap();
    settle(cx, |cx| {
        cx.update(|cx| view.read(cx).model.rows.len() == 72 && !view.read(cx).search.active())
    });
    cx.update_window(window.into(), |_, w, cx| {
        w.render_frame(cx);
        let v = view.read(cx);
        assert_eq!(v.model.selected, Some(60));
        let restored = f32::from(v.list_scroll.0.borrow().base_handle.offset().y);
        assert_eq!(restored, offset - 32.);
        assert_eq!(
            v.model.rows[(-restored / 32.).floor() as usize].uuid,
            anchor
        );
        w.remove_window();
    })
    .unwrap();
    services.cancel_operations();
    runtime.block_on(host.shutdown());
}
