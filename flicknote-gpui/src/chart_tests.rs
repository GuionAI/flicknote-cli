//! Actual owned host projections and Kit Plot pointer dispatch, without native launch.
use super::super::{append_tests, tests::settle};
use super::*;
use gpui_kit::{InputEvent as _, MouseMoveEvent, TestAppContext, test::TestWindowExt};
fn loaded(cx: &mut TestAppContext, view: &Entity<Today>) {
    settle(cx, |cx| {
        cx.update(|cx| view.read(cx).loaded && view.read(cx).chart.is_some())
    });
}
fn setup(
    cx: &mut TestAppContext,
) -> (
    tempfile::TempDir,
    tokio::runtime::Runtime,
    flicknote_sync::LocalHost,
    Arc<Services>,
) {
    cx.executor().allow_parking();
    let root = tempfile::tempdir().unwrap();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let host = append_tests::fixture(&runtime, root.path());
    runtime.block_on(async {
        let writer=host.db.writer().await.unwrap();
        for (id,name,color) in [("chart-p1","Alpha","#4187D9"),("chart-p2","Beta with a long project name to constrain the tooltip", "#E47A36")] {
            writer.execute("INSERT INTO projects(id,user_id,name,color,is_archived) VALUES(?,'append-owner',?,?,0)",[id,name,color]).unwrap();
        }
        writer.execute("UPDATE notes SET project_id=CASE WHEN short_id=1 THEN 'chart-p1' ELSE 'chart-p2' END, metadata='{}'",[]).unwrap();
        writer.execute("INSERT INTO notes(id,short_id,user_id,status,created_at,project_id,metadata) VALUES('chart-draft',3,'append-owner','draft',?,'chart-p1','{\"created_by_ai\":true}')",[chrono::Utc::now().to_rfc3339()]).unwrap();
    });
    let services = append_tests::services(&host, &runtime);
    cx.update(|cx| {
        gpui_kit::init(cx);
        install_today_keys(cx);
    });
    (root, runtime, host, services)
}

#[gpui_kit::test]
#[allow(clippy::too_many_lines)] // One pointer path verifies each stacked segment and bounded theme geometry.
fn native_kit_stacked_plot_pointer_legend_geometry_and_empty_scope(cx: &mut TestAppContext) {
    let (_root, runtime, host, services) = setup(cx);
    *services.destination.lock().unwrap() = Destination::Charts;
    let (window, view) = append_tests::open(cx, services);
    loaded(cx, &view);
    assert_eq!(
        cx.update(|cx| view.read(cx).chart.as_ref().unwrap().snapshot.total()),
        3
    );
    for theme in [
        gpui_kit::component::ThemeMode::Light,
        gpui_kit::component::ThemeMode::Dark,
    ] {
        for width in [760., 980.] {
            cx.update_window(window.into(), |_, w, cx| {
                crate::workspace::apply_theme(theme, cx);
                w.resize(size(px(width), px(560.)));
                w.render_frame(cx);
                let pane = w.find("main-canvas").bounds();
                let plot = w.find("chart-plot").bounds();
                let legend = w.find("chart-legend").bounds();
                let composer = w.find("composer-surface").bounds();
                assert!(plot.size.width > px(450.) && plot.size.height >= px(160.));
                assert!(plot.origin.x >= pane.origin.x && plot.right() <= pane.right());
                assert!(legend.bottom() <= composer.origin.y && legend.size.height <= px(96.));
                assert!(view.read(cx).model.rows.is_empty());
                assert!(view.read(cx).model.selected.is_none() && view.read(cx).detail.is_none());
                let data = view.read(cx).chart.clone().unwrap();
                let primitive = CreationPlot {
                    data: data.clone(),
                    palette: Theme::global(cx).color_tokens(),
                    identity: "probe".into(),
                    hovered: false,
                };
                let (x, y) = primitive.scales(plot);
                for segment in data.segments.iter() {
                    let relative = gpui_kit::point(
                        px(x.tick(&segment.day).unwrap() + x.band_width() / 2.),
                        px(
                            (y.tick(&segment.lower).unwrap() + y.tick(&segment.upper).unwrap())
                                / 2.,
                        ),
                    );
                    w.dispatch_event(
                        MouseMoveEvent {
                            position: plot.origin + relative,
                            pressed_button: None,
                            modifiers: Default::default(),
                        }
                        .to_platform_input(),
                        cx,
                    );
                    w.render_frame(cx);
                    let group = &data.snapshot.groups[segment.group];
                    let date = data.snapshot.days[segment.day]
                        .date
                        .format("%A, %b %-d")
                        .to_string();
                    let count = data.snapshot.days[segment.day].counts[&group.key];
                    let tooltip = w.find("chart-tooltip");
                    assert_eq!(
                        tooltip.label(),
                        Some(format!("{date}, {}, {count}", group.name).as_str())
                    );
                    let tooltip = tooltip.bounds();
                    assert!(tooltip.origin.x >= plot.origin.x && tooltip.right() <= plot.right());
                    assert!(tooltip.origin.y >= plot.origin.y && tooltip.bottom() <= plot.bottom());
                }
                // A zero day and the axis resolve no segment, rather than a nearest-day total.
                w.dispatch_event(
                    MouseMoveEvent {
                        position: plot.origin + gpui_kit::point(px(60.), px(12.)),
                        pressed_button: None,
                        modifiers: Default::default(),
                    }
                    .to_platform_input(),
                    cx,
                );
                w.render_frame(cx);
                assert!(w.try_find("chart-tooltip").is_none());
            })
            .unwrap();
        }
    }
    let stale = cx.update(|cx| view.read(cx).chart.as_ref().unwrap().snapshot.clone());
    let epoch = cx.update(|cx| view.read(cx).watch_epoch);
    // Source changes replace the actual authoritative counts, not legend-only decoration.
    cx.update_window(window.into(), |_, w, cx| {
        w.click("only-mine", cx);
        assert!(view.read(cx).chart.is_none());
    })
    .unwrap();
    loaded(cx, &view);
    assert_eq!(
        cx.update(|cx| view.read(cx).chart.as_ref().unwrap().snapshot.total()),
        2
    );
    cx.update_window(window.into(), |_, w, cx| {
        let mut stale = stale.clone();
        stale.groups[0].total = 999;
        view.update(cx, |this, cx| {
            this.receive_chart(epoch, false, Ok(Arc::new(Data::new(stale))), w, cx)
        });
        assert_eq!(view.read(cx).chart.as_ref().unwrap().snapshot.total(), 2);
    })
    .unwrap();
    runtime
        .block_on(host.db.writer())
        .unwrap()
        .execute("UPDATE notes SET deleted_at='2026-01-01'", [])
        .unwrap();
    settle(cx, |cx| {
        cx.update(|cx| {
            view.read(cx)
                .chart
                .as_ref()
                .is_some_and(|d| d.snapshot.total() == 0)
        })
    });
    cx.update_window(window.into(), |_, w, cx| {
        w.render_frame(cx);
        assert_eq!(
            w.find("chart-status").label(),
            Some("No notes created in this period")
        );
        assert!(w.try_find("chart-plot").is_none());
        w.remove_window();
    })
    .unwrap();
    cx.run_until_parked();
    drop(view);
    runtime.block_on(host.shutdown());
}

#[gpui_kit::test]
#[allow(clippy::too_many_lines)] // A retained journey exercises rail/search/capture and stale watch rejection.
fn chart_rail_traversal_no_note_actions_search_origin_draft_and_reopen(cx: &mut TestAppContext) {
    let (_root, runtime, host, services) = setup(cx);
    let (window, view) = append_tests::open(cx, services.clone());
    settle(cx, |cx| cx.update(|cx| view.read(cx).loaded));
    let composer = cx.update(|cx| view.read(cx).composer.clone());
    cx.update_window(window.into(), |_, w, cx| {
        view.update(cx, |this, cx| this.select(2, w, cx));
        composer.update(cx, |i, cx| {
            i.set_value("retained draft", w, cx);
            i.set_selected_range(2..7, cx);
        });
        w.render_frame(cx);
        w.click("charts", cx);
        assert_eq!(view.read(cx).destination, Destination::Charts);
        assert!(!view.read(cx).detail_open);
        assert_eq!(composer.read(cx).value(), "retained draft");
        assert_eq!(composer.read(cx).selected_range(), 2..7);
        assert!(view.read(cx).model.rows.is_empty());
    })
    .unwrap();
    loaded(cx, &view);
    let old_epoch = cx.update(|cx| view.read(cx).watch_epoch);
    let mut old_snapshot = cx.update(|cx| view.read(cx).chart.as_ref().unwrap().snapshot.clone());
    old_snapshot.groups[0].total = 999;
    cx.update_window(window.into(), |_, w, cx| {
        composer.update(cx, |i, cx| i.set_value("", w, cx));
        w.render_frame(cx);
        for key in [
            "alt-j",
            "alt-k",
            "alt-a",
            "alt-h",
            "alt-l",
            "alt-left",
            "alt-right",
            "enter",
        ] {
            w.press(key, cx);
        }
        assert!(view.read(cx).model.selected.is_none());
        assert!(!view.read(cx).detail_open);
        assert!(
            !view
                .read(cx)
                .note_busy("00000000-0000-4000-8000-000000000002")
        );
        w.press("alt-down", cx);
        assert_eq!(view.read(cx).destination, Destination::Charts);
        w.press("alt-up", cx);
        assert_eq!(view.read(cx).destination, Destination::Archive);
    })
    .unwrap();
    settle(cx, |cx| cx.update(|cx| view.read(cx).loaded));
    cx.update_window(window.into(), |_, w, cx| {
        w.press("alt-down", cx);
        assert_eq!(view.read(cx).destination, Destination::Charts);
    })
    .unwrap();
    loaded(cx, &view);
    cx.update_window(window.into(), |_, w, cx| {
        w.press("cmd-f", cx);
        assert!(view.read(cx).search_focused(w, cx));
        let input = view.read(cx).search_input.clone();
        input.update(cx, |i, cx| i.set_value("Original", w, cx));
    })
    .unwrap();
    settle(cx, |cx| {
        cx.update(|cx| view.read(cx).search.active() && !view.read(cx).search.loading)
    });
    assert!(cx.update(|cx| view.read(cx).chart_watch.is_none()));
    cx.update_window(window.into(), |_, w, cx| {
        view.update(cx, |this, cx| {
            this.receive_chart(
                old_epoch,
                false,
                Ok(Arc::new(Data::new(old_snapshot.clone()))),
                w,
                cx,
            )
        });
        assert!(view.read(cx).chart.is_none());
    })
    .unwrap();
    cx.update_window(window.into(), |_, w, cx| {
        w.press("escape", cx);
        assert_eq!(view.read(cx).destination, Destination::Charts);
        assert!(!view.read(cx).search.active());
        assert!(view.read(cx).model.rows.is_empty());
    })
    .unwrap();
    loaded(cx, &view);
    cx.update_window(window.into(), |_, w, cx| {
        view.update(cx, |this, cx| {
            this.receive_chart(
                old_epoch,
                false,
                Ok(Arc::new(Data::new(old_snapshot.clone()))),
                w,
                cx,
            )
        });
        assert_eq!(view.read(cx).chart.as_ref().unwrap().snapshot.total(), 3);
        // Explicit retry recovers from an error on the same active scope.
        let epoch = view.read(cx).watch_epoch;
        view.update(cx, |this, cx| {
            this.receive_chart(epoch, false, Err("owned injected failure".into()), w, cx)
        });
        w.render_frame(cx);
        assert!(w.try_find("chart-plot").is_none());
        w.click("retry-chart", cx);
    })
    .unwrap();
    loaded(cx, &view);
    cx.update_window(window.into(), |_, w, cx| {
        composer.update(cx, |i, cx| {
            i.set_value("after close", w, cx);
            i.set_selected_range(1..4, cx);
        });
        w.remove_window();
    })
    .unwrap();
    cx.run_until_parked();
    drop(view);
    let (reopened, view) = append_tests::open(cx, services.clone());
    loaded(cx, &view);
    cx.update_window(reopened.into(), |_, w, cx| {
        assert_eq!(view.read(cx).destination, Destination::Charts);
        assert_eq!(view.read(cx).composer.read(cx).value(), "after close");
        assert_eq!(view.read(cx).composer.read(cx).selected_range(), 1..4);
        assert!(view.read(cx).model.rows.is_empty());
        w.remove_window();
    })
    .unwrap();
    cx.run_until_parked();
    drop(view);
    runtime.block_on(host.shutdown());
}
