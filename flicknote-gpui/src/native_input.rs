//! The empty workspace composer gives bindings first refusal before native IME.
//! Text, selection, geometry and composition remain owned by the retained Kit input.
use gpui_kit::component::input::TextareaState;
use gpui_kit::{
    App, ElementInputHandler, Entity, Focusable, InputHandler, IntoElement, Window, canvas,
    prelude::*, px,
};
use std::ops::Range;

pub(crate) struct ComposerInput {
    inner: ElementInputHandler<TextareaState>,
    composer: Entity<TextareaState>,
}
impl ComposerInput {
    pub(crate) fn new(composer: Entity<TextareaState>, cx: &App) -> Option<Self> {
        let bounds = composer.read(cx).text_bounds()?;
        Some(Self {
            inner: ElementInputHandler::new(bounds, composer.clone()),
            composer,
        })
    }
}

// Paint after Kit has registered its input handler. Use Kit's own text bounds;
// an outer surface's padding would displace native IME candidate geometry.
pub(crate) fn install(composer: Entity<TextareaState>) -> impl IntoElement {
    canvas(
        |_, _, _| (),
        move |_, (), window, cx| {
            if let Some(handler) = ComposerInput::new(composer.clone(), cx) {
                let focus = composer.read(cx).focus_handle(cx);
                window.handle_input(&focus, handler, cx);
            }
        },
    )
    .absolute()
    .size(px(0.))
}

impl InputHandler for ComposerInput {
    fn prefers_ime_for_printable_keys(&mut self, window: &mut Window, cx: &mut App) -> bool {
        // macOS otherwise consumes Option letters in an active Chinese input
        // source before GPUI can match workspace bindings. Unmatched letters
        // still fall through to inputContext in the pinned native backend.
        self.inner.prefers_ime_for_printable_keys(window, cx)
            && (!self.composer.read(cx).value().is_empty()
                || self.inner.marked_text_range(window, cx).is_some())
    }
    fn apple_press_and_hold_enabled(&mut self) -> bool {
        self.inner.apple_press_and_hold_enabled()
    }
    fn selected_text_range(
        &mut self,
        ignore_disabled_input: bool,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<gpui_kit::UTF16Selection> {
        self.inner
            .selected_text_range(ignore_disabled_input, window, cx)
    }
    fn marked_text_range(&mut self, window: &mut Window, cx: &mut App) -> Option<Range<usize>> {
        self.inner.marked_text_range(window, cx)
    }
    fn text_for_range(
        &mut self,
        range: Range<usize>,
        adjusted_range: &mut Option<Range<usize>>,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<String> {
        self.inner.text_for_range(range, adjusted_range, window, cx)
    }
    fn replace_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.inner.replace_text_in_range(range, text, window, cx)
    }
    fn replace_and_mark_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        selected: Option<Range<usize>>,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.inner
            .replace_and_mark_text_in_range(range, text, selected, window, cx)
    }
    fn unmark_text(&mut self, window: &mut Window, cx: &mut App) {
        self.inner.unmark_text(window, cx)
    }
    fn paste(&mut self, item: gpui_kit::ClipboardItem, window: &mut Window, cx: &mut App) {
        self.inner.paste(item, window, cx)
    }
    fn bounds_for_range(
        &mut self,
        range: Range<usize>,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<gpui_kit::Bounds<gpui_kit::Pixels>> {
        self.inner.bounds_for_range(range, window, cx)
    }
    fn character_index_for_point(
        &mut self,
        point: gpui_kit::Point<gpui_kit::Pixels>,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<usize> {
        self.inner.character_index_for_point(point, window, cx)
    }
    fn set_selected_text_range(&mut self, range: Range<usize>, window: &mut Window, cx: &mut App) {
        self.inner.set_selected_text_range(range, window, cx)
    }
    fn element_bounds(
        &mut self,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<gpui_kit::Bounds<gpui_kit::Pixels>> {
        self.inner.element_bounds(window, cx)
    }
    fn text_length_utf16(&mut self, window: &mut Window, cx: &mut App) -> Option<usize> {
        self.inner.text_length_utf16(window, cx)
    }
    fn accepts_text_input(&mut self, window: &mut Window, cx: &mut App) -> bool {
        self.inner.accepts_text_input(window, cx)
    }
    fn text_input_configuration(
        &mut self,
        window: &mut Window,
        cx: &mut App,
    ) -> gpui_kit::TextInputConfiguration {
        self.inner.text_input_configuration(window, cx)
    }
    fn text_input_editable_range(
        &mut self,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<Range<usize>> {
        self.inner.text_input_editable_range(window, cx)
    }
}
