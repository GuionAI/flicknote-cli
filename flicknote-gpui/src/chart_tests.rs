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

fn move_pointer(w: &mut Window, position: Point<Pixels>, cx: &mut App) {
    w.dispatch_event(
        MouseMoveEvent {
            position,
            pressed_button: None,
            modifiers: Default::default(),
        }
        .to_platform_input(),
        cx,
    );
    w.render_frame(cx);
}
fn label_for(data: &Data, segment: &Segment) -> String {
    let group = &data.snapshot.groups[segment.group];
    let day = &data.snapshot.days[segment.day];
    format!(
        "{}, {}, {}",
        day.date.format("%A, %b %-d"),
        group.name,
        day.counts[&group.key]
    )
}

fn hover_last(w: &mut Window, data: &Arc<Data>, cx: &mut App) {
    let plot = w.find("chart-plot").bounds();
    let primitive = CreationPlot {
        data: data.clone(),
        palette: Theme::global(cx).color_tokens(),
        identity: "probe".into(),
        hovered: false,
    };
    let (x, y) = primitive.scales(plot);
    let s = data.segments.last().unwrap();
    move_pointer(
        w,
        plot.origin
            + gpui_kit::point(
                px(x.tick(&s.day).unwrap() + x.band_width() / 2.),
                px(68. + (y.tick(&s.lower).unwrap() + y.tick(&s.upper).unwrap()) / 2.),
            ),
        cx,
    );
}

fn bars(w: &Window, plot: Bounds<Pixels>) -> Vec<Bounds<gpui_kit::ScaledPixels>> {
    let swatch_width = px(7.).scale(w.scale_factor());
    let plot = plot.scale(w.scale_factor());
    w.painted_quads()
        .into_iter()
        .filter(|q| {
            q.bounds.size.width > swatch_width
                && q.bounds.origin.x >= plot.origin.x
                && q.bounds.right() <= plot.right()
                && q.bounds.origin.y >= plot.origin.y
                && q.bounds.bottom() <= plot.bottom()
                && ["#4187D9", "#E47A36"]
                    .iter()
                    .any(|c| q.background == gpui_kit::solid_background(group_color(c)))
        })
        .map(|q| q.bounds)
        .collect()
}
fn assert_fill(w: &Window, bounds: Bounds<Pixels>, color: Hsla) {
    assert!(w.painted_quads().iter().any(|q| {
        q.bounds == bounds.scale(w.scale_factor())
            && q.background == gpui_kit::solid_background(color)
    }));
}

fn assert_card(
    w: &Window,
    cx: &App,
    plot: Bounds<Pixels>,
    tooltip: Bounds<Pixels>,
    center: Pixels,
    color: Hsla,
) {
    let expected_left = (center - px(95.)).clamp(plot.origin.x + px(8.), plot.right() - px(198.));
    assert!((tooltip.origin.x - expected_left).abs() <= px(0.5));
    assert!(tooltip.origin.x >= plot.origin.x + px(8.));
    assert!(tooltip.right() <= plot.right() - px(8.));
    assert_eq!(tooltip.size.width, px(190.), "settled card width");
    assert!(
        tooltip.bottom() <= plot.origin.y + px(68.),
        "card must remain above bars in reserved band"
    );
    assert!((tooltip.center().y - plot.origin.y - px(34.)).abs() <= px(0.5));
    let guide = w.find("chart-guide").bounds();
    assert!((guide.center().x - center).abs() <= px(0.5));
    assert_eq!(guide.size.width, px(1.));
    assert_eq!(guide.origin.y, plot.origin.y + px(68.));
    assert_eq!(guide.bottom(), plot.bottom() - px(28.));
    let palette = Theme::global(cx).color_tokens();
    let mut guide_color = palette.muted_foreground;
    guide_color.a *= 0.35;
    assert_fill(w, guide, guide_color);
    assert_fill(w, w.find("chart-tooltip-color").bounds(), color);
    assert_eq!(
        w.find("chart-tooltip-color").bounds().size,
        size(px(7.), px(7.))
    );
    let name = w.find("chart-tooltip-project").bounds();
    let count = w.find("chart-tooltip-count").bounds();
    assert!(name.right() + px(12.) <= count.origin.x);
    assert!(count.right() <= tooltip.right() - px(9.));
}

fn check_hover(w: &mut Window, cx: &mut App, data: &Arc<Data>, plot: Bounds<Pixels>) {
    let primitive = CreationPlot {
        data: data.clone(),
        palette: Theme::global(cx).color_tokens(),
        identity: "probe".into(),
        hovered: false,
    };
    let (x, y) = primitive.scales(plot);
    move_pointer(w, plot.origin + gpui_kit::point(px(20.), px(12.)), cx);
    let unhovered_bars = bars(w, plot);
    assert!(!unhovered_bars.is_empty());
    assert!(
        unhovered_bars
            .iter()
            .all(|b| b.origin.y >= (plot.origin.y + px(68.)).scale(w.scale_factor()))
    );
    for segment in data.segments.iter() {
        let relative = gpui_kit::point(
            px(x.tick(&segment.day).unwrap() + x.band_width() / 2.),
            px(68. + (y.tick(&segment.lower).unwrap() + y.tick(&segment.upper).unwrap()) / 2.),
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
        let center = plot.origin.x + px(x.tick(&segment.day).unwrap() + x.band_width() / 2.);
        assert_card(w, cx, plot, tooltip, center, group_color(&group.color));
        assert_eq!(w.find("chart-plot").bounds(), plot);
        assert_eq!(bars(w, plot), unhovered_bars);
        // The guide overlays this pointer, yet never occludes Kit's hitbox.
        move_pointer(w, plot.origin + relative, cx);
        assert_eq!(w.find("chart-tooltip").bounds(), tooltip);
        move_pointer(
            w,
            plot.origin + relative + gpui_kit::point(px(x.band_width() / 4.), px(1.)),
            cx,
        );
        assert_eq!(w.find("chart-tooltip").bounds(), tooltip);
        assert_eq!(
            w.find("chart-tooltip").label(),
            Some(label_for(data, segment).as_str())
        );
        // Entering the card, gap, axis or zero day clears immediately.
        for point in [
            tooltip.center(),
            plot.origin + gpui_kit::point(px(20.), px(100.)),
            plot.origin + gpui_kit::point(px(x.tick(&segment.day).unwrap() - 1.), relative.y),
            plot.origin + gpui_kit::point(px(x.tick(&1).unwrap()), px(100.)),
            plot.origin - gpui_kit::point(px(1.), px(1.)),
        ] {
            move_pointer(w, point, cx);
            assert!(w.try_find("chart-tooltip").is_none());
            assert!(w.try_find("chart-guide").is_none());
            assert_eq!(w.find("chart-plot").bounds(), plot);
            assert_eq!(bars(w, plot), unhovered_bars);
        }
    }
}

#[gpui_kit::test]
#[allow(clippy::too_many_lines)] // One pointer path verifies each stacked segment and bounded theme geometry.
fn native_kit_stacked_plot_pointer_legend_geometry_and_empty_scope(cx: &mut TestAppContext) {
    let (_root, runtime, host, services) = setup(cx);
    // Real owned rows populate both edges and an unclamped middle date.
    runtime.block_on(async {
        let days = flicknote_sync::creation_chart::days(&chrono::Local::now()).unwrap();
        let writer = host.db.writer().await.unwrap();
        for (id, day) in [(4, 0), (5, 14)] {
            let created = (days[day].range.0 + chrono::Duration::hours(1)).to_rfc3339();
            writer.execute("INSERT INTO notes(id,short_id,user_id,status,created_at,project_id,metadata) VALUES(?,?,'append-owner','ready',?,'chart-p1','{}')", [format!("edge-{id}"), id.to_string(), created]).unwrap();
        }
    });
    *services.destination.lock().unwrap() = Destination::Charts;
    let (window, view) = append_tests::open(cx, services);
    loaded(cx, &view);
    assert_eq!(
        cx.update(|cx| view.read(cx).chart.as_ref().unwrap().snapshot.total()),
        5
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
                check_hover(w, cx, &data, plot);
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
    cx.update_window(window.into(), |_, w, cx| {
        let original = view.read(cx).chart.clone().unwrap();
        hover_last(w, &original, cx);
        let mut replacement = original.snapshot.clone();
        let last = original.segments.last().unwrap();
        replacement.groups[last.group].name = "Updated project".into();
        let epoch = view.read(cx).watch_epoch;
        let human = view.read(cx).source.human_only;
        view.update(cx, |this, cx| {
            this.receive_chart(epoch, human, Ok(Arc::new(Data::new(replacement))), w, cx)
        });
        w.render_frame(cx);
        assert!(w.try_find("chart-tooltip").is_none());
        assert!(w.try_find("chart-guide").is_none());
        let current = view.read(cx).chart.clone().unwrap();
        hover_last(w, &current, cx);
        assert_eq!(
            w.find("chart-tooltip").label(),
            Some(label_for(&current, current.segments.last().unwrap()).as_str())
        );
    })
    .unwrap();
    let stale = cx.update(|cx| view.read(cx).chart.as_ref().unwrap().snapshot.clone());
    let epoch = cx.update(|cx| view.read(cx).watch_epoch);
    // Source changes replace the actual authoritative counts, not legend-only decoration.
    cx.update_window(window.into(), |_, w, cx| {
        w.click("only-mine", cx);
        assert!(view.read(cx).chart.is_none());
        w.render_frame(cx);
        assert!(w.try_find("chart-tooltip").is_none());
        assert!(w.try_find("chart-guide").is_none());
    })
    .unwrap();
    loaded(cx, &view);
    assert_eq!(
        cx.update(|cx| view.read(cx).chart.as_ref().unwrap().snapshot.total()),
        4
    );
    cx.update_window(window.into(), |_, w, cx| {
        let mut stale = stale.clone();
        stale.groups[0].total = 999;
        view.update(cx, |this, cx| {
            this.receive_chart(epoch, false, Ok(Arc::new(Data::new(stale))), w, cx)
        });
        assert_eq!(view.read(cx).chart.as_ref().unwrap().snapshot.total(), 4);
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
