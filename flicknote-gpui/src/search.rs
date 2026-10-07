//! Disposable same-list search state; capture and ordinary calendar watches remain independent.
use super::*;
use flicknote_sync::{today::TodayRow, workspace_search::Results};
use futures_lite::StreamExt;
use gpui_kit::Focusable;

pub(super) struct Origin {
    pub(super) destination: Destination,
    pub(super) period: Period,
    pub(super) rows: Arc<Vec<TodayRow>>,
    pub(super) selected: Option<String>,
    pub(super) scroll: gpui_kit::UniformListScrollHandle,
}
pub(super) struct RetainedOrigin {
    destination: Destination,
    period: Period,
    rows: Arc<Vec<TodayRow>>,
    selected: Option<String>,
    offset: f32,
}
impl Origin {
    pub(super) fn retain(&self) -> RetainedOrigin {
        RetainedOrigin {
            destination: self.destination.clone(),
            period: self.period.clone(),
            rows: self.rows.clone(),
            selected: self.selected.clone(),
            offset: f32::from(self.scroll.0.borrow().base_handle.offset().y),
        }
    }
    pub(super) fn reopen(value: RetainedOrigin) -> Self {
        let scroll = gpui_kit::UniformListScrollHandle::new();
        scroll
            .0
            .borrow()
            .base_handle
            .set_offset(gpui_kit::point(px(0.), px(value.offset)));
        Self {
            destination: value.destination,
            period: value.period,
            rows: value.rows,
            selected: value.selected,
            scroll,
        }
    }
    pub(super) fn restore_anchor(&self, rows: &[TodayRow]) {
        let offset = f32::from(self.scroll.0.borrow().base_handle.offset().y);
        let index = (-offset / 32.).floor().max(0.) as usize;
        if let Some(anchor) = self
            .rows
            .get(index)
            .and_then(|r| rows.iter().position(|n| n.uuid == r.uuid))
        {
            let y = offset + (index as f32 - anchor as f32) * 32.;
            self.scroll
                .0
                .borrow()
                .base_handle
                .set_offset(gpui_kit::point(px(0.), px(y)));
        }
    }
}
#[derive(Default)]
pub(super) struct Search {
    pub(super) query: String,
    pub(super) marked: bool,
    pub(super) generation: u64,
    pub(super) origin: Option<Origin>,
    pub(super) restoring: Option<Origin>,
    pub(super) hits: Vec<flicknote_client::dto::SearchHit>,
    pub(super) detail: Option<i64>,
    pub(super) loading: bool,
    pub(super) error: Option<String>,
    pub(super) bounded: bool,
    pub(super) open_after: bool,
    pub(super) job: Option<tokio::task::AbortHandle>,
    pub(super) task: Option<Task<()>>,
}
impl Drop for Search {
    fn drop(&mut self) {
        if let Some(job) = self.job.take() {
            job.abort();
        }
    }
}
impl Search {
    pub(super) fn new(restoring: Option<Origin>) -> Self {
        let mut search = Self::default();
        search.restoring = restoring;
        search
    }
    pub(super) fn cancel(&mut self) {
        self.generation += 1;
        self.task.take();
        if let Some(job) = self.job.take() {
            job.abort();
        }
    }
    pub(super) fn active(&self) -> bool {
        self.origin.is_some()
    }
}
impl Today {
    pub(super) fn search_focused(&self, window: &Window, cx: &App) -> bool {
        self.search_input
            .read(cx)
            .focus_handle(cx)
            .is_focused(window)
    }
    pub(super) fn search_composing(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        self.search_input.update(cx, |input, cx| {
            input.marked_text_range(window, cx).is_some()
        })
    }
    pub(super) fn focus_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.editor.is_some() || self.composing(window, cx) || self.search_composing(window, cx)
        {
            return;
        }
        self.search_input
            .update(cx, |input, cx| input.focus(window, cx));
        cx.notify();
    }
    pub(super) fn search_changed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let query = self.search_input.read(cx).value().trim().to_string();
        let marked = self.search_composing(window, cx);
        if self.editor.is_some() || (query == self.search.query && marked == self.search.marked) {
            return;
        }
        self.search.marked = marked;
        if query.is_empty() && !self.search.active() {
            self.search.query.clear();
            self.search.cancel();
            cx.notify();
            return;
        }
        if query.is_empty() && !marked {
            self.exit_search(true, window, cx);
            return;
        }
        if !self.search.active() && !query.is_empty() {
            self.search.origin = Some(Origin {
                destination: self.destination.clone(),
                period: self.period.clone(),
                selected: self
                    .model
                    .selected
                    .and_then(|id| self.model.rows.iter().find(|r| r.id == id))
                    .map(|r| r.uuid.clone()),
                rows: self.model.rows.clone(),
                scroll: self.list_scroll.clone(),
            });
            self.model.search = true;
            self.model.rows = Arc::default();
            self.model.selected = None;
            self.list_scroll = gpui_kit::UniformListScrollHandle::new();
        }
        self.search.query = query;
        self.search.cancel();
        self.search.hits.clear();
        self.canonical_search_rows = Arc::default();
        self.search.detail = None;
        self.search.error = None;
        self.search.bounded = false;
        self.search.open_after = false;
        let focused = self.search_focused(window, cx);
        self.close_detail(window, cx);
        if focused {
            self.search_input
                .update(cx, |input, cx| input.focus(window, cx));
        }
        self.model.rows = Arc::default();
        self.model.selected = None;
        self.search.loading = true;
        if !marked {
            self.run_search(Duration::from_millis(200), window, cx);
        }
        cx.notify();
    }
    pub(super) fn run_search(
        &mut self,
        delay: Duration,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.search.active() || self.search.marked || !self.source.ready {
            return;
        }
        self.search.cancel();
        self.search.loading = true;
        self.search.error = None;
        let generation = self.search.generation;
        let services = self.services.clone();
        let query = self.search.query.clone();
        let human = self.source.human_only;
        let selected = self.model.selected;
        let (send, mut receive) = tokio::sync::watch::channel(None);
        let job = self.services.runtime.spawn(async move {
            // PowerSync resolves the referenced tables, batching notifications while reads run.
            // The mapper does not iterate notes or fetch bodies: only active search is refreshed.
            let changes = services.db.watch_statement(
                "SELECT n.id FROM notes n LEFT JOIN projects p ON p.id = n.project_id LEFT JOIN note_shares s ON s.id = n.id LIMIT 1".into(),
                (), |_, ()| Ok(()),
            );
            futures_lite::pin!(changes);
            tokio::time::sleep(delay).await;
            let mut initial = true;
            while let Some(change) = changes.next().await {
                if !initial {
                    tokio::select! {
                        _ = send.closed() => break,
                        _ = tokio::time::sleep(Duration::from_millis(200)) => {},
                    }
                }
                initial = false;
                let result = match change {
                    Ok(()) => flicknote_sync::workspace_search::read(&services.app, &services.db, &services.user_id, &query, human, selected).await,
                    Err(error) => Err(error.to_string()),
                };
                let failed = result.is_err();
                send.send_replace(Some(result));
                if failed || send.is_closed() { break; }
            }
        });
        self.search.job = Some(job.abort_handle());
        self.services.track(&job);
        self.search.task = Some(cx.spawn_in(window, async move |entity, cx| {
            loop {
                let result = receive.borrow_and_update().clone();
                if let Some(result) = result
                    && entity
                        .update_in(cx, |this, window, cx| {
                            this.receive_search(generation, human, result, window, cx);
                        })
                        .is_err()
                {
                    break;
                }
                if receive.changed().await.is_err() {
                    break;
                }
            }
        }));
        cx.notify();
    }
    pub(super) fn receive_search(
        &mut self,
        generation: u64,
        human: bool,
        result: Result<Results, String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.search.active()
            || generation != self.search.generation
            || human != self.source.human_only
        {
            return;
        }
        self.search.loading = false;
        match result {
            Ok(result) => {
                self.search.error = None;
                self.search.bounded = result.bounded;
                self.search.hits = result.hits;
                self.search.detail = result.detail;
                self.model
                    .capture()
                    .note_actions
                    .observe(&result.rows, &self.action_scope());
                if let Some(row) = result.rows.iter().find(|r| Some(r.id) == result.detail) {
                    self.model
                        .capture()
                        .appends
                        .observe(std::slice::from_ref(row));
                }
                self.canonical_search_rows = result.rows;
                self.project_notes(window, cx);
                if std::mem::take(&mut self.search.open_after)
                    && let Some(row) = self.model.rows.first()
                {
                    self.select(row.id, window, cx);
                }
            }
            Err(error) => {
                self.canonical_search_rows = Arc::default();
                self.search.error = Some(format!("Could not search: {error}"));
                self.search.detail = None;
                self.close_detail(window, cx);
                self.model.rows = Arc::default();
            }
        }
        cx.notify();
    }
    pub(super) fn search_enter(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.search_composing(window, cx) || self.enter_composing || self.editor.is_some() {
            return;
        }
        if self.search.loading || self.search.error.is_some() {
            self.search.open_after = true;
            self.run_search(Duration::ZERO, window, cx);
        } else if let Some(id) = self
            .model
            .selected
            .or_else(|| self.model.rows.first().map(|r| r.id))
        {
            self.select(id, window, cx);
        }
    }
    pub(super) fn search_navigate(
        &mut self,
        next: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.editor.is_some() || self.search_composing(window, cx) || self.model.rows.is_empty()
        {
            return;
        }
        let index = self
            .model
            .selected
            .and_then(|id| self.model.rows.iter().position(|r| r.id == id));
        let index = match index {
            None => 0,
            Some(i) if next => (i + 1).min(self.model.rows.len() - 1),
            Some(i) => i.saturating_sub(1),
        };
        let id = self.model.rows[index].id;
        self.list_scroll
            .scroll_to_item_strict(index, gpui_kit::ScrollStrategy::Nearest);
        self.select(id, window, cx);
    }
    pub(super) fn escape_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.search.active() {
            self.exit_search(true, window, cx);
        } else {
            self.composer.update(cx, |i, cx| i.focus(window, cx));
        }
    }
    pub(super) fn exit_search(
        &mut self,
        restore: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.search.cancel();
        let origin = self.search.origin.take();
        self.search.query.clear();
        self.search.marked = false;
        self.search_input
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.search.hits.clear();
        self.search.detail = None;
        self.search.open_after = false;
        self.canonical_search_rows = Arc::default();
        self.model.search = false;
        self.close_detail(window, cx);
        if let Some(origin) = origin {
            self.destination = origin.destination.clone();
            self.period = origin.period.clone();
            self.model.rows = origin.rows.clone();
            self.model.selected = origin
                .selected
                .as_ref()
                .and_then(|uuid| origin.rows.iter().find(|r| &r.uuid == uuid))
                .map(|r| r.id);
            self.list_scroll = origin.scroll.clone();
            if restore {
                self.search.restoring = Some(origin);
            }
            self.subscribe(window, cx);
        }
        self.composer
            .update(cx, |input, cx| input.focus(window, cx));
        cx.notify();
    }
}
