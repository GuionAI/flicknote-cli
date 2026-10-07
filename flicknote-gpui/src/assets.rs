//! Package the screen's Lucide icons alongside Kit's default component icons.
use gpui_kit::{AssetSource, Result, SharedString};
use std::borrow::Cow;

gpui_kit::assets::icon_assets!(
    ScreenIcons,
    [
        House,
        ChartBar,
        Archive,
        ArchiveRestore,
        Unlink,
        Zap,
        MessagesSquare,
        Link,
        FileImage,
        X,
    ]
);

pub(crate) struct TodayAssets;
impl AssetSource for TodayAssets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        if let Some(bytes) = ScreenIcons.load(path)? {
            return Ok(Some(bytes));
        }
        gpui_kit::assets::Assets.load(path)
    }
    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut paths = gpui_kit::assets::Assets.list(path)?;
        paths.extend(ScreenIcons.list(path)?);
        Ok(paths)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::{SvgRenderer, assets::IconName};
    use std::sync::Arc;

    #[test]
    fn packaged_today_icons_rasterize_visible_strokes() {
        let renderer = SvgRenderer::new(Arc::new(TodayAssets));
        for name in [
            IconName::House,
            IconName::ChartBar,
            IconName::Archive,
            IconName::ArchiveRestore,
            IconName::Unlink,
            IconName::Zap,
            IconName::MessagesSquare,
            IconName::Link,
            IconName::FileImage,
            IconName::X,
            IconName::Search,
            IconName::FileText,
            IconName::Copy,
        ] {
            let bytes = TodayAssets.load(&name.path()).unwrap().unwrap();
            let image = renderer.render_single_frame(&bytes, 1.).unwrap();
            assert!(
                image
                    .as_bytes(0)
                    .unwrap()
                    .chunks_exact(4)
                    .any(|pixel| pixel[3] != 0),
                "{name:?} must have visible strokes"
            );
        }
    }
}
