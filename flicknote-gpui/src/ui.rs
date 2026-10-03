use crate::model::Model;
use flicknote_client::dto::NoteAddInput;
use flicknote_client::{AppRequest, AppResponse};
use flicknote_sync::{app::Application as NoteApplication, spike::today::TodayWatch};
use gpui_kit::base::{Disableable, TestSupportExt};
use gpui_kit::{
    App, Bounds, Context, Entity, EntityInputHandler, KeyBinding, Menu, MenuItem, QuitMode, Role,
    Subscription, Task, TitlebarOptions, Window, WindowBounds, WindowOptions, actions,
    component::{
        button::Button,
        input::{InputEvent, Textarea, TextareaState},
    },
    div,
    prelude::*,
    px, rgb, size, uniform_list,
};
use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

struct Services {
    app: Arc<NoteApplication>,
    db: flicknote_sync::spike::Database,
    runtime: tokio::runtime::Handle,
    operations: Mutex<Vec<tokio::task::AbortHandle>>,
}
impl Services {
    fn track<T>(&self, job: &tokio::task::JoinHandle<T>) {
        let mut operations = self.operations.lock().expect("operation registry");
        operations.retain(|h| !h.is_finished());
        operations.push(job.abort_handle());
    }
    fn cancel_operations(&self) {
        for operation in self
            .operations
            .lock()
            .expect("operation registry")
            .drain(..)
        {
            operation.abort();
        }
    }
}
actions!(spike, [Quit, Reopen]);

struct Today {
    services: Arc<Services>,
    model: Model,
    composer: Entity<TextareaState>,
    detail: Entity<TextareaState>,
    error: Option<String>,
    watch_error: Option<String>,
    archive_busy: bool,
    archived: Vec<i64>,
    watch: Option<TodayWatch>,
    watch_task: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
    heartbeat: Option<Task<()>>,
    frames: u64,
    visible: bool,
    max_main_tick_ms: f64,
}

impl Today {
    fn new(services: Arc<Services>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let composer = cx.new(|cx| {
            TextareaState::new(window, cx)
                .rows(3)
                .submit_on_enter(true)
                .placeholder("Write a note… Return to save · Shift-Return for a new line")
        });
        let detail = cx.new(|cx| TextareaState::new(window, cx).rows(12));
        let subscription = cx.subscribe_in(&composer, window, |this, _, event, window, cx| {
            if matches!(
                event,
                InputEvent::PressEnter {
                    shift: false,
                    secondary: false
                }
            ) {
                this.submit(window, cx);
            }
            cx.notify();
        });
        composer.update(cx, |input, cx| input.focus(window, cx));
        let mut this = Self {
            services,
            model: Model::default(),
            composer,
            detail,
            error: None,
            watch_error: None,
            archive_busy: false,
            archived: vec![],
            watch: None,
            watch_task: None,
            _subscriptions: vec![subscription],
            heartbeat: None,
            frames: 0,
            visible: window.is_visible(),
            max_main_tick_ms: 0.,
        };
        let weak = cx.entity().downgrade();
        this._subscriptions
            .push(window.observe_window_visibility(move |visibility, _, cx| {
                let _updated = weak.update(cx, |this, _| {
                    log::info!("spike native visibility={}", visibility.is_visible());
                    this.visible = visibility.is_visible();
                });
            }));
        this.subscribe(window, cx);
        // Measure main-loop scheduling without notifying or requesting repaint.
        // Render count is diagnostic; idle time between renders is not a stall.
        this.heartbeat = Some(cx.spawn_in(window, async move |entity, cx| {
            loop {
                let tick = Instant::now();
                cx.background_executor()
                    .timer(Duration::from_millis(16))
                    .await;
                if entity
                    .update_in(cx, |this, window, _| {
                        if window.is_visible() {
                            this.max_main_tick_ms = this
                                .max_main_tick_ms
                                .max(tick.elapsed().as_secs_f64() * 1000.);
                        }
                    })
                    .is_err()
                {
                    break;
                }
            }
        }));
        this
    }

    fn subscribe(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.watch_task.take();
        self.watch.take();
        let _entered = self.services.runtime.enter();
        let watcher = TodayWatch::start(self.services.db.clone());
        let mut receiver = watcher.receiver.clone();
        self.watch = Some(watcher);
        self.watch_task = Some(cx.spawn_in(window, async move |entity, cx| {
            loop {
                let snapshot = receiver.borrow_and_update().clone();
                if let Some(result) = snapshot
                    && entity
                        .update_in(cx, |this, window, cx| {
                            match result {
                                Ok(snapshot) => {
                                    this.archived
                                        .retain(|id| snapshot.rows.iter().any(|r| r.id == *id));
                                    let rows = if this.archived.is_empty() {
                                        snapshot.rows.clone()
                                    } else {
                                        Arc::new(
                                            snapshot
                                                .rows
                                                .iter()
                                                .filter(|r| !this.archived.contains(&r.id))
                                                .cloned()
                                                .collect(),
                                        )
                                    };
                                    this.model.snapshot(rows);
                                    this.refresh_detail(window, cx);
                                    this.watch_error = None;
                                }
                                Err(error) => {
                                    this.watch_error =
                                        Some(format!("Could not load Today: {error}"))
                                }
                            }
                            cx.notify();
                        })
                        .is_err()
                {
                    break;
                }
                if receiver.changed().await.is_err() {
                    break;
                }
            }
        }));
    }

    fn composing(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        self.composer.update(cx, |input, cx| {
            input.marked_text_range(window, cx).is_some()
        })
    }

    fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.composing(window, cx) {
            return;
        }
        let text = self.composer.read(cx).value().to_string();
        if text.trim().is_empty() {
            return;
        }
        let token = self.model.accept(text.clone());
        self.composer
            .update(cx, |input, cx| input.set_value("", window, cx));
        let app = self.services.app.clone();
        let job = self.services.runtime.spawn(async move {
            app.handle(AppRequest::NoteAdd(NoteAddInput {
                content: text,
                project: None,
                interpret_as_url: false,
                draft: false,
                topics: vec![],
                created_by: None,
                created_at: None,
            }))
            .await
        });
        self.services.track(&job);
        cx.spawn_in(window, async move |entity, cx| {
            let result = match job.await {
                Ok(Ok(AppResponse::NoteCreate(note))) => Ok(note.id),
                Ok(Ok(_)) => Err("Unexpected create response".into()),
                Ok(Err(error)) => Err(error.message),
                Err(error) => Err(error.to_string()),
            };
            let _result = entity.update_in(cx, |this, window, cx| {
                if let Some(error) = this.model.ack(token, result) {
                    this.error = Some(format!("Could not save: {error}"));
                    if this.composer.read(cx).value().is_empty()
                        && !this.composing(window, cx)
                        && let Some(text) = this.model.recovery.pop()
                    {
                        this.composer
                            .update(cx, |input, cx| input.set_value(text, window, cx));
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn refresh_detail(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self
            .model
            .selected
            .and_then(|id| self.model.rows.iter().find(|r| r.id == id))
            .map(|r| r.content.clone())
            .unwrap_or_default();
        if self.detail.read(cx).value().as_ref() != text {
            self.detail
                .update(cx, |input, cx| input.set_value(text, window, cx));
        }
    }

    fn select(&mut self, id: i64, window: &mut Window, cx: &mut Context<Self>) {
        self.model.selected = Some(id);
        self.refresh_detail(window, cx);
        cx.notify();
    }

    fn archive(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.archive_busy
            || !self.composer.read(cx).value().is_empty()
            || self.composing(window, cx)
        {
            return;
        }
        let Some(id) = self.model.selected else {
            return;
        };
        self.archive_busy = true;
        let app = self.services.app.clone();
        let job = self.services.runtime.spawn(async move {
            app.handle(AppRequest::NoteArchive { id: id.to_string() })
                .await
        });
        self.services.track(&job);
        cx.spawn_in(window, async move |entity, cx| {
            let result = job.await;
            let _result = entity.update_in(cx, |this, window, cx| {
                this.archive_busy = false;
                match result {
                    Ok(Ok(AppResponse::NoteArchive(_))) => {
                        this.archived.push(id);
                        this.model.archive_success(id);
                        this.refresh_detail(window, cx);
                    }
                    Ok(Err(error)) => {
                        this.error = Some(format!("Could not archive: {}", error.message))
                    }
                    other => this.error = Some(format!("Could not archive: {other:?}")),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
}

impl Drop for Today {
    fn drop(&mut self) {
        log::info!(
            "spike window visible_renders={} max_visible_main_tick_ms={:.3}",
            self.frames,
            self.max_main_tick_ms
        );
    }
}

impl Today {
    fn render_list(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("today-notes")
            .role(Role::ListBox)
            .aria_label("Today notes")
            .flex()
            .flex_col()
            .w(px(420.))
            .min_w_0()
            .gap_1()
            .children(self.model.pending.iter().map(|p| {
                div()
                    .id(("pending", p.token))
                    .w_full()
                    .h(px(32.))
                    .flex_shrink_0()
                    .px_2()
                    .text_color(rgb(0x777777))
                    .overflow_hidden()
                    .truncate()
                    .child(format!(
                        "Saving… {}",
                        p.text.split_whitespace().collect::<Vec<_>>().join(" ")
                    ))
                    .test_support()
            }))
            .children(self.model.rows.is_empty().then(|| {
                div().p_2().child(
                    if self
                        .watch
                        .as_ref()
                        .is_some_and(|w| w.receiver.borrow().is_none())
                    {
                        "Loading Today…"
                    } else {
                        "No notes today"
                    },
                )
            }))
            .child(
                uniform_list(
                    "today-list",
                    self.model.rows.len(),
                    cx.processor(|this, range: std::ops::Range<usize>, _, cx| {
                        range
                            .map(|index| {
                                let row = &this.model.rows[index];
                                let id = row.id;
                                div()
                                    .id(("note", id as u64))
                                    .role(Role::ListBoxOption)
                                    .aria_selected(this.model.selected == Some(id))
                                    .aria_label(format!("Note {id}: {}", row.preview))
                                    .h(px(32.))
                                    .w_full()
                                    .flex()
                                    .items_center()
                                    .hover(|row| row.bg(rgb(0xeeeeec)))
                                    .when(this.model.selected == Some(id), |row| {
                                        row.bg(rgb(0xe5e7eb))
                                    })
                                    .px_2()
                                    .child(
                                        div()
                                            .min_w_0()
                                            .flex_1()
                                            .truncate()
                                            .child(row.preview.clone()),
                                    )
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.select(id, window, cx)
                                    }))
                                    .test_support()
                            })
                            .collect::<Vec<_>>()
                    }),
                )
                .w_full()
                .h_full(),
            )
            .test_support()
    }
    fn render_detail(&self, archive_disabled: bool, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(
                        Button::new("previous")
                            .label("Previous")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.model.move_selection(false);
                                this.refresh_detail(window, cx);
                                cx.notify();
                            })),
                    )
                    .child(Button::new("next").label("Next").on_click(cx.listener(
                        |this, _, window, cx| {
                            this.model.move_selection(true);
                            this.refresh_detail(window, cx);
                            cx.notify();
                        },
                    )))
                    .child(
                        Button::new("archive")
                            .label("Archive")
                            .disabled(archive_disabled)
                            .on_click(cx.listener(|this, _, window, cx| this.archive(window, cx))),
                    ),
            )
            .child(
                Textarea::new(&self.detail)
                    .accessibility_id("detail")
                    .aria_label("Note detail")
                    .readonly(true)
                    .h_full(),
            )
    }
}
impl Render for Today {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.visible && window.is_visible() {
            self.frames += 1;
        }
        self.visible = window.is_visible();
        let selected = self.model.selected;
        let archive_disabled =
            selected.is_none() || self.archive_busy || !self.composer.read(cx).value().is_empty();
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(0xf7f7f5))
            .text_color(rgb(0x262626))
            .p_4()
            .gap_3()
            .child(
                div()
                    .flex()
                    .justify_between()
                    .items_center()
                    .child(div().text_lg().child("Today"))
                    .child(div().text_sm().child("Synthetic workspace")),
            )
            .child(
                Textarea::new(&self.composer)
                    .accessibility_id("composer")
                    .aria_label("New note")
                    .h(px(88.)),
            )
            .children(
                self.error
                    .clone()
                    .or_else(|| self.watch_error.clone())
                    .map(|error| {
                        div()
                            .text_sm()
                            .text_color(rgb(0xa02c28))
                            .child(error)
                            .child(Button::new("retry-watch").label("Retry Today").on_click(
                                cx.listener(|this, _, window, cx| this.subscribe(window, cx)),
                            ))
                    }),
            )
            .children((!self.model.recovery.is_empty()).then(|| {
                Button::new("recover")
                    .label("Recover unsaved text")
                    .on_click(cx.listener(|this, _, window, cx| {
                        if this.composer.read(cx).value().is_empty()
                            && !this.composing(window, cx)
                            && let Some(text) = this.model.recovery.pop()
                        {
                            this.composer
                                .update(cx, |input, cx| input.set_value(text, window, cx));
                        }
                    }))
            }))
            .child(
                div()
                    .flex()
                    .gap_4()
                    .flex_1()
                    .min_h_0()
                    .child(self.render_list(cx))
                    .child(self.render_detail(archive_disabled, cx)),
            )
    }
}

type HostState = tokio::sync::watch::Receiver<Option<Result<Arc<Services>, String>>>;
struct Shell {
    content: Option<Entity<Today>>,
    error: Option<String>,
    _startup: Task<()>,
}
impl Shell {
    fn new(mut state: HostState, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let task = cx.spawn_in(window, async move |entity, cx| {
            loop {
                let result = state.borrow_and_update().clone();
                if let Some(result) = result {
                    let _updated = entity.update_in(cx, |this, window, cx| {
                        match result {
                            Ok(services) => {
                                this.content = Some(cx.new(|cx| Today::new(services, window, cx)))
                            }
                            Err(error) => this.error = Some(error),
                        }
                        cx.notify();
                    });
                    break;
                }
                if state.changed().await.is_err() {
                    break;
                }
            }
        });
        Self {
            content: None,
            error: None,
            _startup: task,
        }
    }
}
impl Render for Shell {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .children(self.content.clone())
            .when(self.content.is_none(), |d| {
                d.p_4().child(
                    self.error
                        .clone()
                        .unwrap_or_else(|| "Opening synthetic workspace…".into()),
                )
            })
    }
}
fn open(state: HostState, cx: &mut App) {
    if let Some(window) = cx.windows().first().copied() {
        let _result = cx.update_window(window, |_, window, _| window.activate_window());
        return;
    }
    let options = WindowOptions {
        titlebar: Some(TitlebarOptions {
            title: Some("FlickNote — Synthetic Spike".into()),
            ..Default::default()
        }),
        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
            None,
            size(px(980.), px(720.)),
            cx,
        ))),
        ..Default::default()
    };
    match gpui_kit::open_window(options, cx, |window, cx| {
        cx.new(|cx| Shell::new(state, window, cx))
    }) {
        Ok(_) => cx.activate(true),
        Err(error) => log::error!("Could not open window: {error}"),
    }
}

pub(crate) fn run() -> anyhow::Result<()> {
    let options = flicknote_spike::Options::parse();
    let runtime = tokio::runtime::Runtime::new()?;
    let handle = runtime.handle().clone();
    let (ready, state) = tokio::sync::watch::channel(None);
    let (quit, receiver) = tokio::sync::oneshot::channel();
    let task = runtime.spawn(async move {
        let host = match options.start().await {
            Ok(host) => host,
            Err(error) => {
                ready.send_replace(Some(Err(error.to_string())));
                return Err(error.to_string());
            }
        };
        ready.send_replace(Some(Ok(Arc::new(Services {
            app: host.app.clone(),
            db: host.db.clone(),
            runtime: handle,
            operations: Mutex::new(vec![]),
        }))));
        host.burst(options.burst_batches).await?;
        let app_state = ready.borrow().clone();
        host.run_until(async {
            let result = receiver.await.map_err(|e| e.to_string());
            if let Some(Ok(services)) = app_state {
                services.cancel_operations();
            }
            result
        })
        .await
    });
    let application = gpui_kit::application()
        .with_assets(gpui_kit::assets::Assets)
        .with_quit_mode(QuitMode::Explicit);
    let reopen = state.clone();
    application.on_reopen(move |cx| open(reopen.clone(), cx));
    application.run(move |cx| {
        gpui_kit::init(cx);
        cx.bind_keys([
            KeyBinding::new("cmd-q", Quit, None),
            KeyBinding::new("cmd-1", Reopen, None),
        ]);
        cx.on_action(|_: &Quit, cx| cx.quit());
        let reopen = state.clone();
        cx.on_action(move |_: &Reopen, cx| open(reopen.clone(), cx));
        cx.set_menus(vec![Menu {
            name: "FlickNote".into(),
            disabled: false,
            items: vec![
                MenuItem::action("Open Today", Reopen),
                MenuItem::separator(),
                MenuItem::action("Quit FlickNote Spike", Quit),
            ],
        }]);
        let mut quit = Some(quit);
        cx.on_app_quit(move |_| {
            if let Some(quit) = quit.take() {
                let _result = quit.send(());
            }
            async {}
        })
        .detach();
        open(state, cx);
    });
    // Native event loop has exited; wait for the existing shutdown coordinator.
    runtime.block_on(task)?.map_err(anyhow::Error::msg)?;
    Ok(())
}

#[cfg(test)]
#[path = "ui_tests.rs"]
mod tests;
