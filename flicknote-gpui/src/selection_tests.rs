//! Owned rendered selection/role evidence; native pixels remain separate.
use gpui_kit::component::{
    Theme, ThemeMode,
    input::{Textarea, TextareaState},
};
use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    Bounds, Context, ElementInputHandler, Entity, Focusable, InputHandler, IntoElement, Render,
    TestAppContext, Window, WindowBounds, WindowOptions, div, point, prelude::*, px, size,
};

struct InputFixture {
    input: Entity<TextareaState>,
    adapter: bool,
}
impl Render for InputFixture {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .bg(Theme::global(cx).color_tokens().surface)
            .child(
                Textarea::new(&self.input)
                    .appearance(false)
                    .bordered(false)
                    .text_size(px(17.)),
            )
            .when(self.adapter, |root| {
                root.child(crate::native_input::install(self.input.clone()))
            })
    }
}

#[gpui_kit::test]
fn text_selection_preserves_kit_role_and_readable_composed_contrast(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    for mode in [ThemeMode::Light, ThemeMode::Dark] {
        let stock = cx.update(|cx| {
            Theme::change(mode, None, cx);
            let stock = Theme::global(cx).selection;
            crate::workspace::apply_theme(mode, cx);
            assert_eq!(
                Theme::global(cx).selection,
                stock,
                "retain Kit's dedicated input selection, not row fill"
            );
            stock
        });
        for adapter in [false, true] {
            let (window, view) = cx.update(|cx| {
                gpui_kit::open_window(
                    WindowOptions {
                        window_bounds: Some(WindowBounds::Windowed(Bounds {
                            origin: point(px(0.), px(0.)),
                            size: size(px(480.), px(200.)),
                        })),
                        ..Default::default()
                    },
                    cx,
                    |window, cx| {
                        cx.new(|cx| {
                            let input = cx.new(|cx| TextareaState::new(window, cx).rows(3));
                            input.update(cx, |input, cx| {
                                input.set_value("Owned selection fixture", window, cx);
                                input.focus(window, cx);
                                input.set_selected_range(0..5, cx);
                            });
                            InputFixture { input, adapter }
                        })
                    },
                )
                .unwrap()
            });
            let window = window.downcast::<gpui_kit::base::Root>().unwrap();
            cx.update_window(window.into(), |_, window, _| window.activate_window())
                .unwrap();
            cx.run_until_parked();
            cx.update_window(window.into(), |_, window, cx| {
                window.render_frame(cx);
                assert!(window.is_window_active());
                let input = view.read(cx).input.clone();
                assert_eq!(input.read(cx).selected_range(), 0..5);
                assert!(input.read(cx).focus_handle(cx).is_focused(window));
                let mut native =
                    crate::native_input::ComposerInput::new(input.clone(), cx).unwrap();
                let mut kit =
                    ElementInputHandler::new(input.read(cx).text_bounds().unwrap(), input.clone());
                assert_eq!(
                    native.selected_text_range(false, window, cx).unwrap().range,
                    kit.selected_text_range(false, window, cx).unwrap().range
                );
                assert_eq!(
                    native.bounds_for_range(0..5, window, cx),
                    kit.bounds_for_range(0..5, window, cx)
                );
                assert!(native.marked_text_range(window, cx).is_none());
                let bounds = native.bounds_for_range(0..5, window, cx).unwrap();
                assert!(bounds.size.width > px(20.) && bounds.size.height > px(10.));
                assert_selection_contrast(stock, Theme::global(cx));
                window.remove_window();
            })
            .unwrap();
        }
    }
}
fn assert_selection_contrast(stock: gpui_kit::Hsla, theme: &Theme) {
    // Pinned Kit InputElement paints with this dedicated role. The
    // stock test platform has no renderer, so these are composed
    // color/geometry contracts, not native screenshot assertions.
    assert_eq!(theme.selection, stock);
    assert_eq!(stock.a, 0.3);
    let background = theme.color_tokens().surface.to_rgb();
    let selection = stock.to_rgb();
    let composed = [selection.r, selection.g, selection.b]
        .into_iter()
        .zip([background.r, background.g, background.b])
        .map(|(fill, bg)| fill * selection.a + bg * (1. - selection.a))
        .collect::<Vec<_>>();
    let distinction = composed
        .iter()
        .zip([background.r, background.g, background.b])
        .map(|(fill, bg)| (fill - bg).abs())
        .fold(0_f32, f32::max);
    assert!(
        distinction > 0.1,
        "selected area must be visibly distinct from the surface"
    );
    let foreground = theme.foreground.to_rgb();
    let text = luminance([foreground.r, foreground.g, foreground.b]);
    let fill = luminance([composed[0], composed[1], composed[2]]);
    assert!(
        (text.max(fill) + 0.05) / (text.min(fill) + 0.05) >= 4.5,
        "selected text must remain readable"
    );
}
fn luminance(rgb: [f32; 3]) -> f32 {
    let linear = rgb.map(|v| {
        if v <= 0.04045 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    });
    linear[0] * 0.2126 + linear[1] * 0.7152 + linear[2] * 0.0722
}
