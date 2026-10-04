use crate::model::Model;
use crate::workspace::{MIN_HEIGHT, MIN_WIDTH, apply_theme};
use flicknote_client::dto::NoteAddInput;
use flicknote_client::{AppRequest, AppResponse};
use flicknote_sync::{app::Application as NoteApplication, spike::today::TodayWatch};
use gpui_kit::{
    App, Bounds, Context, Entity, EntityInputHandler, KeyBinding, Menu, MenuItem, QuitMode, Role,
    Subscription, Task, TitlebarOptions, Window, WindowBounds, WindowOptions, actions,
    component::{
        Theme,
        input::{InputEvent, Textarea, TextareaState},
    },
    div,
    prelude::*,
    px, size,
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
actions!(
    spike,
    [
        Quit,
        Reopen,
        SystemTheme,
        LightTheme,
        DarkTheme,
        NextNote,
        PreviousNote,
        ArchiveNote
    ]
);

fn install_today_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("alt-j", NextNote, Some("Today")),
        KeyBinding::new("alt-k", PreviousNote, Some("Today")),
        KeyBinding::new("alt-a", ArchiveNote, Some("Today")),
        KeyBinding::new("escape", gpui_kit::base::input::Escape, Some("Today")),
    ]);
}

struct Today {
    services: Arc<Services>,
    model: Model,
    composer: Entity<TextareaState>,
    detail: Entity<TextareaState>,
    detail_open: bool,
    list_scroll: gpui_kit::UniformListScrollHandle,
    escape_composing: bool,
    enter_composing: bool,
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
                .auto_grow(1, 6)
                .submit_on_enter(true)
                .placeholder("Create a new note")
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
                let composing_at_enter = std::mem::take(&mut this.enter_composing);
                if !composing_at_enter {
                    if this.composer.read(cx).value().is_empty() {
                        this.open_selected(window, cx);
                    } else {
                        this.submit(window, cx);
                    }
                }
            }
            cx.notify();
        });
        composer.update(cx, |input, cx| input.focus(window, cx));
        let mut this = Self {
            services,
            model: Model::default(),
            composer,
            detail,
            detail_open: false,
            list_scroll: gpui_kit::UniformListScrollHandle::new(),
            escape_composing: false,
            enter_composing: false,
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
        if self.detail_open && self.model.selected.is_none() {
            self.close_detail(window, cx);
        }
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
        self.detail_open = true;
        self.refresh_detail(window, cx);
        cx.notify();
    }

    fn shortcuts_blocked(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        !self.composer.read(cx).value().is_empty() || self.composing(window, cx)
    }

    fn open_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.shortcuts_blocked(window, cx) {
            return;
        }
        if let Some(id) = self.model.selected {
            self.select(id, window, cx);
        }
    }

    fn navigate(&mut self, next: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.shortcuts_blocked(window, cx) || self.model.rows.is_empty() {
            return;
        }
        let index = self
            .model
            .selected
            .and_then(|id| self.model.rows.iter().position(|r| r.id == id));
        let index = match index {
            None => 0,
            Some(index) if next => (index + 1).min(self.model.rows.len() - 1),
            Some(index) => index.saturating_sub(1),
        };
        self.model.selected = Some(self.model.rows[index].id);
        if self.detail_open {
            self.refresh_detail(window, cx);
        }
        // A semantic key action reveals the row above the composer; snapshots never scroll.
        self.list_scroll
            .scroll_to_item_strict(index, gpui_kit::ScrollStrategy::Center);
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

#[path = "presentation.rs"]
mod presentation;

type HostState = tokio::sync::watch::Receiver<Option<Result<Arc<Services>, String>>>;
struct Shell {
    _appearance: Subscription,
    content: Option<Entity<Today>>,
    error: Option<String>,
    _startup: Task<()>,
}
impl Shell {
    fn new(
        mut state: HostState,
        system: std::rc::Rc<std::cell::Cell<bool>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let appearance = window.observe_window_appearance(move |window, cx| {
            if system.get() {
                apply_theme(window.appearance(), cx);
            }
        });
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
            _appearance: appearance,
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
fn install_appearance_menu(system: &std::rc::Rc<std::cell::Cell<bool>>, cx: &mut App) {
    use gpui_kit::component::ThemeMode;
    apply_theme(cx.window_appearance(), cx);
    let flag = system.clone();
    cx.on_action(move |_: &SystemTheme, cx| {
        flag.set(true);
        apply_theme(cx.window_appearance(), cx);
    });
    let flag = system.clone();
    cx.on_action(move |_: &LightTheme, cx| {
        flag.set(false);
        apply_theme(ThemeMode::Light, cx);
    });
    let flag = system.clone();
    cx.on_action(move |_: &DarkTheme, cx| {
        flag.set(false);
        apply_theme(ThemeMode::Dark, cx);
    });
}

fn open(state: HostState, system: std::rc::Rc<std::cell::Cell<bool>>, cx: &mut App) {
    if let Some(window) = cx.windows().first().copied() {
        let _result = cx.update_window(window, |_, window, _| window.activate_window());
        return;
    }
    let options = WindowOptions {
        window_min_size: Some(size(px(MIN_WIDTH), px(MIN_HEIGHT))),
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
        cx.new(|cx| Shell::new(state, system, window, cx))
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
        .with_assets(crate::assets::TodayAssets)
        .with_quit_mode(QuitMode::Explicit);
    let reopen = state.clone();
    let system = std::rc::Rc::new(std::cell::Cell::new(true));
    let reopen_system = system.clone();
    application.on_reopen(move |cx| open(reopen.clone(), reopen_system.clone(), cx));
    application.run(move |cx| {
        gpui_kit::init(cx);
        install_appearance_menu(&system, cx);
        install_today_keys(cx);
        cx.bind_keys([
            KeyBinding::new("cmd-q", Quit, None),
            KeyBinding::new("cmd-1", Reopen, None),
        ]);
        cx.on_action(|_: &Quit, cx| cx.quit());
        let reopen = state.clone();
        let reopen_system = system.clone();
        cx.on_action(move |_: &Reopen, cx| open(reopen.clone(), reopen_system.clone(), cx));
        cx.set_menus(vec![Menu {
            name: "FlickNote".into(),
            disabled: false,
            items: vec![
                MenuItem::action("Open Today", Reopen),
                MenuItem::action("Next note", NextNote),
                MenuItem::action("Previous note", PreviousNote),
                MenuItem::action("Archive selected note", ArchiveNote),
                MenuItem::separator(),
                MenuItem::action("Appearance: System", SystemTheme),
                MenuItem::action("Appearance: Light", LightTheme),
                MenuItem::action("Appearance: Dark", DarkTheme),
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
        open(state, system, cx);
    });
    // Native event loop has exited; wait for the existing shutdown coordinator.
    runtime.block_on(task)?.map_err(anyhow::Error::msg)?;
    Ok(())
}

#[cfg(test)]
#[path = "ui_tests.rs"]
mod tests;
