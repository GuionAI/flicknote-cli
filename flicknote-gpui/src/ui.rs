use crate::model::{Capture, Model};
use crate::workspace::{MIN_HEIGHT, MIN_WIDTH, apply_theme};
use flicknote_client::dto::NoteAddInput;
use flicknote_client::{AppRequest, AppResponse};
use flicknote_sync::{
    app::Application as NoteApplication,
    today::{Destination, TodayWatch},
};
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

pub(super) struct Services {
    pub(super) app: Arc<NoteApplication>,
    pub(super) db: flicknote_sync::PowerSyncDatabase,
    pub(super) runtime: tokio::runtime::Handle,
    pub(super) operations: Mutex<Vec<tokio::task::AbortHandle>>,
    pub(super) user_id: String,
    pub(super) real_account: bool,
    pub(super) first_sync: Mutex<crate::sync_progress::FirstSync>,
    pub(super) destination: Mutex<Destination>,
    pub(super) capture: Arc<Mutex<Capture>>,
    pub(super) draft: Mutex<(String, std::ops::Range<usize>)>,
    pub(super) capture_changed: tokio::sync::watch::Sender<()>,
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
        ArchiveNote,
        Project2,
        Project3,
        Project4,
        Project5,
        Project6,
        Project7,
        Project8,
        Project9,
        NextDestination,
        PreviousDestination
    ]
);

fn install_today_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("cmd-1", Reopen, Some("Today")),
        KeyBinding::new("cmd-2", Project2, Some("Today")),
        KeyBinding::new("cmd-3", Project3, Some("Today")),
        KeyBinding::new("cmd-4", Project4, Some("Today")),
        KeyBinding::new("cmd-5", Project5, Some("Today")),
        KeyBinding::new("cmd-6", Project6, Some("Today")),
        KeyBinding::new("cmd-7", Project7, Some("Today")),
        KeyBinding::new("cmd-8", Project8, Some("Today")),
        KeyBinding::new("cmd-9", Project9, Some("Today")),
        KeyBinding::new(
            "alt-up",
            PreviousDestination,
            Some("Today && destination_navigation"),
        ),
        KeyBinding::new(
            "alt-down",
            NextDestination,
            Some("Today && destination_navigation"),
        ),
        KeyBinding::new("alt-j", NextNote, Some("Today")),
        KeyBinding::new("alt-k", PreviousNote, Some("Today")),
        KeyBinding::new("alt-a", ArchiveNote, Some("Today")),
        KeyBinding::new("escape", gpui_kit::base::input::Escape, Some("Today")),
    ]);
}

struct Today {
    services: Arc<Services>,
    destination: Destination,
    loaded: bool,
    model: Model,
    composer: Entity<TextareaState>,
    detail: Entity<TextareaState>,
    detail_open: bool,
    list_scroll: gpui_kit::UniformListScrollHandle,
    escape_composing: bool,
    enter_composing: bool,
    error: Option<String>,
    watch_error: Option<String>,
    sync_message: Option<String>,
    sync_progress: Option<crate::sync_progress::Progress>,
    first_synced: bool,
    projects: Arc<Vec<flicknote_sync::today::ProjectContext>>,
    status_task: Option<Task<()>>,
    capture_task: Option<Task<()>>,
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
        let (text, caret) = services.draft.lock().expect("composer draft").clone();
        composer.update(cx, |input, cx| {
            input.set_value(text, window, cx);
            input.set_selected_range(caret, cx);
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
        let destination = services
            .destination
            .lock()
            .expect("window destination")
            .clone();
        let mut this = Self {
            destination,
            loaded: false,
            model: Model {
                capture: services.capture.clone(),
                ..Model::default()
            },
            services,
            composer,
            detail,
            detail_open: false,
            list_scroll: gpui_kit::UniformListScrollHandle::new(),
            escape_composing: false,
            enter_composing: false,
            error: None,
            watch_error: None,
            sync_message: None,
            sync_progress: None,
            first_synced: false,
            projects: Arc::default(),
            status_task: None,
            capture_task: None,
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
                    log::info!("trial native visibility={}", visibility.is_visible());
                    this.visible = visibility.is_visible();
                });
            }));
        this.subscribe_capture(window, cx);
        this.subscribe(window, cx);
        this.observe_main_loop(window, cx);
        this.subscribe_status(window, cx);
        this
    }

    fn observe_main_loop(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // Measure main-loop scheduling without notifying or requesting repaint.
        // Render count is diagnostic; idle time between renders is not a stall.
        self.heartbeat = Some(cx.spawn_in(window, async move |entity, cx| {
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
    }

    fn subscribe(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.watch_task.take();
        self.watch.take();
        let _entered = self.services.runtime.enter();
        let destination = self.destination.clone();
        let watcher = TodayWatch::start_destination(
            self.services.db.clone(),
            self.services.user_id.clone(),
            destination.clone(),
        );
        let mut receiver = watcher.receiver.clone();
        self.watch = Some(watcher);
        self.watch_task = Some(cx.spawn_in(window, async move |entity, cx| {
            loop {
                let snapshot = receiver.borrow_and_update().clone();
                if let Some(result) = snapshot
                    && entity
                        .update_in(cx, |this, window, cx| {
                            if this.destination != destination {
                                return;
                            }
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
                                    this.projects = snapshot.projects;
                                    if let Destination::Project(id) = &this.destination
                                        && !this.projects.iter().any(|p| &p.id == id)
                                    {
                                        this.set_destination(Destination::Home, window, cx);
                                        return;
                                    }
                                    this.loaded = true;
                                    this.model.snapshot(rows);
                                    this.refresh_detail(window, cx);
                                    this.watch_error = None;
                                }
                                Err(error) => {
                                    this.watch_error =
                                        Some(format!("Could not load notes: {error}"))
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

    fn change_destination(
        &mut self,
        destination: Destination,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.composing(window, cx) {
            return;
        }
        self.set_destination(destination, window, cx);
    }

    fn select_number(&mut self, number: usize, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(project) = number
            .checked_sub(2)
            .and_then(|index| self.projects.get(index))
        {
            self.change_destination(Destination::Project(project.id.clone()), window, cx);
        }
    }

    fn traverse_destination(&mut self, next: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.shortcuts_blocked(window, cx) {
            return;
        }
        let index = match &self.destination {
            Destination::Home => 0,
            Destination::Project(id) => {
                let Some(index) = self.projects.iter().position(|p| &p.id == id) else {
                    return;
                };
                index + 1
            }
        };
        let target = if next {
            index + 1
        } else {
            index.saturating_sub(1)
        };
        if target == 0 {
            self.change_destination(Destination::Home, window, cx);
        } else if let Some(project) = self.projects.get(target - 1) {
            self.change_destination(Destination::Project(project.id.clone()), window, cx);
        }
    }

    fn set_destination(
        &mut self,
        destination: Destination,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Destination::Project(id) = &destination
            && !self.projects.iter().any(|p| &p.id == id)
        {
            return;
        }
        self.close_detail(window, cx);
        self.composer
            .update(cx, |input, cx| input.focus(window, cx));
        if destination == self.destination {
            return;
        }
        self.destination = destination.clone();
        *self
            .services
            .destination
            .lock()
            .expect("window destination") = destination;
        self.model.selected = None;
        self.model.rows = Arc::default();
        self.archived.clear();
        self.loaded = false;
        self.watch_error = None;
        self.list_scroll = gpui_kit::UniformListScrollHandle::new();
        self.refresh_detail(window, cx);
        self.subscribe(window, cx);
        cx.notify();
    }

    fn subscribe_status(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.services.real_account {
            return;
        }
        let services = self.services.clone();
        let db = services.db.clone();
        let (sender, mut receiver) = tokio::sync::watch::channel(None);
        let job = self.services.runtime.spawn(async move {
            use futures_lite::StreamExt;
            let notes_stream = db.sync_stream("notes", None);
            let stream = db.watch_status();
            futures_lite::pin!(stream);
            while let Some(status) = stream.next().await {
                let mut defaults = status
                    .streams()
                    .filter(|s| s.subscription.is_default() && s.subscription.is_active())
                    .peekable();
                let required_ready = defaults.peek().map(|_| ());
                let required_ready =
                    required_ready.map(|()| defaults.all(|s| s.subscription.has_synced()));
                let notes = status.for_stream(&notes_stream);
                let snapshot = crate::sync_progress::Snapshot {
                    connected: status.is_connected(),
                    connecting: status.is_connecting(),
                    downloading: status.is_downloading(),
                    error: status.download_error().is_some() || status.upload_error().is_some(),
                    required_ready,
                    notes_applied: notes.as_ref().is_some_and(|s| s.subscription.has_synced()),
                    notes_progress: notes
                        .and_then(|s| s.progress)
                        .map(|p| (p.total, p.downloaded)),
                };
                let mut first_sync = services.first_sync.lock().expect("first-sync presentation");
                let message = first_sync.update(snapshot);
                sender.send_replace(Some((first_sync.complete, message, first_sync.progress)));
                if sender.is_closed() {
                    break;
                }
            }
        });
        // Dropping the window receiver ends this observer even while sync is quiet.
        let abort = job.abort_handle();
        self.status_task = Some(cx.spawn_in(window, async move |entity, cx| {
            struct AbortOnDrop(tokio::task::AbortHandle);
            impl Drop for AbortOnDrop {
                fn drop(&mut self) {
                    self.0.abort();
                }
            }
            let _guard = AbortOnDrop(abort);
            loop {
                if let Some((synced, message, progress)) = receiver.borrow_and_update().clone()
                    && entity
                        .update_in(cx, |this, _, cx| {
                            this.first_synced |= synced;
                            this.sync_message = message;
                            this.sync_progress = progress;
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

    fn subscribe_capture(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let services = self.services.clone();
        let app: &mut App = cx;
        self._subscriptions
            .push(app.observe(&self.composer, move |input, cx| {
                let input = input.read(cx);
                *services.draft.lock().expect("composer draft") =
                    (input.value().to_string(), input.selected_range());
            }));
        let mut receiver = self.services.capture_changed.subscribe();
        self.refresh_capture(window, cx);
        self.capture_task = Some(cx.spawn_in(window, async move |entity, cx| {
            while receiver.changed().await.is_ok() {
                if entity.update_in(cx, Self::refresh_capture).is_err() {
                    break;
                }
            }
        }));
    }

    fn refresh_capture(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.model.reconcile();
        self.error = self.model.capture().error.clone();
        let restore = std::mem::take(&mut self.model.capture().restore);
        if restore && self.composer.read(cx).value().is_empty() && !self.composing(window, cx) {
            let text = self.model.capture().recovery.pop();
            if let Some(text) = text {
                self.composer
                    .update(cx, |input, cx| input.set_value(text, window, cx));
            }
        }
        cx.notify();
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
        *self.services.draft.lock().expect("composer draft") = (String::new(), 0..0);
        let services = self.services.clone();
        let job = self.services.runtime.spawn(async move {
            let result = services
                .app
                .handle(AppRequest::NoteAdd(NoteAddInput {
                    content: text,
                    project: None,
                    interpret_as_url: false,
                    draft: false,
                    topics: vec![],
                    created_by: None,
                    created_at: None,
                }))
                .await;
            let restore = services.draft.lock().expect("composer draft").0.is_empty();
            services
                .capture
                .lock()
                .expect("capture state")
                .complete(token, result, restore);
            services.capture_changed.send_replace(());
        });
        self.services.track(&job);
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
            "trial window visible_renders={} max_visible_main_tick_ms={:.3}",
            self.frames,
            self.max_main_tick_ms
        );
    }
}

#[path = "presentation.rs"]
mod presentation;

#[derive(Clone)]
pub(super) enum WorkspaceState {
    Login(crate::login::LoginHandle),
    Ready(Arc<Services>),
}
type HostState = tokio::sync::watch::Receiver<Option<Result<WorkspaceState, String>>>;
struct Shell {
    _appearance: Subscription,
    content: Option<Entity<Today>>,
    login: Option<Entity<crate::login::LoginPane>>,
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
                            Ok(WorkspaceState::Ready(services)) => {
                                this.login = None;
                                this.content = Some(cx.new(|cx| Today::new(services, window, cx)))
                            }
                            Ok(WorkspaceState::Login(handle)) => {
                                this.content = None;
                                this.login =
                                    Some(cx.new(|cx| {
                                        crate::login::LoginPane::new(handle, window, cx)
                                    }));
                            }
                            Err(error) => {
                                this.content = None;
                                this.login = None;
                                this.error = Some(error);
                            }
                        }
                        cx.notify();
                    });
                }
                if state.changed().await.is_err() {
                    break;
                }
            }
        });
        Self {
            content: None,
            login: None,
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
            .children(self.login.clone())
            .when(self.content.is_none() && self.login.is_none(), |d| {
                d.p_4().child(
                    self.error
                        .clone()
                        .unwrap_or_else(|| "Opening workspace…".into()),
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
            title: Some("FlickNote — Independent Trial".into()),
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
    let options = crate::launch::Options::parse();
    let runtime = tokio::runtime::Runtime::new()?;
    let handle = runtime.handle().clone();
    let (ready, state) = tokio::sync::watch::channel(None);
    let (quit, mut receiver) = tokio::sync::watch::channel(false);
    let task = runtime.spawn(async move {
        let login_state = ready.clone();
        let host = match options
            .start(receiver.clone(), move |handle| {
                login_state.send_replace(Some(Ok(WorkspaceState::Login(handle))));
            })
            .await
        {
            Ok(host) => host,
            Err(_) if *receiver.borrow() => return Ok(()),
            Err(error) => {
                ready.send_replace(Some(Err(error.to_string())));
                return Err(error.to_string());
            }
        };
        ready.send_replace(Some(Ok(WorkspaceState::Ready(Arc::new(
            host.services(handle),
        )))));
        host.burst(options.burst_batches).await?;
        let app_state = ready.borrow().clone();
        host.run_until(
            async {
                if !*receiver.borrow() {
                    let _changed = receiver.changed().await;
                }
                Ok(())
            },
            || {
                if let Some(Ok(WorkspaceState::Ready(services))) = app_state {
                    services.cancel_operations();
                }
            },
        )
        .await
        .map_err(|error| {
            if let Some(Ok(WorkspaceState::Ready(services))) = ready.borrow().clone() {
                services.cancel_operations();
            }
            ready.send_replace(Some(Err(format!("Workspace stopped: {error}"))));
            error
        })
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
        cx.on_action(move |_: &Reopen, cx| {
            if let Some(Ok(WorkspaceState::Ready(services))) = reopen.borrow().as_ref() {
                *services.destination.lock().expect("window destination") = Destination::Home;
            }
            open(reopen.clone(), reopen_system.clone(), cx);
        });
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
                MenuItem::action("Quit FlickNote Trial", Quit),
            ],
        }]);
        let mut quit = Some(quit);
        cx.on_app_quit(move |_| {
            if let Some(quit) = quit.take() {
                quit.send_replace(true);
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

#[cfg(test)]
#[path = "sync_progress_tests.rs"]
mod sync_progress_tests;
