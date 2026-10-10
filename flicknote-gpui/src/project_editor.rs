//! Bounded workspace editors; Kit owns text, composition and selection.
use super::*;
use flicknote_client::dto::{Patch, ProjectAddInput, ProjectModifyInput};
use gpui_kit::base::{Dialog, DialogBackdrop, DialogPopup, Disableable, TestSupportExt};
use gpui_kit::component::{
    Sizable,
    button::{Button, ButtonVariants},
};
#[derive(Clone, PartialEq, Eq)]
pub(super) enum Kind {
    Add,
    Description(String),
}
pub(super) struct Editor {
    pub(super) kind: Kind,
    pub(super) input: Entity<TextareaState>,
    pub(super) busy: bool,
    pub(super) error: Option<String>,
    focus: gpui_kit::FocusHandle,
    _enter: Subscription,
    _operation: Option<Task<()>>,
}
impl Today {
    pub(super) fn edit(&mut self, kind: Kind, window: &mut Window, cx: &mut Context<Self>) {
        if self.editor.is_some() || self.composing(window, cx) {
            return;
        }
        gpui_kit::base::TextSelection::clear(window, cx);
        let value = match &kind {
            Kind::Description(id) => self
                .projects
                .iter()
                .find(|p| &p.id == id)
                .and_then(|p| p.description.clone())
                .unwrap_or_default(),
            _ => String::new(),
        };
        let multiline = matches!(kind, Kind::Description(_));
        let input = cx.new(|cx| {
            TextareaState::new(window, cx)
                .auto_grow(1, 6)
                .submit_on_enter(!multiline)
        });
        input.update(cx, |i, cx| {
            i.set_value(value, window, cx);
            i.focus(window, cx);
        });
        let subscription = cx.subscribe_in(&input, window, |this, _, event, window, cx| {
            if let InputEvent::PressEnter {
                shift: false,
                secondary,
            } = event
            {
                let composing = std::mem::take(&mut this.enter_composing);
                let save = this.editor.as_ref().is_some_and(|e| match e.kind {
                    Kind::Add => !*secondary,
                    Kind::Description(_) => *secondary,
                });
                if save && !composing {
                    this.save_editor(window, cx);
                }
            }
        });
        self.editor = Some(Editor {
            focus: cx.focus_handle(),
            kind,
            input,
            busy: false,
            error: None,
            _enter: subscription,
            _operation: None,
        });

        cx.notify();
    }
    pub(super) fn editor_composing(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        self.editor.as_ref().is_some_and(|e| {
            e.input
                .update(cx, |i, cx| i.marked_text_range(window, cx).is_some())
        })
    }

    pub(super) fn cancel_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.editor.as_ref().is_some_and(|e| e.busy) || self.editor_composing(window, cx) {
            return;
        }
        self.editor = None;
        self.composer.update(cx, |i, cx| i.focus(window, cx));
        cx.notify();
    }
    pub(super) fn save_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.editor_composing(window, cx) {
            return;
        }
        let Some(editor) = self.editor.as_mut() else {
            return;
        };
        if editor.busy {
            return;
        }
        let kind = editor.kind.clone();
        let value = editor.input.read(cx).value().to_string();
        if kind == Kind::Add && value.trim().is_empty() {
            editor.error = Some("Enter a project name".into());
            cx.notify();
            return;
        }
        editor.busy = true;
        editor.error = None;
        let app = self.services.app.clone();
        let op_kind = kind.clone();
        let operation = self.services.runtime.spawn(async move {
            match op_kind {
                Kind::Add => app
                    .handle(AppRequest::ProjectAdd(ProjectAddInput {
                        name: value.trim().into(),
                        color: None,
                    }))
                    .await
                    .map_err(|e| e.message),
                Kind::Description(id) => app
                    .handle(AppRequest::ProjectModify(ProjectModifyInput {
                        id,
                        color: Patch::Missing,
                        description: if value.trim().is_empty() {
                            Patch::Null
                        } else {
                            Patch::Value(value)
                        },
                    }))
                    .await
                    .map_err(|e| e.message),
            }
        });
        self.services.track(&operation);
        let identity = editor.input.entity_id();
        editor._operation = Some(cx.spawn_in(window, async move |entity, cx| {
            let result = operation.await;
            let _updated = entity.update_in(cx, |this, window, cx| {
                if !this
                    .editor
                    .as_ref()
                    .is_some_and(|e| e.input.entity_id() == identity)
                {
                    return;
                }
                match result {
                    Ok(Ok(response)) => {
                        this.editor = None;
                        if kind == Kind::Add
                            && let AppResponse::Project(p) = response
                        {
                            this.pending_project = Some(p.id);
                            this.select_created(window, cx);
                        }
                        this.composer.update(cx, |i, cx| i.focus(window, cx));
                    }
                    Ok(Err(error)) => {
                        let e = this.editor.as_mut().expect("editor identity");
                        e.busy = false;
                        e.error = Some(error);
                    }
                    Err(_) => {
                        let e = this.editor.as_mut().expect("editor identity");
                        e.busy = false;
                        e.error =
                            Some("Could not save. Check the stored value before retrying.".into());
                    }
                }
                cx.notify();
            });
        }));
        cx.notify();
    }
    pub(super) fn select_created(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(id) = self.pending_project.clone()
            && self.projects.iter().any(|p| p.id == id)
        {
            self.pending_project = None;
            self.set_destination(Destination::Project(id), window, cx);
        }
    }

    pub(super) fn render_description(
        &self,
        cx: &mut Context<Self>,
    ) -> Option<gpui_kit::AnyElement> {
        let Destination::Project(id) = &self.destination else {
            return None;
        };
        let project = self.projects.iter().find(|p| &p.id == id)?;
        let id = id.clone();
        Some(
            div()
                .id("project-description-region")
                .test_support()
                .h(px(80.))
                .flex_shrink_0()
                .px_3()
                .py_2()
                .flex()
                .gap_2()
                .items_start()
                .child(
                    div()
                        .id("project-description")
                        .flex_1()
                        .min_w_0()
                        .h_full()
                        .test_support()
                        .overflow_y_scroll()
                        .text_sm()
                        .child(
                            div().id("project-description-text").test_support().child(
                                project
                                    .description
                                    .clone()
                                    .filter(|s| !s.trim().is_empty())
                                    .unwrap_or_else(|| "Add a project description".into()),
                            ),
                        ),
                )
                .child(
                    Button::new("edit-project-description")
                        .label("Edit")
                        .ghost()
                        .small()
                        .on_click(cx.listener(move |this, _, window, cx| {
                            cx.stop_propagation();
                            this.edit(Kind::Description(id.clone()), window, cx);
                        })),
                )
                .into_any_element(),
        )
    }
    fn editor_buttons(&self, e: &Editor, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        div()
            .flex()
            .gap_2()
            .child(
                Button::new("save-editor")
                    .label("Save")
                    .disabled(e.busy)
                    .on_click(cx.listener(|this, _, window, cx| this.save_editor(window, cx))),
            )
            .child(
                Button::new("cancel-editor")
                    .label("Cancel")
                    .ghost()
                    .disabled(e.busy)
                    .on_click(cx.listener(|this, _, window, cx| this.cancel_editor(window, cx))),
            )
    }
    pub(super) fn render_editor(&self, cx: &mut Context<Self>) -> Option<gpui_kit::AnyElement> {
        let e = self.editor.as_ref()?;
        let title = match e.kind {
            Kind::Add => "Add project",
            Kind::Description(_) => "Project description",
        };
        let error = e.error.clone();
        let theme = Theme::global(cx).clone();
        let input = Textarea::new(&e.input)
            .aria_label(title)
            .disabled(e.busy)
            .into_any_element();
        let buttons = self.editor_buttons(e, cx);
        let pane = div()
            .id("editor-pane")
            .test_support()
            .w(px(420.))
            .max_h_full()
            .overflow_y_scroll()
            .p_4()
            .bg(theme.popover)
            .border_1()
            .border_color(theme.border)
            .rounded_lg()
            .flex()
            .flex_col()
            .gap_3()
            .child(title)
            .child(input)
            .children(error.map(|e| div().text_sm().text_color(theme.danger).child(e)))
            .child(buttons);
        let save = cx.weak_entity();
        let cancel = save.clone();
        Some(
            Dialog::new(cx)
                .focus_handle(e.focus.clone())
                .close_on_escape(true)
                .close_on_backdrop_press(false)
                .on_ok(move |_, window, cx| {
                    let _updated = save.update(cx, |this, cx| this.save_editor(window, cx));
                    // The editor's UUID-bound async completion owns dismissal.
                    false
                })
                .on_cancel(move |_, window, cx| {
                    let _updated = cancel.update(cx, |this, cx| this.cancel_editor(window, cx));
                    false
                })
                .backdrop(
                    DialogBackdrop::new()
                        .absolute()
                        .inset_0()
                        .bg(theme.background.opacity(0.8))
                        .child(div().size_full().occlude()),
                )
                .popup(DialogPopup::new().max_h_full().child(pane))
                .into_any_element(),
        )
    }
}
