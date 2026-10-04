//! Today layout and the minimal desktop role adaptation of the existing Kit theme.
pub(crate) const MIN_WIDTH: f32 = 760.;
pub(crate) const MIN_HEIGHT: f32 = 560.;
// Fixed list tail reserves the bounded six-line composer plus feedback.
pub(crate) const COMPOSER_CLEARANCE: f32 = 240.;

/// Approved desktop surfaces with fn-ios universal text/caret colors at
/// 300261c7467827208dfd71f7af0eb6615d9935cb.
/// Keep one mapping here; presentation consumes Kit's semantic colors.
pub(crate) fn apply_theme(mode: impl Into<gpui_kit::component::ThemeMode>, cx: &mut gpui_kit::App) {
    use gpui_kit::{component::Theme, rgb};
    Theme::change(mode, None, cx);
    Theme::update(cx, |theme| {
        let pair = |light, dark| rgb(if theme.is_dark() { dark } else { light }).into();
        let background = pair(0xffffff, 0x0d0d0d);
        let rail = pair(0xfafafa, 0x141414);
        let surface = pair(0xffffff, 0x1c1c1c);
        let hover = pair(0xf5f5f5, 0x202020);
        let selection = pair(0xebebeb, 0x2b2b2b);
        let border = pair(0xe8e8e8, 0x303030);
        let foreground = pair(0x171717, 0xededed);
        let secondary = pair(0x525252, 0xa6a6a6);
        let tertiary = pair(0x737373, 0x808080);
        let primary = pair(0x2e2e2e, 0xc6c6c6);
        let on_primary = pair(0xe2e2e2, 0x222222);
        let cursor = rgb(0x05c7f7).into();
        theme.background = background;
        theme.foreground = foreground;
        theme.popover = surface;
        theme.popover_foreground = foreground;
        theme.secondary = rail;
        theme.secondary_foreground = secondary;
        theme.muted = hover;
        theme.muted_foreground = tertiary;
        theme.accent = hover;
        theme.accent_foreground = foreground;
        theme.selection = selection;
        theme.border = border;
        theme.input = border;
        theme.primary = primary;
        theme.primary_foreground = on_primary;
        theme.caret = cursor;
        theme.ring = cursor;
        theme.button = surface;
        theme.button_foreground = foreground;
        theme.button_hover = hover;
        theme.button_active = selection;
    });
}
