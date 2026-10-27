//! Read-only saved history, held as one owner-scoped snapshot per dialog.
use super::*;
use flicknote_sync::DesktopVersion;
use gpui_kit::base::{Dialog, DialogBackdrop, DialogPopup, Selectable, TestSupportExt};
use gpui_kit::component::{
    Sizable,
    button::{Button, ButtonVariants},
};

pub(super) struct HistoryDialog {
    id: i64,
    uuid: String,
    archived: bool,
    focus: gpui_kit::FocusHandle,
    list_focus: gpui_kit::FocusHandle,
    previous_focus: Option<gpui_kit::FocusHandle>,
    versions: Option<Result<Vec<DesktopVersion>, ()>>,
    selected: usize,
    reader: Option<Entity<gpui_kit::base::TextViewState>>,
    scroll: gpui_kit::ScrollHandle,
    list_scroll: gpui_kit::ScrollHandle,
    _task: Option<Task<()>>,
}

#[cfg(test)]
impl HistoryDialog {
    pub(super) fn ready(&self) -> bool {
        self.versions.is_some()
    }
    pub(super) fn loading(&self) -> bool {
        self.versions.is_none()
    }
    pub(super) fn empty(&self) -> bool {
        matches!(&self.versions, Some(Ok(versions)) if versions.len() == 1)
    }
    pub(super) fn failed(&self) -> bool {
        matches!(&self.versions, Some(Err(())))
    }
    pub(super) fn list_focused(&self, window: &Window) -> bool {
        self.list_focus.is_focused(window)
    }
    pub(super) fn previous_focus(&self) -> Option<gpui_kit::FocusHandle> {
        self.previous_focus.clone()
    }
    pub(super) fn offsets(&self) -> (gpui_kit::Pixels, gpui_kit::Pixels) {
        (self.list_scroll.offset().y, self.scroll.offset().y)
    }
    pub(super) fn reader(&self) -> Option<Entity<gpui_kit::base::TextViewState>> {
        self.reader.clone()
    }
    pub(super) fn text(&self, cx: &App) -> String {
        self.reader
            .as_ref()
            .map(|r| r.read(cx).rendered_text().as_str().to_string())
            .unwrap_or_default()
    }
}

impl Today {
    pub(super) fn open_history(
        &mut self,
        row: flicknote_sync::today::TodayRow,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.history.is_some() || self.editor.is_some() || self.composing(window, cx) {
            return;
        }
        let previous_focus = window.focused(cx);
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        let app = self.services.app.clone();
        let local = flicknote_core::backend::LocalPowerSyncBackend::new(
            self.services.db.clone(),
            self.services.user_id.clone(),
        );
        let uuid = row.uuid.clone();
        let id = row.id;
        let archived = row.archived;
        let operation = self
            .services
            .runtime
            .spawn(async move { app.desktop_history(&local, &uuid, id, archived).await });
        self.services.track(&operation);
        let dialog = HistoryDialog {
            id,
            uuid: row.uuid,
            archived,
            focus,
            list_focus: cx.focus_handle(),
            previous_focus,
            versions: None,
            selected: 0,
            reader: None,
            scroll: gpui_kit::ScrollHandle::new(),
            list_scroll: gpui_kit::ScrollHandle::new(),
            _task: None,
        };
        self.history = Some(dialog);
        let identity = self.history.as_ref().unwrap().focus.clone();
        let task = cx.spawn_in(window, async move |entity, cx| {
            let result = operation.await;
            let _updated = entity.update_in(cx, |this, window, cx| {
                let Some(dialog) = this.history.as_mut() else {
                    return;
                };
                if dialog.focus != identity
                    || this.model.selected != Some(dialog.id)
                    || !this.model.rows.iter().any(|row| {
                        row.id == dialog.id
                            && row.uuid == dialog.uuid
                            && row.archived == dialog.archived
                    })
                {
                    return;
                }
                dialog.versions = Some(match result {
                    Ok(Ok(versions)) => Ok(versions),
                    _ => Err(()),
                });
                if let Some(Ok(versions)) = &dialog.versions
                    && let Some(version) = versions.first()
                {
                    dialog.reader =
                        Some(cx.new(|cx| {
                            gpui_kit::base::TextViewState::markdown(&version.content, cx)
                        }));
                    window.focus(&dialog.list_focus, cx);
                }
                cx.notify();
            });
        });
        self.history.as_mut().unwrap()._task = Some(task);
        cx.notify();
    }

    pub(super) fn close_history(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(dialog) = self.history.take() {
            if let Some(focus) = dialog.previous_focus {
                window.focus(&focus, cx);
            }
            cx.notify();
        }
    }

    pub(super) fn reconcile_history(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.history.as_ref().is_some_and(|dialog| {
            self.model.selected != Some(dialog.id)
                || !self.detail_open
                || !self.model.rows.iter().any(|row| {
                    row.id == dialog.id
                        && row.uuid == dialog.uuid
                        && row.archived == dialog.archived
                })
        }) {
            self.close_history(window, cx);
        }
    }

    fn select_history(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(dialog) = self.history.as_mut() else {
            return;
        };
        let Some(Ok(versions)) = &dialog.versions else {
            return;
        };
        let Some(version) = versions.get(index) else {
            return;
        };
        dialog.selected = index;
        dialog.list_scroll.scroll_to_item(index);
        dialog.reader =
            Some(cx.new(|cx| gpui_kit::base::TextViewState::markdown(&version.content, cx)));
        dialog.scroll = gpui_kit::ScrollHandle::new();
        cx.notify();
    }

    fn move_history(&mut self, next: bool, cx: &mut Context<Self>) {
        let Some(dialog) = self.history.as_ref() else {
            return;
        };
        let Some(Ok(versions)) = &dialog.versions else {
            return;
        };
        if versions.is_empty() {
            return;
        }
        let index = if next {
            (dialog.selected + 1).min(versions.len() - 1)
        } else {
            dialog.selected.saturating_sub(1)
        };
        self.select_history(index, cx);
    }

    #[allow(clippy::too_many_lines)] // Keep the dialog's states and layout together.
    pub(super) fn render_history(&self, cx: &mut Context<Self>) -> Option<gpui_kit::AnyElement> {
        let dialog = self.history.as_ref()?;
        let theme = Theme::global(cx).clone();
        let mut body = div()
            .id("history-body")
            .test_support()
            .flex_1()
            .min_h_0()
            .flex()
            .overflow_hidden();
        match &dialog.versions {
            None => body = body.child(div().p_4().child("Loading history…")),
            Some(Err(())) => {
                body = body.child(
                    div()
                        .p_4()
                        .child("Could not load history. Close and reopen to try again."),
                )
            }
            Some(Ok(versions)) => {
                let mut list =
                    div()
                        .id("history-list")
                        .track_focus(&dialog.list_focus)
                        .key_context("HistoryList")
                        .on_action(
                            cx.listener(|this, _: &HistoryNext, _, cx| this.move_history(true, cx)),
                        )
                        .on_action(cx.listener(|this, _: &HistoryPrevious, _, cx| {
                            this.move_history(false, cx)
                        }))
                        .test_support()
                        .w(px(180.))
                        .flex_shrink_0()
                        .h_full()
                        .overflow_y_scroll()
                        .track_scroll(&dialog.list_scroll)
                        .border_r_1()
                        .border_color(theme.border)
                        .p_2()
                        .flex()
                        .flex_col()
                        .gap_1();
                for (index, version) in versions.iter().enumerate() {
                    let time = version.time.as_ref().map(|value| {
                        chrono::DateTime::parse_from_rfc3339(value)
                            .map(|t| {
                                t.with_timezone(&chrono::Local)
                                    .format("%b %-d, %Y %H:%M")
                                    .to_string()
                            })
                            .unwrap_or_else(|_| value.clone())
                    });
                    let label = version
                        .version
                        .map_or_else(|| "Current".to_owned(), |n| format!("v{n}"));
                    let accessibility = time
                        .as_ref()
                        .map_or_else(|| label.clone(), |t| format!("{label}, {t}"));
                    list = list.child(
                        Button::new(("history-version", version.version.unwrap_or(0) as u64))
                            .ghost()
                            .selected(index == dialog.selected)
                            .toggled(index == dialog.selected)
                            .w_full()
                            .h(px(54.))
                            .accessibility_label(accessibility)
                            .child(
                                div()
                                    .w_full()
                                    .flex()
                                    .flex_col()
                                    .items_start()
                                    .child(label)
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(theme.muted_foreground)
                                            .child(time.unwrap_or_default()),
                                    ),
                            )
                            .on_click(
                                cx.listener(move |this, _, _, cx| this.select_history(index, cx)),
                            ),
                    );
                }
                if versions.len() == 1 {
                    list = list.child(
                        div()
                            .p_2()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child("No saved versions."),
                    );
                }
                body = body.child(list);
                if let Some(state) = &dialog.reader {
                    let weak = cx.entity().downgrade();
                    body = body.child(
                        div()
                            .id("history-reading")
                            .test_support()
                            .flex_1()
                            .min_w_0()
                            .h_full()
                            .overflow_y_scroll()
                            .track_scroll(&dialog.scroll)
                            .p_4()
                            .child(presentation::reader(state, cx, move |text, cx| {
                                let _updated = weak.update(cx, |this, cx| this.copy_text(text, cx));
                            })),
                    );
                }
            }
        }
        let pane = div()
            .id("history-pane")
            .test_support()
            .w(px(700.))
            .max_w_full()
            .h(px(500.))
            .max_h_full()
            .bg(theme.popover)
            .border_1()
            .border_color(theme.border)
            .rounded_lg()
            .flex()
            .flex_col()
            .overflow_hidden()
            .child(
                div()
                    .h(px(52.))
                    .flex_shrink_0()
                    .px_4()
                    .flex()
                    .items_center()
                    .justify_between()
                    .border_b_1()
                    .border_color(theme.border)
                    .child("History")
                    .child(
                        Button::new("close-history")
                            .label("Close")
                            .ghost()
                            .small()
                            .on_click(
                                cx.listener(|this, _, window, cx| this.close_history(window, cx)),
                            ),
                    ),
            )
            .child(body);
        let cancel = cx.weak_entity();
        Some(
            Dialog::new(cx)
                .focus_handle(dialog.focus.clone())
                .close_on_escape(true)
                .close_on_backdrop_press(true)
                .on_cancel(move |_, window, cx| {
                    let _updated = cancel.update(cx, |this, cx| this.close_history(window, cx));
                    false
                })
                .backdrop(
                    DialogBackdrop::new()
                        .absolute()
                        .inset_0()
                        .bg(theme.background.opacity(0.8))
                        .child(div().id("history-backdrop").test_support().size_full()),
                )
                .popup(DialogPopup::new().max_h_full().child(pane))
                .into_any_element(),
        )
    }
}
