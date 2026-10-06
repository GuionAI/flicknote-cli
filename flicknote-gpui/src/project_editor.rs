//! Bounded workspace editors; Kit owns text, composition and selection.
use super::*;
use flicknote_client::dto::{Patch, ProjectAddInput, ProjectModifyInput};
use gpui_kit::base::{Disableable, TestSupportExt};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::{
    Sizable,
    button::{Button, ButtonVariants},
};
#[derive(Clone, PartialEq, Eq)]
pub(super) enum Kind {
    Add,
    Summary(String),
    Organization,
}
pub(super) struct Editor {
    pub(super) kind: Kind,
    pub(super) input: Entity<TextareaState>,
    pub(super) key: Option<Entity<InputState>>,
    pub(super) busy: bool,
    pub(super) error: Option<String>,
    _enter: Subscription,
    _operation: Option<Task<()>>,
}
impl Today {
    pub(super) fn edit(&mut self, kind: Kind, window: &mut Window, cx: &mut Context<Self>) {
        if self.editor.is_some() || self.composing(window, cx) {
            return;
        }
        let value = match &kind {
            Kind::Summary(id) => self
                .projects
                .iter()
                .find(|p| &p.id == id)
                .and_then(|p| p.summary.clone())
                .unwrap_or_default(),
            _ => String::new(),
        };
        let masked = kind == Kind::Organization;
        let multiline = matches!(kind, Kind::Summary(_));
        let input = cx.new(|cx| {
            TextareaState::new(window, cx)
                .auto_grow(1, 6)
                .submit_on_enter(!multiline)
        });
        input.update(cx, |i, cx| {
            i.set_value(value, window, cx);
            i.focus(window, cx);
        });
        let key = masked.then(|| {
            cx.new(|cx| {
                InputState::new(window, cx)
                    .masked(true)
                    .placeholder("OpenRouter key")
            })
        });
        if let Some(key) = &key {
            key.update(cx, |i, cx| i.focus(window, cx));
        }
        let subscription = cx.subscribe_in(&input, window, |this, _, event, window, cx| {
            if let InputEvent::PressEnter {
                shift: false,
                secondary,
            } = event
            {
                let composing = std::mem::take(&mut this.enter_composing);
                let save = this.editor.as_ref().is_some_and(|e| match e.kind {
                    Kind::Add => !*secondary,
                    Kind::Summary(_) => *secondary,
                    Kind::Organization => false,
                });
                if save && !composing {
                    this.save_editor(window, cx);
                }
            }
        });
        self.editor = Some(Editor {
            kind,
            input,
            key,
            busy: false,
            error: None,
            _enter: subscription,
            _operation: None,
        });
        cx.notify();
    }
    pub(super) fn editor_composing(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        self.editor.as_ref().is_some_and(|e| {
            if let Some(key) = &e.key {
                key.update(cx, |i, cx| i.marked_text_range(window, cx).is_some())
            } else {
                e.input
                    .update(cx, |i, cx| i.marked_text_range(window, cx).is_some())
            }
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
        let value = editor.key.as_ref().map_or_else(
            || editor.input.read(cx).value().to_string(),
            |key| key.read(cx).value().to_string(),
        );
        if kind == Kind::Add && value.trim().is_empty() {
            editor.error = Some("Enter a project name".into());
            cx.notify();
            return;
        }
        editor.busy = true;
        editor.error = None;
        let control = self
            .services
            .organization
            .lock()
            .expect("organization control")
            .clone();
        let app = self.services.app.clone();
        let op_kind = kind.clone();
        let operation = self.services.runtime.spawn(async move {
            match op_kind {
                Kind::Organization => control
                    .ok_or("Organization is available in the normal GUI host".to_string())?
                    .change(crate::organization::Change::Save(value))
                    .await
                    .map(|()| None),
                Kind::Add => app
                    .handle(AppRequest::ProjectAdd(ProjectAddInput {
                        name: value.trim().into(),
                        color: None,
                    }))
                    .await
                    .map(Some)
                    .map_err(|e| e.message),
                Kind::Summary(id) => app
                    .handle(AppRequest::ProjectModify(ProjectModifyInput {
                        id,
                        color: Patch::Missing,
                        summary: if value.trim().is_empty() {
                            Patch::Null
                        } else {
                            Patch::Value(value)
                        },
                    }))
                    .await
                    .map(Some)
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
                            && let Some(AppResponse::Project(p)) = response
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
    fn organization_change(
        &mut self,
        change: crate::organization::Change,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(editor) = self.editor.as_mut() else {
            return;
        };
        if editor.busy {
            return;
        }
        let Some(control) = self
            .services
            .organization
            .lock()
            .expect("organization control")
            .clone()
        else {
            return;
        };
        editor.busy = true;
        let job = self
            .services
            .runtime
            .spawn(async move { control.change(change).await });
        self.services.track(&job);
        let identity = editor.input.entity_id();
        editor._operation = Some(cx.spawn_in(window, async move |entity, cx| {
            let result = job.await;
            let _updated = entity.update(cx, |this, cx| {
                if let Some(editor) = this
                    .editor
                    .as_mut()
                    .filter(|e| e.input.entity_id() == identity)
                {
                    editor.busy = false;
                    editor.error = match result {
                        Ok(Ok(())) => None,
                        Ok(Err(e)) => Some(e),
                        Err(_) => Some("Could not change organization".into()),
                    };
                    cx.notify();
                }
            });
        }));
        cx.notify();
    }
    pub(super) fn render_summary(&self, cx: &mut Context<Self>) -> Option<gpui_kit::AnyElement> {
        let Destination::Project(id) = &self.destination else {
            return None;
        };
        let project = self.projects.iter().find(|p| &p.id == id)?;
        let id = id.clone();
        Some(
            div()
                .px_3()
                .py_2()
                .flex()
                .gap_2()
                .items_start()
                .child(
                    div()
                        .id("project-summary")
                        .flex_1()
                        .min_w_0()
                        .max_h(px(96.))
                        .overflow_y_scroll()
                        .text_sm()
                        .child(
                            project
                                .summary
                                .clone()
                                .filter(|s| !s.trim().is_empty())
                                .unwrap_or_else(|| "Add a project summary".into()),
                        ),
                )
                .child(
                    Button::new("edit-project-summary")
                        .label("Edit")
                        .ghost()
                        .small()
                        .on_click(cx.listener(move |this, _, window, cx| {
                            cx.stop_propagation();
                            this.edit(Kind::Summary(id.clone()), window, cx);
                        })),
                )
                .into_any_element(),
        )
    }
    fn editor_buttons(
        &self,
        e: &Editor,
        state: &crate::organization::State,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let enabled = state.enabled;
        div()
            .flex()
            .flex_wrap()
            .gap_2()
            .child(
                Button::new("save-editor")
                    .label(if e.kind == Kind::Organization && state.has_key {
                        "Replace key"
                    } else {
                        "Save"
                    })
                    .disabled(e.busy || (e.kind == Kind::Organization && !state.ready))
                    .on_click(cx.listener(|this, _, window, cx| this.save_editor(window, cx))),
            )
            .when(e.kind == Kind::Organization, |d| {
                d.child(
                    Button::new("toggle-organization")
                        .label(if state.enabled { "Disable" } else { "Enable" })
                        .disabled(e.busy || !state.ready || !state.has_key)
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.organization_change(
                                crate::organization::Change::Enable(!enabled),
                                window,
                                cx,
                            )
                        })),
                )
                .child(
                    Button::new("remove-organization-key")
                        .label("Remove key")
                        .disabled(e.busy || !state.ready)
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.organization_change(
                                crate::organization::Change::Remove,
                                window,
                                cx,
                            )
                        })),
                )
            })
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
            Kind::Summary(_) => "Project summary",
            Kind::Organization => "Automatic organization",
        };
        let control = self
            .services
            .organization
            .lock()
            .expect("organization control")
            .clone();
        let state = control
            .as_ref()
            .map(|c| c.state.borrow().clone())
            .unwrap_or_default();
        let error = e
            .error
            .clone()
            .or_else(|| {
                (e.kind == Kind::Organization)
                    .then_some(state.error.clone())
                    .flatten()
            })
            .or_else(|| {
                if e.kind == Kind::Organization {
                    control
                        .as_ref()
                        .and_then(|c| c.routing_error.borrow().clone())
                } else {
                    None
                }
            });
        let theme = Theme::global(cx).clone();
        let input = if let Some(key) = &e.key {
            Input::new(key)
                .aria_label("OpenRouter key")
                .disabled(e.busy)
                .into_any_element()
        } else {
            Textarea::new(&e.input)
                .aria_label(title)
                .disabled(e.busy)
                .into_any_element()
        };
        let buttons = self.editor_buttons(e, &state, cx);
        let pane = div()
            .id("editor-pane")
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
            .when(e.kind == Kind::Organization, |d| {
                d.child(if state.enabled {
                    "Enabled — new eligible notes are sent to OpenRouter"
                } else {
                    "Disabled — save a key to enable"
                })
                .child("Project names, summaries and note summaries are sent to a paid provider.")
            })
            .child(input)
            .children(error.map(|e| div().text_sm().text_color(theme.danger).child(e)))
            .child(buttons);
        Some(
            div()
                .id("workspace-editor")
                .absolute()
                .inset_0()
                .flex()
                .items_center()
                .justify_center()
                .bg(theme.background.opacity(0.8))
                .on_click(cx.listener(|_, _, _, cx| cx.stop_propagation()))
                .child(pane)
                .test_support()
                .into_any_element(),
        )
    }
}
