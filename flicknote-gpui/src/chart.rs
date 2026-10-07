//! One native stacked Plot; Kit owns pointer tracking and tooltip placement.
use super::*;
use flicknote_sync::creation_chart::{ChartWatch, Snapshot};
use gpui_kit::base::{ColorTokens, TestSupportExt};
use gpui_kit::component::{
    Sizable,
    button::{Button, ButtonVariants},
    plot::{
        AxisLabelSide, AxisText, Grid, IntoPlot, Plot, PlotAxis,
        scale::{Scale, ScaleBand, ScaleLinear},
        shape::{Bar, Stack},
        tooltip::{PlotHover, Tooltip, TooltipState},
    },
};
use gpui_kit::{AnyElement, ElementId, Hsla, IntoElement, Pixels, Point, TextAlign, rgb};

#[derive(Clone)]
struct Segment {
    day: usize,
    group: usize,
    lower: f32,
    upper: f32,
}
#[derive(Clone)]
pub(super) struct Data {
    snapshot: Snapshot,
    segments: Vec<Segment>,
    maximum: f32,
}
impl Data {
    fn new(snapshot: Snapshot) -> Self {
        let series = Stack::new()
            .data(0..30)
            .keys(snapshot.groups.iter().map(|g| g.key.clone()))
            .value({
                let days = snapshot.days.clone();
                move |day, key| Some(*days[*day].counts.get(key).unwrap_or(&0) as f32)
            })
            .series();
        let segments: Vec<_> = series
            .into_iter()
            .flat_map(|s| {
                s.points
                    .into_iter()
                    .filter(|p| p.y1 > p.y0)
                    .map(move |p| Segment {
                        day: p.data,
                        group: s.index,
                        lower: p.y0,
                        upper: p.y1,
                    })
            })
            .collect();
        let maximum = segments.iter().map(|s| s.upper).fold(1., f32::max);
        Self {
            snapshot,
            segments,
            maximum,
        }
    }
}
#[derive(IntoPlot)]
struct CreationPlot {
    data: Arc<Data>,
    palette: ColorTokens,
    identity: String,
    hovered: bool,
}
impl CreationPlot {
    fn scales(&self, bounds: Bounds<Pixels>) -> (ScaleBand<usize>, ScaleLinear<f32>) {
        (
            ScaleBand::new(0..30, [42., (f32::from(bounds.size.width) - 8.).max(43.)])
                .padding_inner(0.3)
                .padding_outer(0.1),
            ScaleLinear::new(
                [0., self.data.maximum],
                [(f32::from(bounds.size.height) - 28.).max(9.), 8.],
            ),
        )
    }
    fn hit(&self, position: Point<Pixels>, bounds: Bounds<Pixels>) -> Option<usize> {
        let (x, y) = self.scales(bounds);
        self.data.segments.iter().position(|s| {
            let left = x.tick(&s.day).unwrap();
            let top = y.tick(&s.upper).unwrap();
            let bottom = y.tick(&s.lower).unwrap();
            let (px, py) = (f32::from(position.x), f32::from(position.y));
            px >= left && px < left + x.band_width() && py >= top && py < bottom
        })
    }
}
fn group_color(color: &str) -> Hsla {
    rgb(u32::from_str_radix(color.trim_start_matches('#'), 16).expect("normalized chart color"))
        .into()
}
impl Plot for CreationPlot {
    fn hover(&mut self, hover: Option<&PlotHover>, _: &mut Window, _: &mut App) {
        self.hovered = hover.is_some_and(PlotHover::is_hovered);
    }
    fn id(&self) -> Option<ElementId> {
        Some(self.identity.clone().into())
    }
    fn paint(&mut self, bounds: Bounds<Pixels>, window: &mut Window, cx: &mut App) {
        let (x, y) = self.scales(bounds);
        let step = (self.data.maximum / 4.).ceil().max(1.);
        let ticks: Vec<_> = (0..=(self.data.maximum / step).floor() as usize)
            .map(|i| {
                let value = step * i as f32;
                (value, y.tick(&value).unwrap())
            })
            .collect();
        let plot_bounds = Bounds::new(
            bounds.origin + gpui_kit::point(px(42.), px(0.)),
            size(bounds.size.width - px(50.), bounds.size.height - px(28.)),
        );
        Grid::new()
            .y(ticks.iter().map(|(_, v)| px(*v)))
            .stroke(self.palette.border)
            .paint(&plot_bounds, window);
        PlotAxis::new()
            .x(px(f32::from(bounds.size.height) - 28.))
            .y(px(42.))
            .y_label_side(AxisLabelSide::Start)
            .stroke(self.palette.border)
            .x_label([0, 7, 14, 21, 29].into_iter().map(|i| {
                AxisText::new(
                    self.data.snapshot.days[i].date.format("%b %-d").to_string(),
                    px(x.tick(&i).unwrap() + x.band_width() / 2.),
                    self.palette.muted_foreground,
                )
                .font_size(px(11.))
                .align(TextAlign::Center)
            }))
            .y_label(ticks.into_iter().map(|(v, t)| {
                AxisText::new(format!("{v:.0}"), px(t), self.palette.muted_foreground)
                    .font_size(px(11.))
                    .align(TextAlign::Right)
            }))
            .paint(&bounds, window, cx);
        let base = y.clone();
        let end = y.clone();
        let colors: Vec<_> = self
            .data
            .snapshot
            .groups
            .iter()
            .map(|g| group_color(&g.color))
            .collect();
        Bar::new()
            .data(self.data.segments.iter())
            .band_width(x.band_width())
            .cross(move |s| x.tick(&s.day))
            .base(move |s| base.tick(&s.lower).unwrap())
            .value(move |s| end.tick(&s.upper))
            .fill(move |s, _, _| colors[s.group])
            .paint(&bounds, window, cx);
    }
    fn tooltip_state(
        &self,
        position: Point<Pixels>,
        bounds: Bounds<Pixels>,
        _: &App,
    ) -> Option<TooltipState> {
        self.hit(position, bounds)
            .map(|index| TooltipState::new(index, position, vec![]))
    }
    fn tooltip(
        &self,
        state: &TooltipState,
        cursor: Point<Pixels>,
        bounds: Bounds<Pixels>,
        _: &mut Window,
        _: &mut App,
    ) -> Option<AnyElement> {
        if !self.hovered {
            return None;
        }
        let segment = self.data.segments.get(state.index)?;
        let group = &self.data.snapshot.groups[segment.group];
        let date = self.data.snapshot.days[segment.day]
            .date
            .format("%A, %b %-d")
            .to_string();
        let count = self.data.snapshot.days[segment.day].counts[&group.key];
        let label = format!("{date}, {}, {count}", group.name);
        Some(
            Tooltip::new(cursor, bounds.size)
                .glide(false)
                .progress(1.)
                .appearance(false)
                .max_w(px(220.))
                .px(px(8.))
                .py(px(6.))
                .bg(self.palette.surface)
                .text_color(self.palette.foreground)
                .border_1()
                .border_color(self.palette.border)
                .child(
                    div()
                        .id("chart-tooltip")
                        .aria_label(label.clone())
                        .text_size(px(12.))
                        .child(date)
                        .child(
                            div()
                                .flex()
                                .gap(px(8.))
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .truncate()
                                        .child(group.name.clone()),
                                )
                                .child(count.to_string()),
                        )
                        .test_support(),
                )
                .into_any_element(),
        )
    }
}
fn legend(data: &Data, summary: String) -> impl IntoElement + use<> {
    div()
        .id("chart-legend")
        .aria_label(summary)
        .max_h(px(96.))
        .overflow_y_scroll()
        .flex()
        .flex_wrap()
        .gap(px(12.))
        .children(data.snapshot.groups.iter().map(|g| {
            div()
                .id(gpui_kit::SharedString::from(format!(
                    "chart-group-{}",
                    g.key
                )))
                .aria_label(format!("{}: {} notes", g.name, g.total))
                .max_w(px(260.))
                .flex()
                .items_center()
                .gap(px(6.))
                .text_size(px(12.))
                .child(div().size(px(8.)).flex_shrink_0().bg(group_color(&g.color)))
                .child(div().min_w_0().truncate().child(g.name.clone()))
                .child(g.total.to_string())
                .test_support()
        }))
        .test_support()
}

impl Today {
    pub(super) fn showing_chart(&self) -> bool {
        self.destination == Destination::Charts && !self.search.active()
    }
    pub(super) fn subscribe_chart(
        &mut self,
        epoch: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.chart = None;
        self.loaded = false;
        self.watch_error = None;
        let human = self.source.human_only;
        let _entered = self.services.runtime.enter();
        let watcher = ChartWatch::start(
            self.services.db.clone(),
            self.services.user_id.clone(),
            human,
        );
        let mut receiver = watcher.receiver.clone();
        self.chart_watch = Some(watcher);
        let (send, mut prepared) = tokio::sync::watch::channel(None);
        let job = self.services.runtime.spawn(async move {
            loop {
                let value = receiver.borrow_and_update().clone();
                if let Some(result) = value {
                    send.send_replace(Some(result.map(|snapshot| Arc::new(Data::new(snapshot)))));
                }
                tokio::select! {
                    _ = send.closed() => break,
                    result = receiver.changed() => if result.is_err() { break; },
                }
            }
        });
        self.services.track(&job);
        self.watch_task = Some(cx.spawn_in(window, async move |entity, cx| {
            loop {
                if let Some(result) = prepared.borrow_and_update().clone()
                    && entity
                        .update_in(cx, |this, window, cx| {
                            this.receive_chart(epoch, human, result, window, cx)
                        })
                        .is_err()
                {
                    break;
                }
                if prepared.changed().await.is_err() {
                    break;
                }
            }
        }));
        cx.notify();
    }
    pub(super) fn receive_chart(
        &mut self,
        epoch: u64,
        human: bool,
        result: Result<Arc<Data>, String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.showing_chart() || self.watch_epoch != epoch || human != self.source.human_only {
            return;
        }
        match result {
            Ok(data) => {
                let snapshot = &data.snapshot;
                // Reject an old range whose background callback crossed local 04:00.
                if flicknote_sync::creation_chart::days(&chrono::Local::now())
                    .ok()
                    .is_none_or(|d| d[0].range.0 != snapshot.range().0)
                {
                    return;
                }
                self.projects = snapshot.projects.clone();
                if self
                    .pending_project
                    .as_ref()
                    .is_some_and(|id| self.projects.iter().any(|p| &p.id == id))
                {
                    self.select_created(window, cx);
                    return;
                }
                self.chart = Some(data);
                self.watch_error = None;
                self.loaded = true;
            }
            Err(error) => {
                self.chart = None;
                self.watch_error = Some(format!("Could not load chart: {error}"));
            }
        }
        self.close_detail(window, cx);
        self.model.rows = Arc::default();
        self.model.selected = None;
        cx.notify();
    }
    pub(super) fn render_chart(
        &self,
        p: ColorTokens,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let status = if self.watch_error.is_some() {
            "Chart unavailable"
        } else if !self.loaded {
            "Loading chart…"
        } else {
            "No notes created in this period"
        };
        let data = self.chart.clone().filter(|d| d.snapshot.total() > 0);
        div()
            .id("creation-chart")
            .aria_label("Notes created in the last 30 days")
            .flex_1()
            .min_h_0()
            .min_w_0()
            .flex()
            .flex_col()
            .p(px(12.))
            .gap(px(12.))
            .text_color(p.muted_foreground)
            .child(
                div()
                    .text_size(px(12.))
                    .child("Notes created in the last 30 days"),
            )
            .map(|d| {
                if let Some(data) = data {
                    let summary = format!("{} notes created", data.snapshot.total());
                    let identity = format!(
                        "creation-plot-{}-{}",
                        self.watch_epoch,
                        data.snapshot.range().0.timestamp()
                    );
                    d.child(
                        div()
                            .id("chart-plot")
                            .aria_label(summary.clone())
                            .flex_1()
                            .min_h(px(160.))
                            .min_w_0()
                            .child(CreationPlot {
                                data: data.clone(),
                                palette: p,
                                identity,
                                hovered: false,
                            })
                            .test_support(),
                    )
                    .child(legend(&data, summary))
                } else {
                    d.child(
                        div()
                            .id("chart-status")
                            .aria_label(status)
                            .flex_1()
                            .flex()
                            .flex_col()
                            .justify_center()
                            .items_center()
                            .gap(px(8.))
                            .child(status)
                            .children(self.watch_error.clone().map(|error| {
                                div()
                                    .text_size(px(12.))
                                    .text_color(p.destructive)
                                    .child(error)
                                    .child(
                                        Button::new("retry-chart")
                                            .label("Retry")
                                            .ghost()
                                            .small()
                                            .on_click(
                                                cx.listener(|this, _, w, cx| this.subscribe(w, cx)),
                                            ),
                                    )
                            }))
                            .test_support(),
                    )
                }
            })
            .test_support()
    }
}

#[cfg(test)]
#[path = "chart_tests.rs"]
mod tests;
