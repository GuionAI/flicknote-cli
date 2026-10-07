use super::*;
use crate::workspace::{HEADER_HEIGHT, RAIL_WIDTH, reading_width};
use gpui_kit::assets::IconName;
use gpui_kit::base::ColorTokens;
use gpui_kit::base::{Disableable, Selectable, TestSupportExt};
use gpui_kit::component::{
    Icon, Sizable,
    button::{Button, ButtonVariants},
    progress::Progress,
};
use gpui_kit::{ClipboardItem, FontWeight, Hsla, rgb, uniform_list};

fn icon(name: IconName, color: Hsla, width: f32) -> impl IntoElement {
    Icon::new(name).size(px(width)).text_color(color)
}
fn dot(color: &str, width: f32, fallback: Hsla) -> impl IntoElement {
    div()
        .size(px(width))
        .flex_shrink_0()
        .rounded_full()
        .bg(u32::from_str_radix(color.trim_start_matches('#'), 16)
            .map(|value| rgb(value).into())
            .unwrap_or(fallback))
}
fn glyph(note_type: &str) -> IconName {
    match note_type {
        "flash" => IconName::Zap,
        "meeting" => IconName::MessagesSquare,
        "link" => IconName::Link,
        "file" => IconName::FileImage,
        _ => IconName::FileText,
    }
}
fn preview(text: &str, note_type: &str, color: Option<&str>, p: ColorTokens) -> impl IntoElement {
    let name = glyph(note_type);
    div()
        .w_full()
        .min_w_0()
        .h(px(32.))
        .flex()
        .items_center()
        .gap(px(10.))
        .px(px(8.))
        .text_size(px(14.))
        .child(
            div()
                .id("type-glyph")
                .w(px(17.))
                .flex_shrink_0()
                .child(icon(name, p.foreground, 12.))
                .test_support(),
        )
        .child(
            div()
                .id("note-preview")
                .aria_label(text.to_owned())
                .flex_1()
                .min_w_0()
                .truncate()
                .child(text.to_owned())
                .test_support(),
        )
        .children(color.map(|color| {
            div()
                .id("project-dot")
                .size(px(5.))
                .flex_shrink_0()
                .child(dot(color, 5., p.foreground))
                .test_support()
        }))
}
fn rail_slot(child: impl IntoElement) -> impl IntoElement {
    div()
        .w(px(17.))
        .h(px(17.))
        .flex_shrink_0()
        .flex()
        .items_center()
        .justify_center()
        .child(child)
}
fn rail_label(title: &str) -> impl IntoElement {
    div()
        .id(gpui_kit::SharedString::from(format!("rail-label-{title}")))
        .child(title.to_owned())
        .test_support()
}
fn landmark(title: &'static str, name: IconName, p: ColorTokens) -> impl IntoElement {
    div()
        .h(px(32.))
        .px(px(12.))
        .flex()
        .items_center()
        .gap(px(10.))
        .text_color(p.secondary_foreground)
        .child(rail_slot(icon(name, p.secondary_foreground, 14.)))
        .child(rail_label(title))
}

impl Today {
    pub(super) fn close_detail(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.detail_open && self.editor.is_none() {
            self.composer
                .update(cx, |input, cx| input.focus(window, cx));
        }
        self.composer.update(cx, |input, cx| {
            input.set_placeholder("Create a new note", window, cx)
        });
        self.detail_open = false;
        self.detail = None;
        gpui_kit::base::TextSelection::clear(window, cx);
        cx.notify();
    }
    fn render_source(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        div()
            .id("source-control")
            .flex_shrink_0()
            .flex()
            .items_center()
            .gap(px(4.))
            .on_click(|_, _, cx| cx.stop_propagation())
            .child(
                Button::new("only-mine")
                    .label("Only mine")
                    .accessibility_label("Only mine")
                    .ghost()
                    .small()
                    .selected(self.source.human_only)
                    .toggled(self.source.human_only)
                    .disabled(!self.source.ready || self.editor.is_some())
                    .on_click(cx.listener(|this, _, window, cx| {
                        cx.stop_propagation();
                        this.toggle_source(window, cx);
                    })),
            )
            .children(self.source.error.as_ref().map(|error| {
                Button::new("retry-source")
                    .label("Not saved")
                    .text_color(Theme::global(cx).danger)
                    .icon(IconName::RefreshCw)
                    .ghost()
                    .small()
                    .accessibility_label("Retry saving Only mine")
                    .tooltip(error.clone())
                    .on_click(cx.listener(|this, _, _, cx| {
                        cx.stop_propagation();
                        this.services.source.choose(this.source.human_only);
                    }))
            }))
    }
    fn drop_target(
        &self,
        element: gpui_kit::Stateful<gpui_kit::Div>,
        destination: Destination,
        p: ColorTokens,
        cx: &Context<Self>,
    ) -> gpui_kit::Stateful<gpui_kit::Div> {
        let weak = cx.entity().downgrade();
        let target = destination.clone();
        element
            .can_drop(move |value, window, cx| {
                let Some(drag) = value.downcast_ref::<NoteDrag>() else {
                    return false;
                };
                weak.update(cx, |this, cx| this.accepts_drop(drag, &target, window, cx))
                    .unwrap_or(false)
            })
            .drag_over::<NoteDrag>(move |style, _, _, _| {
                style.bg(p.accent).border_1().border_color(p.primary)
            })
            .on_drop(cx.listener(move |this, drag: &NoteDrag, window, cx| {
                this.drop_note(drag, &destination, window, cx);
            }))
    }
    fn render_collection(
        &self,
        id: &'static str,
        title: &'static str,
        name: IconName,
        destination: Destination,
        p: ColorTokens,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let selected = self.destination == destination;
        let target = destination.clone();
        let row = div()
            .id(id)
            .role(Role::Button)
            .aria_label(title)
            .aria_selected(selected)
            .h(px(32.))
            .px(px(12.))
            .flex()
            .items_center()
            .gap(px(10.))
            .text_color(p.secondary_foreground)
            .when(selected, |d| d.bg(p.selection))
            .hover(move |d| d.bg(if selected { p.selection } else { p.accent }))
            .child(rail_slot(icon(name, p.secondary_foreground, 14.)))
            .child(rail_label(title))
            .on_click(cx.listener(move |this, _, window, cx| {
                cx.stop_propagation();
                this.change_destination(destination.clone(), window, cx);
            }));
        self.drop_target(row, target, p, cx).test_support()
    }
    fn render_home(&self, p: ColorTokens, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        div()
            .id("home")
            .test_support()
            .role(Role::Button)
            .aria_label("Home")
            .aria_selected(self.destination == Destination::Home)
            .h(px(32.))
            .px(px(12.))
            .when(self.destination == Destination::Home, |d| d.bg(p.selection))
            .hover(|d| {
                d.bg(if self.destination == Destination::Home {
                    p.selection
                } else {
                    p.accent
                })
            })
            .flex()
            .items_center()
            .gap(px(10.))
            .text_color(p.foreground)
            .child(rail_slot(icon(IconName::House, p.foreground, 14.)))
            .child(div().flex_1().child(rail_label("Home")))
            .child(
                div()
                    .id("home-accessory")
                    .w(px(20.))
                    .flex_shrink_0()
                    .text_center()
                    .test_support()
                    .text_color(p.muted_foreground)
                    .text_size(px(12.))
                    .child("⌘1"),
            )
            .on_click(cx.listener(|this, _, window, cx| {
                cx.stop_propagation();
                this.change_destination(Destination::Home, window, cx);
            }))
    }
    fn render_project(
        &self,
        index: usize,
        project: &flicknote_sync::today::ProjectContext,
        p: ColorTokens,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let destination = Destination::Project(project.id.clone());
        let selected = self.destination == destination;
        let target = destination.clone();
        self.drop_target(
            div()
                .id(gpui_kit::SharedString::from(format!(
                    "project-{}",
                    project.id
                )))
                .role(Role::Button)
                .aria_label(project.name.clone())
                .aria_selected(selected)
                .when(
                    !selected
                        && !cx.has_active_drag()
                        && self.related_project() == Some(project.id.as_str()),
                    |d| d.bg(p.accent),
                )
                .when(selected, |d| d.bg(p.selection))
                .hover(|d| d.bg(if selected { p.selection } else { p.accent }))
                .h(px(32.))
                .px(px(12.))
                .flex()
                .items_center()
                .gap(px(10.))
                .text_color(if selected {
                    p.foreground
                } else {
                    p.secondary_foreground
                })
                .child(rail_slot(dot(
                    project.color.as_deref().unwrap_or(""),
                    8.,
                    p.secondary_foreground,
                )))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .child(rail_label(&project.name)),
                )
                .children((index < 8).then(|| {
                    div()
                        .id(gpui_kit::SharedString::from(format!(
                            "project-accessory-{}",
                            project.id
                        )))
                        .w(px(20.))
                        .flex_shrink_0()
                        .text_center()
                        .test_support()
                        .text_size(px(12.))
                        .text_color(p.muted_foreground)
                        .child(format!("⌘{}", index + 2))
                }))
                .on_click(cx.listener(move |this, _, window, cx| {
                    cx.stop_propagation();
                    this.change_destination(destination.clone(), window, cx);
                })),
            target,
            p,
            cx,
        )
        .test_support()
    }
    fn projects_heading(&self, p: ColorTokens, cx: &Context<Self>) -> impl IntoElement + use<> {
        div()
            .id("projects-heading")
            .test_support()
            .pl(px(12.))
            .pr(px(8.))
            .mt(px(12.))
            .mb(px(5.))
            .flex()
            .items_center()
            .justify_between()
            .text_size(px(12.))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(p.muted_foreground)
            .child("Projects")
            .child(
                Button::new("add-project")
                    .child(
                        div()
                            .id("project-plus-glyph")
                            .size(px(12.))
                            .flex_shrink_0()
                            .child(icon(IconName::Plus, p.secondary_foreground, 12.))
                            .test_support(),
                    )
                    .text_color(p.secondary_foreground)
                    .ghost()
                    .small()
                    .w(px(28.))
                    .h(px(28.))
                    .accessibility_label("Add project")
                    .tooltip("Add project")
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.edit(project_editor::Kind::Add, window, cx);
                    })),
            )
    }
    fn render_rail(&self, p: ColorTokens, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("navigation-rail")
            .w(px(RAIL_WIDTH))
            .h_full()
            .flex_shrink_0()
            .flex()
            .flex_col()
            .border_r_1()
            .border_color(p.border)
            .bg(p.secondary)
            .text_size(px(14.))
            .child(
                div()
                    .id("workspace-heading")
                    .test_support()
                    .h(px(HEADER_HEIGHT))
                    .flex_shrink_0()
                    .px(px(12.))
                    .border_b_1()
                    .border_color(p.border)
                    .flex()
                    .gap(px(10.))
                    .items_center()
                    .text_color(p.secondary_foreground)
                    .child("Workspace"),
            )
            .child(
                div()
                    .id("rail-destinations")
                    .test_support()
                    .bg(p.secondary)
                    .pb(px(8.))
                    .min_h_0()
                    .flex_1()
                    .overflow_y_scroll()
                    .flex()
                    .flex_col()
                    .gap(px(0.))
                    .child(self.render_home(p, cx))
                    .child(self.projects_heading(p, cx))
                    .children(
                        self.projects
                            .iter()
                            .enumerate()
                            .map(|(index, project)| self.render_project(index, project, p, cx)),
                    )
                    .child(div().mt(px(12.)).child(self.render_collection(
                        "shared",
                        "Shared",
                        IconName::Link,
                        Destination::Shared,
                        p,
                        cx,
                    )))
                    .child(self.render_collection(
                        "archive-destination",
                        "Archive",
                        IconName::Archive,
                        Destination::Archive,
                        p,
                        cx,
                    ))
                    .child(landmark("Charts", IconName::ChartBar, p)),
            )
            .test_support()
    }
    fn render_sync_progress(&self, p: ColorTokens) -> Option<impl IntoElement> {
        self.sync_progress.map(|progress| {
            let bar = match progress {
                crate::sync_progress::Progress::Indeterminate => {
                    Progress::new("first-sync-bar").loading(true)
                }
                crate::sync_progress::Progress::Percent(value) => {
                    Progress::new("first-sync-bar").value(f32::from(value))
                }
            };
            div()
                .id("first-sync-progress")
                .test_support()
                .px(px(8.))
                .py(px(4.))
                .child(
                    bar.color(p.primary)
                        .h(px(4.))
                        .accessibility_label("First sync progress"),
                )
        })
    }

    fn render_list(&self, p: ColorTokens, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("today-notes")
            .role(Role::ListBox)
            .aria_label(if self.destination == Destination::Home {
                "Today notes"
            } else {
                "Project All notes"
            })
            .w_full()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .flex()
            .flex_col()
            .child(self.render_list_status(p, cx))
            .child(
                uniform_list(
                    "today-list",
                    self.model.rows.len(),
                    cx.processor(move |this, range: std::ops::Range<usize>, _, cx| {
                        range
                            .map(|index| this.render_row(index, p, cx))
                            .collect::<Vec<_>>()
                    }),
                )
                .track_scroll(&self.list_scroll)
                .w_full()
                .flex_1()
                .min_h_0(),
            )
            .test_support()
    }
    fn render_list_status(&self, p: ColorTokens, cx: &mut Context<Self>) -> impl IntoElement {
        let capture = self.model.capture();
        div()
            .id("list-status")
            .test_support()
            .w_full()
            .max_h(px(128.))
            .flex_shrink_0()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .children(
                capture
                    .pending
                    .iter()
                    .filter(|pending| self.pending_visible(pending))
                    .rev()
                    .map(|pending| {
                        div()
                            .id(("pending", pending.token))
                            .w_full()
                            .h(px(32.))
                            .flex_shrink_0()
                            .child(preview(
                                &flicknote_sync::today::fold_preview(&pending.text),
                                "normal",
                                None,
                                p,
                            ))
                            .test_support()
                    }),
            )
            .children(
                (self.model.rows.is_empty()
                    && !capture.pending.iter().any(|p| self.pending_visible(p)))
                .then(|| {
                    div()
                        .px(px(8.))
                        .py(px(12.))
                        .text_size(px(14.))
                        .text_color(p.secondary_foreground)
                        .child(if self.watch_error.is_some() {
                            "Notes unavailable"
                        } else if !self.loaded {
                            "Loading notes…"
                        } else if self.services.real_account && !self.first_synced {
                            "Waiting for the first sync…"
                        } else if self.destination == Destination::Home {
                            if matches!(self.period, Period::Day(None)) {
                                "No notes today"
                            } else {
                                "No notes on this day"
                            }
                        } else {
                            match self.destination {
                                Destination::Shared => "No shared notes",
                                Destination::Archive => "No archived notes",
                                _ => "No active notes in this project",
                            }
                        })
                }),
            )
            .children(self.render_sync_progress(p))
            .children(self.sync_message.clone().map(|message| {
                div()
                    .id("sync-message")
                    .test_support()
                    .px(px(8.))
                    .text_size(px(12.))
                    .text_color(p.secondary_foreground)
                    .child(message)
            }))
            .children(self.watch_error.clone().map(|error| {
                div()
                    .p_2()
                    .text_color(p.destructive)
                    .text_size(px(12.))
                    .child(error)
                    .child(
                        Button::new("retry-watch").label("Retry notes").on_click(
                            cx.listener(|this, _, window, cx| this.subscribe(window, cx)),
                        ),
                    )
            }))
    }
    fn render_row(
        &self,
        index: usize,
        p: ColorTokens,
        cx: &mut Context<Self>,
    ) -> gpui_kit::AnyElement {
        let row = &self.model.rows[index];
        let id = row.id;
        div()
            .id(("note", id as u64))
            .role(Role::ListBoxOption)
            .aria_selected(self.model.selected == Some(id))
            .aria_label(format!("Note {id}: {}", row.preview))
            .h(px(32.))
            .w_full()
            .when(self.model.selected == Some(id), |row| row.bg(p.selection))
            .hover(|row| {
                row.bg(if self.model.selected == Some(id) {
                    p.selection
                } else {
                    p.accent
                })
            })
            .child(preview(
                &row.preview,
                &row.note_type,
                row.project_color.as_deref(),
                p,
            ))
            .on_hover(cx.listener(move |this, hovered, _, cx| {
                if *hovered {
                    this.related_note = Some(id);
                } else if this.related_note == Some(id) {
                    this.related_note = None;
                }
                cx.notify();
            }))
            .when(self.drag_eligible(row, cx), |d| {
                d.on_drag(
                    NoteDrag {
                        row: row.clone(),
                        owner: self.services.user_id.clone(),
                    },
                    |drag, _, _, cx| cx.new(|_| drag.clone()),
                )
            })
            .on_click(cx.listener(move |this, _, window, cx| {
                cx.stop_propagation();
                this.select(id, window, cx);
            }))
            .test_support()
            .into_any_element()
    }
    fn share_actions(
        &self,
        row: &flicknote_sync::today::TodayRow,
        disabled: bool,
        cx: &Context<Self>,
    ) -> Vec<gpui_kit::AnyElement> {
        if row.archived || row.draft {
            return vec![];
        }
        let label = if row.shared {
            "Copy share link"
        } else {
            "Share note"
        };
        let target = row.clone();
        let mut buttons = vec![
            Button::new("share-note")
                .icon(IconName::Link)
                .ghost()
                .small()
                .accessibility_label(label)
                .tooltip(label)
                .disabled(disabled)
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.perform_note_action(target.clone(), NoteAction::Share, window, cx)
                }))
                .into_any_element(),
        ];
        if row.shared {
            let target = row.clone();
            buttons.push(
                Button::new("unshare-note")
                    .icon(IconName::Unlink)
                    .ghost()
                    .small()
                    .accessibility_label("Unshare note")
                    .tooltip("Unshare note")
                    .disabled(disabled)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.perform_note_action(target.clone(), NoteAction::Unshare, window, cx)
                    }))
                    .into_any_element(),
            );
        }
        buttons
    }
    fn render_detail(
        &self,
        p: ColorTokens,
        width: f32,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let reading = self.detail.as_ref().expect("open detail state");
        let row = self
            .model
            .rows
            .iter()
            .find(|row| row.uuid == reading.uuid)
            .expect("selected watched note");
        let disabled = self.note_busy(&row.uuid) || !self.drag_allowed;
        let content = row.content.clone();
        let target = row.clone();
        let lifecycle = if row.archived {
            NoteAction::Restore
        } else {
            NoteAction::Archive
        };
        let (action_icon, label, tooltip) = if row.archived {
            (IconName::ArchiveRestore, "Restore note", "Restore note")
        } else {
            (IconName::Archive, "Archive note", "Archive note (⌥A)")
        };
        div()
            .id("detail-surface")
            .w(px(width))
            .h_full()
            .min_h_0()
            .flex_shrink_0()
            .border_l_1()
            .border_color(p.border)
            .bg(p.surface)
            .flex()
            .flex_col()
            .child(
                div()
                    .flex()
                    .justify_end()
                    .items_center()
                    .id("detail-toolbar")
                    .h(px(HEADER_HEIGHT))
                    .flex_shrink_0()
                    .border_b_1()
                    .border_color(p.border)
                    .gap(px(4.))
                    .px(px(8.))
                    .test_support()
                    .child(
                        div()
                            .id("detail-id")
                            .flex_1()
                            .text_color(p.muted_foreground)
                            .child(format!("#{}", row.id))
                            .test_support(),
                    )
                    .child(
                        Button::new("copy-detail")
                            .icon(IconName::Copy)
                            .ghost()
                            .small()
                            .accessibility_label("Copy note")
                            .tooltip("Copy note")
                            .on_click(move |_, _, cx| {
                                cx.write_to_clipboard(ClipboardItem::new_string(content.clone()));
                            }),
                    )
                    .children(self.share_actions(row, disabled, cx))
                    .child(
                        Button::new("archive")
                            .icon(action_icon)
                            .ghost()
                            .small()
                            .accessibility_label(label)
                            .tooltip(tooltip)
                            .disabled(disabled)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.perform_note_action(
                                    target.clone(),
                                    lifecycle.clone(),
                                    window,
                                    cx,
                                );
                            })),
                    )
                    .child(
                        Button::new("close-detail")
                            .icon(IconName::X)
                            .ghost()
                            .small()
                            .accessibility_label("Close detail")
                            .tooltip("Close detail")
                            .on_click(
                                cx.listener(|this, _, window, cx| this.close_detail(window, cx)),
                            ),
                    ),
            )
            .child(render_reading(reading, row, p, cx))
            .test_support()
    }
    fn render_composer(&self, p: ColorTokens, cx: &mut Context<Self>) -> impl IntoElement {
        let capture = self.model.capture();
        div()
            .id("composer-surface")
            .w_full()
            .flex_shrink_0()
            .p(px(12.))
            .border_t_1()
            .border_color(p.border)
            .bg(p.surface)
            .flex()
            .flex_col()
            .gap(px(8.))
            .on_click(|_, _, cx| cx.stop_propagation())
            .child(
                Textarea::new(&self.composer)
                    .accessibility_id("composer")
                    .aria_label(
                        if self.detail.as_ref().is_some_and(|detail| {
                            self.model
                                .rows
                                .iter()
                                .any(|row| row.uuid == detail.uuid && !row.archived)
                        }) {
                            "Append to note"
                        } else {
                            "New note"
                        },
                    )
                    .appearance(false)
                    .bordered(false)
                    .text_size(px(15.)),
            )
            .child(
                div()
                    .id("capture-feedback")
                    .test_support()
                    .max_h(px(120.))
                    .overflow_y_scroll()
                    .flex()
                    .flex_col()
                    .children(self.error.clone().map(|error| {
                        div()
                            .flex_shrink_0()
                            .text_size(px(12.))
                            .text_color(p.destructive)
                            .child(error)
                    }))
                    .children(append_recovery(&capture.appends, p))
                    .children(capture.uncertain.last().map(|(text, error)| {
                        let id = error
                            .details
                            .as_ref()
                            .and_then(|d| d["note_id"].as_str())
                            .unwrap_or("unavailable");
                        let text = text.clone();
                        div()
                            .flex_shrink_0()
                            .text_size(px(12.))
                            .text_color(p.secondary_foreground)
                            .child(format!("Creation reference: {id}. Do not submit it again."))
                            .child(
                                Button::new("copy-uncertain")
                                    .label("Copy captured text")
                                    .ghost()
                                    .on_click(move |_, _, cx| {
                                        cx.write_to_clipboard(ClipboardItem::new_string(
                                            text.clone(),
                                        ))
                                    }),
                            )
                    }))
                    .children((!capture.recovery.is_empty()).then(|| {
                        Button::new("recover")
                            .flex_shrink_0()
                            .label("Recover unsaved text")
                            .ghost()
                            .on_click(cx.listener(|this, _, window, cx| {
                                if this.composer.read(cx).value().is_empty()
                                    && !this.composing(window, cx)
                                    && let Some(text) = this.model.capture().recovery.pop()
                                {
                                    this.composer
                                        .update(cx, |input, cx| input.set_value(text, window, cx));
                                    this.error = None;
                                    cx.notify();
                                }
                            }))
                    })),
            )
            .test_support()
    }
    fn render_period_controls(&self, cx: &mut Context<Self>) -> Option<impl IntoElement + use<>> {
        if matches!(self.destination, Destination::Shared | Destination::Archive) {
            return None;
        }
        let temporal = self.period != Period::All;
        let project = matches!(self.destination, Destination::Project(_));
        let forward = self.period.shifted(true, &chrono::Local::now()).is_some();
        Some(
            div()
                .id("period-header")
                // Reserve the Week range line in All too, keeping the list start fixed.
                .when(project, |d| d.h(px(52.)))
                .on_click(|_, _, cx| cx.stop_propagation())
                .flex_shrink_0()
                .px(px(8.))
                .py(px(2.))
                .flex()
                .flex_col()
                .gap(px(2.))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(2.))
                        .when(project, |d| {
                            d.child(
                                Button::new("scope-all")
                                    .label("All")
                                    .accessibility_label("All dates")
                                    .small()
                                    .ghost()
                                    .selected(!temporal)
                                    .toggled(!temporal)
                                    .on_click(cx.listener(|this, _, w, cx| {
                                        this.project_scope(false, w, cx)
                                    })),
                            )
                            .child(
                                Button::new("scope-week")
                                    .label("Week")
                                    .accessibility_label("Week")
                                    .small()
                                    .ghost()
                                    .selected(temporal)
                                    .toggled(temporal)
                                    .on_click(cx.listener(|this, _, w, cx| {
                                        this.project_scope(true, w, cx)
                                    })),
                            )
                        })
                        .when(temporal, |d| {
                            d.child(
                                Button::new("period-back")
                                    .icon(IconName::ChevronLeft)
                                    .accessibility_label("Previous period")
                                    .small()
                                    .ghost()
                                    .on_click(cx.listener(|this, _, w, cx| {
                                        this.mouse_period(false, w, cx)
                                    })),
                            )
                            .child(
                                Button::new("period-forward")
                                    .icon(IconName::ChevronRight)
                                    .accessibility_label("Next period")
                                    .small()
                                    .ghost()
                                    .disabled(!forward)
                                    .on_click(
                                        cx.listener(|this, _, w, cx| {
                                            this.mouse_period(true, w, cx)
                                        }),
                                    ),
                            )
                            .child(
                                Button::new("period-current")
                                    .label(if project { "This week" } else { "Today" })
                                    .small()
                                    .ghost()
                                    .on_click(
                                        cx.listener(|this, _, w, cx| this.current_period(w, cx)),
                                    ),
                            )
                        }),
                )
                .when(project && temporal, |d| {
                    d.child(
                        div()
                            .id("period-range")
                            .aria_label(self.period_label())
                            .text_size(px(12.))
                            .truncate()
                            .text_color(Theme::global(cx).muted_foreground)
                            .child(self.period_label())
                            .test_support(),
                    )
                })
                .test_support(),
        )
    }
    fn render_canvas(&self, p: ColorTokens, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        div()
            .id("center-pane")
            .on_click(cx.listener(|this, _, window, cx| this.close_detail(window, cx)))
            .flex_1()
            .min_w_0()
            .h_full()
            .flex()
            .flex_col()
            .child(
                div()
                    .id("destination-header")
                    .h(px(HEADER_HEIGHT))
                    .flex_shrink_0()
                    .px(px(12.))
                    .border_b_1()
                    .border_color(p.border)
                    .flex()
                    .items_center()
                    .text_size(px(14.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .child(match &self.destination {
                                Destination::Home => self.period_label(),
                                Destination::Shared => "Shared".to_string(),
                                Destination::Archive => "Archive".to_string(),
                                Destination::Project(id) => self
                                    .projects
                                    .iter()
                                    .find(|p| &p.id == id)
                                    .map_or("Project", |p| p.name.as_str())
                                    .to_string(),
                            }),
                    )
                    .child(self.render_source(cx))
                    .test_support(),
            )
            .children(self.render_summary(cx))
            .children(self.render_period_controls(cx))
            .child(
                div()
                    .id("main-canvas")
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .child(self.render_list(p, cx))
                    .test_support(),
            )
            .child(self.render_composer(p, cx))
            .test_support()
    }
}
impl Today {
    fn workspace_actions(
        &self,
        navigation_context: &'static str,
        cx: &mut Context<Self>,
    ) -> gpui_kit::Stateful<gpui_kit::Div> {
        div()
            .id("workspace")
            .key_context(navigation_context)
            .on_action(cx.listener(|this, _: &SaveEditor, window, cx| this.save_editor(window, cx)))
            .on_action(
                cx.listener(|this, _: &Project2, window, cx| this.select_number(2, window, cx)),
            )
            .on_action(
                cx.listener(|this, _: &Project3, window, cx| this.select_number(3, window, cx)),
            )
            .on_action(
                cx.listener(|this, _: &Project4, window, cx| this.select_number(4, window, cx)),
            )
            .on_action(
                cx.listener(|this, _: &Project5, window, cx| this.select_number(5, window, cx)),
            )
            .on_action(
                cx.listener(|this, _: &Project6, window, cx| this.select_number(6, window, cx)),
            )
            .on_action(
                cx.listener(|this, _: &Project7, window, cx| this.select_number(7, window, cx)),
            )
            .on_action(
                cx.listener(|this, _: &Project8, window, cx| this.select_number(8, window, cx)),
            )
            .on_action(
                cx.listener(|this, _: &Project9, window, cx| this.select_number(9, window, cx)),
            )
            .on_action(cx.listener(|this, _: &PreviousPeriod, window, cx| {
                this.move_period(false, window, cx)
            }))
            .on_action(
                cx.listener(|this, _: &NextPeriod, window, cx| this.move_period(true, window, cx)),
            )
            .on_action(cx.listener(|this, _: &NextDestination, window, cx| {
                this.traverse_destination(true, window, cx)
            }))
            .on_action(cx.listener(|this, _: &PreviousDestination, window, cx| {
                this.traverse_destination(false, window, cx)
            }))
            .on_action(
                cx.listener(|this, _: &NextNote, window, cx| this.navigate(true, window, cx)),
            )
            .on_action(
                cx.listener(|this, _: &PreviousNote, window, cx| this.navigate(false, window, cx)),
            )
            .on_action(cx.listener(|this, _: &ArchiveNote, window, cx| this.archive(window, cx)))
    }
}
impl Render for Today {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.visible && window.is_visible() {
            self.frames += 1;
        }
        self.visible = window.is_visible();
        self.drag_allowed = !self.shortcuts_blocked(window, cx);
        let navigation_context = if self.editor.is_some() {
            "WorkspaceEditor"
        } else if self.shortcuts_blocked(window, cx) {
            "Today"
        } else {
            "Today destination_navigation"
        };
        let theme = Theme::global(cx);
        let p = ColorTokens {
            selection: theme.list_active,
            ..theme.color_tokens()
        };
        let viewport = window.viewport_size();
        let detail_width = reading_width(f32::from(viewport.width));
        self.workspace_actions(navigation_context, cx)
            .relative()
            .size_full()
            .bg(p.background)
            .text_color(p.foreground)
            .font_family(".SystemUIFont")
            .capture_action(cx.listener(
                |this, action: &gpui_kit::base::input::Enter, window, cx| {
                    this.enter_composing = if this.editor.is_some() {
                        this.editor_composing(window, cx)
                    } else {
                        this.composing(window, cx)
                    };
                    if action.secondary
                        && !action.shift
                        && !this.enter_composing
                        && this
                            .editor
                            .as_ref()
                            .is_some_and(|e| matches!(e.kind, project_editor::Kind::Summary(_)))
                    {
                        this.save_editor(window, cx);
                        return;
                    }
                    cx.propagate();
                },
            ))
            .capture_action(
                cx.listener(|this, _: &gpui_kit::base::input::Escape, window, cx| {
                    if cx.stop_active_drag(window) {
                        cx.stop_propagation();
                        return;
                    }
                    this.escape_composing = if this.editor.is_some() {
                        this.editor_composing(window, cx)
                    } else {
                        this.composing(window, cx)
                    };
                    cx.propagate();
                }),
            )
            .on_action(
                cx.listener(|this, _: &gpui_kit::base::input::Escape, window, cx| {
                    if !this.escape_composing {
                        if this.editor.is_some() {
                            this.cancel_editor(window, cx);
                        } else {
                            this.close_detail(window, cx);
                        }
                    }
                    this.escape_composing = false;
                }),
            )
            .on_action(cx.listener(|this, _: &Reopen, window, cx| {
                this.home_today(window, cx);
            }))
            .child(
                div()
                    .size_full()
                    .flex()
                    .child(self.render_rail(p, cx))
                    .child(self.render_canvas(p, cx))
                    .children(
                        (self.detail_open && self.model.selected.is_some())
                            .then(|| self.render_detail(p, detail_width, cx)),
                    ),
            )
            .when(self.editor.is_none(), |d| {
                d.child(crate::native_input::install(self.composer.clone()))
            })
            .children(self.render_editor(cx))
    }
}

fn render_reading(
    reading: &Reading,
    row: &flicknote_sync::today::TodayRow,
    p: ColorTokens,
    cx: &App,
) -> impl IntoElement + use<> {
    div()
        .id(("detail-reading", reading.state.entity_id()))
        .flex_1()
        .min_h_0()
        .min_w_0()
        .overflow_y_scroll()
        .track_scroll(&reading.scroll)
        .p(px(16.))
        .text_size(px(15.))
        .children(
            row.title
                .as_ref()
                .filter(|title| !title.trim().is_empty())
                .map(|title| {
                    div()
                        .id("detail-title")
                        .w_full()
                        .min_w_0()
                        .mb(px(8.))
                        .text_size(px(20.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(title.clone())
                        .test_support()
                }),
        )
        .children(row.project_name.as_ref().map(|name| {
            div()
                .id("detail-project")
                .flex()
                .items_start()
                .gap(px(8.))
                .mb(px(16.))
                .text_color(p.secondary_foreground)
                .child(div().pt(px(7.)).child(dot(
                    row.project_color.as_deref().unwrap_or(""),
                    5.,
                    p.secondary_foreground,
                )))
                .child(div().flex_1().min_w_0().child(name.clone()))
                .test_support()
        }))
        .child(reader(&reading.state, cx))
        .test_support()
}

/// Base uses the component-installed theme defaults and Root's existing selection layer.
/// The facade omits the image resolver; use its public Base implementation directly.
fn reader(state: &Entity<gpui_kit::base::TextViewState>, cx: &App) -> impl IntoElement {
    let theme = Theme::global(cx);
    let mut table = gpui_kit::StyleRefinement::default();
    table.overflow.x = Some(gpui_kit::Overflow::Scroll);
    let mut code = gpui_kit::StyleRefinement::default();
    code.padding.top = Some(px(36.).into());
    let style = gpui_kit::base::TextViewStyle::from_theme(&gpui_kit::base::Theme::global(cx))
        .with_foreground(theme.foreground)
        .with_muted_foreground(theme.muted_foreground)
        .with_link(theme.link)
        .with_selection(theme.selection)
        .with_code_background(theme.muted)
        .with_border(theme.border)
        .with_table(table)
        .with_code_block(code)
        .with_dark(theme.is_dark());
    let selection = state.clone();
    div()
        .w_full()
        .min_w_0()
        .capture_action(move |_: &gpui_kit::base::input::Copy, _, cx| {
            let text = selection.read(cx).selected_text();
            if text.is_empty() {
                cx.propagate();
                return;
            }
            cx.write_to_clipboard(ClipboardItem::new_string(text));
            cx.stop_propagation();
        })
        .child(
            gpui_kit::base::TextView::new(state)
                .style(style)
                .selectable(true)
                .selection_format(gpui_kit::base::SelectionFormat::Plain)
                .image_source(suppressed_image)
                .code_block_actions(|block, _, _| {
                    let code = block.code();
                    let start = block.span.map_or(0, |span| span.start);
                    // Kit scopes this action's element ID to its containing code block.
                    Button::new(gpui_kit::SharedString::from(format!("copy-code-{start}")))
                        .small()
                        .icon(IconName::Copy)
                        .ghost()
                        .accessibility_label("Copy code block")
                        .tooltip("Copy code block")
                        .on_click(move |_, _, cx| {
                            cx.write_to_clipboard(ClipboardItem::new_string(code.to_string()));
                        })
                })
                .on_link_click(|url, _, _, cx| activate_link(url, |url| cx.open_url(url)))
                .w_full()
                .min_w_0(),
        )
}

fn suppressed_image(_: &gpui_kit::SharedUri) -> gpui_kit::ImageSource {
    // A failed custom source is authoritative, including for intrinsic measurement.
    // It cannot fall back to a document URI, embedded data, or filesystem path.
    gpui_kit::ImageSource::Custom(Arc::new(|_, _| {
        Some(Err(gpui_kit::ImageCacheError::Other(Arc::new(
            anyhow::anyhow!("Images are deferred"),
        ))))
    }))
}

fn activate_link(url: &str, open: impl FnOnce(&str)) {
    if let Some((scheme, address)) = url.split_once(':')
        && (scheme.eq_ignore_ascii_case("http") || scheme.eq_ignore_ascii_case("https"))
        && address
            .strip_prefix("//")
            .is_some_and(|rest| !rest.is_empty() && !rest.starts_with(['/', '?', '#']))
        && !url.chars().any(|c| c.is_control() || c.is_whitespace())
    {
        open(url);
    }
}

fn append_recovery(appends: &crate::append::Appends, p: ColorTokens) -> Vec<gpui_kit::AnyElement> {
    appends
        .recovery
        .iter()
        .map(|recovery| {
            let text = recovery.text.clone();
            let guidance = if recovery.uncertain {
                "Check the note before resubmitting."
            } else {
                "Submitted text is recoverable."
            };
            div()
                .flex_shrink_0()
                .text_size(px(12.))
                .text_color(p.secondary_foreground)
                .child(format!(
                    "Append #{}: {} {guidance}",
                    recovery.id, recovery.message
                ))
                .child(
                    Button::new(gpui_kit::SharedString::from(format!(
                        "copy-append-{}",
                        recovery.token
                    )))
                    .label("Copy submitted text")
                    .ghost()
                    .on_click(move |_, _, cx| {
                        cx.write_to_clipboard(ClipboardItem::new_string(text.clone()));
                    }),
                )
                .into_any_element()
        })
        .collect()
}

#[cfg(test)]
mod resource_tests {
    use super::*;
    use gpui_kit::test::TestWindowExt;

    #[gpui_kit::test]
    fn plain_markdown_pointer_release(cx: &mut gpui_kit::TestAppContext) {
        pointer_release(cx);
    }
    fn pointer_release(cx: &mut gpui_kit::TestAppContext) {
        use gpui_kit::{InputEvent as _, MouseMoveEvent, point};
        cx.executor().allow_parking();
        cx.update(gpui_kit::init);
        struct Plain {
            state: Entity<gpui_kit::base::TextViewState>,
        }
        impl Render for Plain {
            fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                div().id("plain-reader").w(px(300.)).child(
                    gpui_kit::base::TextView::new(&self.state)
                        .selectable(true)
                        .selection_format(gpui_kit::base::SelectionFormat::Plain),
                )
            }
        }
        let (window, view) = cx.update(|cx| {
            gpui_kit::open_window(Default::default(), cx, |_, cx| {
                cx.new(|cx| Plain {
                    state: cx
                        .new(|cx| gpui_kit::base::TextViewState::markdown("# 中文 Heading", cx)),
                })
            })
            .unwrap()
        });
        super::super::tests::settle(cx, |cx| {
            cx.update(|cx| !view.read(cx).state.read(cx).rendered_text().is_empty())
        });
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            let state = view.read(cx).state.clone();
            let origin = state.read(cx).bounds().origin;
            super::super::markdown_tests::drag_text(
                window,
                cx,
                origin + point(px(2.), px(10.)),
                origin + point(px(90.), px(10.)),
            );
            let released = state.read(cx).selected_text();
            assert!(!released.is_empty());
            window.press("cmd-c", cx);
            let copied = cx.read_from_clipboard().unwrap().text().unwrap();
            assert!(!copied.is_empty());
            for x in [140., 180.] {
                window.dispatch_event(
                    MouseMoveEvent {
                        position: origin + point(px(x), px(10.)),
                        pressed_button: None,
                        modifiers: Default::default(),
                    }
                    .to_platform_input(),
                    cx,
                );
                window.render_frame(cx);
                assert_eq!(
                    state.read(cx).selected_text(),
                    released,
                    "plain pinned TextView release"
                );
                window.press("cmd-c", cx);
                assert_eq!(cx.read_from_clipboard().unwrap().text().unwrap(), copied);
            }
            window.remove_window();
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn document_resources_have_no_implicit_effects(cx: &mut gpui_kit::TestAppContext) {
        cx.executor().allow_parking();
        cx.update(gpui_kit::init);
        let mut opened = Vec::new();
        for url in ["https://example.invalid/page", "HTTP://example.invalid"] {
            activate_link(url, |url| opened.push(url.to_owned()));
        }
        assert_eq!(opened.len(), 2);
        for url in [
            "file:///tmp/owned",
            "data:image/png;base64,bad",
            "javascript:alert(1)",
            "mailto:test@example.invalid",
            "custom://host",
            "//example.invalid",
            "https:",
            "https:///path",
            " https://example.invalid",
            "https://example.invalid\n",
        ] {
            activate_link(url, |_| panic!("rejected URL must not open"));
        }
        // Invoke the actual authoritative resolver, including embedded data and file forms.
        struct Resources {
            state: Entity<gpui_kit::base::TextViewState>,
        }
        impl Render for Resources {
            fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
                div().w(px(300.)).child(reader(&self.state, cx))
            }
        }
        let (window, view) = cx.update(|cx| gpui_kit::open_window(Default::default(), cx, |_, cx| {
            cx.new(|cx| Resources { state: cx.new(|cx| gpui_kit::base::TextViewState::markdown("Owned resources\n\n![remote](https://example.invalid/a.png)\n\n![local](file:///tmp/owned-image)\n\n![data](data:image/png;base64,bad)", cx)) })
        }).unwrap());
        super::super::tests::settle(cx, |cx| {
            cx.update(|cx| {
                view.read(cx)
                    .state
                    .read(cx)
                    .rendered_text()
                    .as_str()
                    .contains("Owned resources")
            })
        });
        cx.update_window(window, |_, window, cx| {
            for uri in [
                "https://example.invalid/a.png",
                "http://example.invalid/b.png",
                "file:///tmp/owned-image",
                "/tmp/owned-image",
                "data:image/png;base64,bad",
            ] {
                let gpui_kit::ImageSource::Custom(load) = suppressed_image(&uri.into()) else {
                    panic!("resource source would permit I/O")
                };
                assert!(load(window, cx).unwrap().is_err());
            }
            window.render_frame(cx);
            assert!(view.read(cx).state.read(cx).bounds().size.width > px(0.));
            window.remove_window();
        })
        .unwrap();
    }
}
