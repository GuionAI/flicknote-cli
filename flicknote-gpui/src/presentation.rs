use super::*;
use crate::workspace::COMPOSER_CLEARANCE;
use gpui_kit::assets::IconName;
use gpui_kit::base::ColorTokens;
use gpui_kit::base::{Disableable, TestSupportExt};
use gpui_kit::component::{
    Icon,
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
        .child(div().flex_1().min_w_0().truncate().child(text.to_owned()))
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
        .h(px(36.))
        .px(px(13.))
        .flex()
        .items_center()
        .gap(px(10.))
        .text_color(p.secondary_foreground)
        .child(rail_slot(icon(name, p.secondary_foreground, 14.)))
        .child(rail_label(title))
}

impl Today {
    pub(super) fn close_detail(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.detail_open {
            self.composer
                .update(cx, |input, cx| input.focus(window, cx));
        }
        self.detail_open = false;
        cx.notify();
    }
    fn render_home(&self, p: ColorTokens, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        div()
            .id("home")
            .role(Role::Button)
            .aria_label("Home")
            .aria_selected(self.destination == Destination::Home)
            .h(px(36.))
            .px(px(13.))
            .rounded(px(9.))
            .when(self.destination == Destination::Home, |d| d.bg(p.selection))
            .hover(|d| d.bg(p.accent))
            .flex()
            .items_center()
            .gap(px(10.))
            .text_color(p.foreground)
            .child(rail_slot(icon(IconName::House, p.foreground, 14.)))
            .child(div().flex_1().child(rail_label("Home")))
            .child(
                div()
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
        div()
            .id(gpui_kit::SharedString::from(format!(
                "project-{}",
                project.id
            )))
            .role(Role::Button)
            .aria_label(project.name.clone())
            .aria_selected(selected)
            .rounded(px(9.))
            .when(selected, |d| d.bg(p.selection))
            .hover(|d| d.bg(p.accent))
            .h(px(36.))
            .px(px(13.))
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
            .child(div().flex_1().child(rail_label(&project.name)))
            .children((index < 8).then(|| {
                div()
                    .text_size(px(12.))
                    .text_color(p.muted_foreground)
                    .child(format!("⌘{}", index + 2))
            }))
            .on_click(cx.listener(move |this, _, window, cx| {
                cx.stop_propagation();
                this.change_destination(destination.clone(), window, cx);
            }))
            .test_support()
    }
    fn render_rail(&self, p: ColorTokens, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("navigation-rail")
            .w(px(252.))
            .h_full()
            .flex_shrink_0()
            .flex()
            .flex_col()
            .rounded(px(16.))
            .border_1()
            .border_color(p.border)
            .bg(p.secondary)
            .text_size(px(14.))
            .child(
                div()
                    .mx(px(14.))
                    .mt(px(20.))
                    .mb(px(14.))
                    .px(px(10.))
                    .py(px(8.))
                    .rounded(px(9.))
                    .bg(p.muted)
                    .flex()
                    .gap(px(8.))
                    .items_center()
                    .text_size(px(15.))
                    .text_color(p.muted_foreground)
                    .child(icon(IconName::Search, p.muted_foreground, 15.))
                    .child("Search notes"),
            )
            .child(
                div()
                    .id("rail-destinations")
                    .mx(px(10.))
                    .min_h_0()
                    .flex_1()
                    .overflow_y_scroll()
                    .flex()
                    .flex_col()
                    .gap(px(3.))
                    .child(self.render_home(p, cx))
                    .child(
                        div()
                            .px(px(13.))
                            .mt(px(18.))
                            .mb(px(5.))
                            .text_size(px(10.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(p.muted_foreground)
                            .child("PROJECTS"),
                    )
                    .children(
                        self.projects
                            .iter()
                            .enumerate()
                            .map(|(index, project)| self.render_project(index, project, p, cx)),
                    )
                    .child(
                        div()
                            .mt(px(20.))
                            .child(landmark("Shared", IconName::Link, p)),
                    )
                    .child(landmark("Archive", IconName::Archive, p))
                    .child(landmark("Charts", IconName::ChartBar, p)),
            )
            .test_support()
    }
    fn render_sync_progress(&self) -> Option<impl IntoElement> {
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
                .child(bar.accessibility_label("First sync progress"))
        })
    }

    fn render_list(&self, p: ColorTokens, cx: &mut Context<Self>) -> impl IntoElement {
        let capture = self.model.capture();
        div()
            .id("today-notes")
            .role(Role::ListBox)
            .aria_label(if self.destination == Destination::Home {
                "Today notes"
            } else {
                "Project All notes"
            })
            .w_full()
            .h_full()
            .min_w_0()
            .min_h_0()
            .flex()
            .flex_col()
            .children(
                capture
                    .pending
                    .iter()
                    .filter(|_| self.destination == Destination::Home)
                    .rev()
                    .map(|pending| {
                        div()
                            .id(("pending", pending.token))
                            .w_full()
                            .h(px(32.))
                            .flex_shrink_0()
                            .child(preview(
                                &pending
                                    .text
                                    .split_whitespace()
                                    .collect::<Vec<_>>()
                                    .join(" "),
                                "normal",
                                None,
                                p,
                            ))
                            .test_support()
                    }),
            )
            .children(
                (self.model.rows.is_empty()
                    && (self.destination != Destination::Home || capture.pending.is_empty()))
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
                            "No notes today"
                        } else {
                            "No active notes in this project"
                        })
                }),
            )
            .children(self.render_sync_progress())
            .children(self.sync_message.clone().map(|message| {
                div()
                    .px(px(8.))
                    .text_size(px(12.))
                    .text_color(p.secondary_foreground)
                    .child(message)
            }))
            .children(self.watch_error.clone().map(|error| {
                div()
                    .p_2()
                    .text_color(p.secondary_foreground)
                    .text_size(px(12.))
                    .child(error)
                    .child(
                        Button::new("retry-watch").label("Retry notes").on_click(
                            cx.listener(|this, _, window, cx| this.subscribe(window, cx)),
                        ),
                    )
            }))
            .child(
                uniform_list(
                    "today-list",
                    self.model.rows.len() + (COMPOSER_CLEARANCE / 32.).ceil() as usize,
                    cx.processor(move |this, range: std::ops::Range<usize>, _, cx| {
                        range
                            .map(|index| this.render_row(index, p, cx))
                            .collect::<Vec<_>>()
                    }),
                )
                .track_scroll(&self.list_scroll)
                .w_full()
                .h_full(),
            )
            .test_support()
    }
    fn render_row(
        &self,
        index: usize,
        p: ColorTokens,
        cx: &mut Context<Self>,
    ) -> gpui_kit::AnyElement {
        // Fixed-height blank tail makes the final note reachable above the composer.
        let Some(row) = self.model.rows.get(index) else {
            return div().h(px(32.)).w_full().into_any_element();
        };
        let id = row.id;
        div()
            .id(("note", id as u64))
            .role(Role::ListBoxOption)
            .aria_selected(self.model.selected == Some(id))
            .aria_label(format!("Note {id}: {}", row.preview))
            .h(px(32.))
            .w_full()
            .rounded(px(6.))
            .when(self.model.selected == Some(id), |row| row.bg(p.selection))
            .hover(|row| row.bg(p.accent))
            .child(preview(
                &row.preview,
                &row.note_type,
                row.project_color.as_deref(),
                p,
            ))
            .on_click(cx.listener(move |this, _, window, cx| {
                cx.stop_propagation();
                this.select(id, window, cx);
            }))
            .test_support()
            .into_any_element()
    }
    fn render_detail(
        &self,
        p: ColorTokens,
        width: f32,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let disabled = self.archive_busy || !self.composer.read(cx).value().is_empty();
        div()
            .id("detail-surface")
            .w(px(width))
            .h_full()
            .min_h_0()
            .rounded(px(16.))
            .border_1()
            .border_color(p.border)
            .bg(p.surface)
            .shadow_xs()
            .flex()
            .flex_col()
            .on_click(|_, _, cx| cx.stop_propagation())
            .child(
                div()
                    .flex()
                    .justify_end()
                    .items_center()
                    .gap(px(12.))
                    .p(px(12.))
                    .child(
                        Button::new("copy-detail")
                            .icon(IconName::Copy)
                            .ghost()
                            .accessibility_label("Copy note")
                            .tooltip("Copy note")
                            .on_click(cx.listener(|this, _, _, cx| {
                                cx.write_to_clipboard(ClipboardItem::new_string(
                                    this.detail.read(cx).value().to_string(),
                                ));
                            })),
                    )
                    .child(
                        Button::new("archive")
                            .icon(IconName::Archive)
                            .ghost()
                            .accessibility_label("Archive note")
                            .tooltip("Archive note (⌥A)")
                            .disabled(disabled)
                            .on_click(cx.listener(|this, _, window, cx| this.archive(window, cx))),
                    )
                    .child(
                        Button::new("close-detail")
                            .icon(IconName::X)
                            .ghost()
                            .accessibility_label("Close detail")
                            .tooltip("Close detail")
                            .on_click(
                                cx.listener(|this, _, window, cx| this.close_detail(window, cx)),
                            ),
                    ),
            )
            .child(
                div().flex_1().min_h_0().px(px(20.)).pb(px(20.)).child(
                    Textarea::new(&self.detail)
                        .accessibility_id("detail")
                        .aria_label("Note detail")
                        .readonly(true)
                        .appearance(false)
                        .bordered(false)
                        .text_size(px(14.))
                        .h_full(),
                ),
            )
            .test_support()
    }
    fn render_composer(&self, p: ColorTokens, cx: &mut Context<Self>) -> impl IntoElement {
        let capture = self.model.capture();
        div()
            .id("composer-surface")
            .w_full()
            .max_w(px(620.))
            .p(px(16.))
            .rounded(px(18.))
            .border_1()
            .border_color(p.border)
            .bg(p.surface)
            .shadow_xs()
            .flex()
            .flex_col()
            .gap(px(8.))
            .on_click(|_, _, cx| cx.stop_propagation())
            .child(
                Textarea::new(&self.composer)
                    .accessibility_id("composer")
                    .aria_label("New note")
                    .appearance(false)
                    .bordered(false)
                    .text_size(px(17.)),
            )
            .children(self.error.clone().map(|error| {
                div()
                    .text_size(px(12.))
                    .text_color(p.secondary_foreground)
                    .child(error)
            }))
            .children(capture.uncertain.last().map(|(text, error)| {
                let id = error
                    .details
                    .as_ref()
                    .and_then(|d| d["note_id"].as_str())
                    .unwrap_or("unavailable");
                let text = text.clone();
                div()
                    .text_size(px(12.))
                    .text_color(p.secondary_foreground)
                    .child(format!("Creation reference: {id}. Do not submit it again."))
                    .child(
                        Button::new("copy-uncertain")
                            .label("Copy captured text")
                            .ghost()
                            .on_click(move |_, _, cx| {
                                cx.write_to_clipboard(ClipboardItem::new_string(text.clone()))
                            }),
                    )
            }))
            .children((!capture.recovery.is_empty()).then(|| {
                Button::new("recover")
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
            }))
            .test_support()
    }
    fn render_canvas(&self, p: ColorTokens, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        div()
            .flex_1()
            .min_w_0()
            .h_full()
            .flex()
            .flex_col()
            .pt(px(6.))
            .gap(px(12.))
            .child(
                div()
                    .px(px(16.))
                    .text_size(px(23.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(match &self.destination {
                        Destination::Home => "Today".to_string(),
                        Destination::Project(id) => format!(
                            "{} — All",
                            self.projects
                                .iter()
                                .find(|p| &p.id == id)
                                .map_or("Project", |p| p.name.as_str())
                        ),
                    }),
            )
            .child(
                div()
                    .id("main-canvas")
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .px(px(16.))
                    .child(self.render_list(p, cx))
                    .test_support(),
            )
    }
    fn render_floating_surfaces(
        &self,
        p: ColorTokens,
        detail_width: f32,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        div()
            .absolute()
            .inset_0()
            // Header and its gutter remain clear of the floating reading surface.
            .pt(px(84.))
            .pb(px(28.))
            .flex()
            .flex_col()
            .gap(px(16.))
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .pr(px(48.))
                    .flex()
                    .justify_end()
                    .children(
                        (self.detail_open && self.model.selected.is_some())
                            .then(|| self.render_detail(p, detail_width, cx)),
                    ),
            )
            .child(
                div()
                    .w_full()
                    .px(px(22.))
                    .flex()
                    .justify_center()
                    .child(self.render_composer(p, cx)),
            )
    }
}
impl Render for Today {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.visible && window.is_visible() {
            self.frames += 1;
        }
        self.visible = window.is_visible();
        let navigation_context = if self.shortcuts_blocked(window, cx) {
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
        // Keep at least 32pt of row content exposed at the minimum window width.
        let detail_width = 520_f32.min(f32::from(viewport.width) - 394.);
        div()
            .id("workspace")
            .key_context(navigation_context)
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
            .relative()
            .size_full()
            .bg(p.background)
            .text_color(p.foreground)
            .font_family(".SystemUIFont")
            .capture_action(
                cx.listener(|this, _: &gpui_kit::base::input::Enter, window, cx| {
                    this.enter_composing = this.composing(window, cx);
                    cx.propagate();
                }),
            )
            .capture_action(
                cx.listener(|this, _: &gpui_kit::base::input::Escape, window, cx| {
                    this.escape_composing = this.composing(window, cx);
                    cx.propagate();
                }),
            )
            .on_action(
                cx.listener(|this, _: &gpui_kit::base::input::Escape, window, cx| {
                    if !this.escape_composing {
                        this.close_detail(window, cx);
                    }
                    this.escape_composing = false;
                }),
            )
            .on_action(cx.listener(|this, _: &Reopen, window, cx| {
                this.change_destination(Destination::Home, window, cx);
            }))
            .on_click(cx.listener(|this, _, window, cx| this.close_detail(window, cx)))
            .child(
                div()
                    .size_full()
                    .flex()
                    .gap(px(24.))
                    .pl(px(22.))
                    .pr(px(32.))
                    .py(px(22.))
                    .child(self.render_rail(p, cx))
                    .child(self.render_canvas(p, cx)),
            )
            .child(self.render_floating_surfaces(p, detail_width, cx))
            .child(crate::native_input::install(self.composer.clone()))
    }
}
