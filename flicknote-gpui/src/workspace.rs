//! Today layout and the minimal desktop role adaptation of the existing Kit theme.
pub(crate) const MIN_WIDTH: f32 = 760.;
pub(crate) const MIN_HEIGHT: f32 = 560.;
pub(crate) const RAIL_WIDTH: f32 = 196.;
pub(crate) const HEADER_HEIGHT: f32 = 44.;

/// Reserve useful center and reading widths at the supported minimum viewport.
pub(crate) fn reading_width(viewport: f32) -> f32 {
    ((viewport - RAIL_WIDTH) * 0.48).clamp(272., 420.).round()
}

/// Independently authored continuous-workbench roles, projected into existing Kit.
/// Presentation consumes semantic colors; Kit owns the dedicated input selection.
pub(crate) fn apply_theme(mode: impl Into<gpui_kit::component::ThemeMode>, cx: &mut gpui_kit::App) {
    use gpui_kit::{component::Theme, rgb};
    Theme::change(mode, None, cx);
    Theme::update(cx, |theme| {
        let dark_mode = theme.is_dark();
        let pair = |light, dark| rgb(if dark_mode { dark } else { light }).into();
        let background = pair(0xfafafc, 0x252830);
        let rail = pair(0xeceef2, 0x20232a);
        let surface = pair(0xffffff, 0x282c34);
        let hover = pair(0xe4e8ef, 0x303641);
        let selection = pair(0xd6e3f5, 0x384963);
        let border = pair(0xd6dae2, 0x3b414d);
        let foreground = pair(0x242831, 0xe3e6ec);
        let secondary = pair(0x505766, 0xaab2c0);
        let tertiary = pair(0x606878, 0xa0a9b8);
        let primary = pair(0x2864b4, 0x80adfa);
        let on_primary = pair(0xffffff, 0x20232a);
        let cursor = primary;
        theme.radius = gpui_kit::px(4.);
        theme.radius_lg = gpui_kit::px(6.);
        theme.danger = pair(0xa12d35, 0xf4a0a5);
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
        // Kit selection is for input text; keep workbench rows separate.
        theme.list_active = selection;
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
