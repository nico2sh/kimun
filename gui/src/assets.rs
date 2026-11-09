//! The app's asset source: the kit's default component icons plus the few
//! Lucide icons the rail and the FILES rows need that the default bundle
//! leaves out. One icon
//! family (Lucide) throughout; only the listed SVGs are embedded.

use std::borrow::Cow;
use std::sync::{Arc, LazyLock};

use gpui_kit::{AssetSource, Image, ImageFormat, Result, SharedString};

/// The app logo, a colour PNG embedded in the binary (no theme tinting).
/// Built once; `img()` decodes it through GPUI's asset cache on the
/// background executor, never on the UI thread.
static LOGO: LazyLock<Arc<Image>> = LazyLock::new(|| {
    Arc::new(Image::from_bytes(
        ImageFormat::Png,
        include_bytes!("../../assets/logo.png").to_vec(),
    ))
});

/// The app logo, for `img()`.
pub(crate) fn logo() -> Arc<Image> {
    LOGO.clone()
}

/// The window icon (`WindowOptions::icon`, which only X11 reads): the logo
/// at 128 px, decoded from embedded bytes (no file is read). Wayland, macOS
/// and Windows take the icon from the desktop entry, the bundle or the
/// executable's resources instead. `None` (logged) if it would not decode.
#[cfg(target_os = "linux")]
pub(crate) fn window_icon() -> Option<Arc<image::RgbaImage>> {
    match image::load_from_memory_with_format(
        include_bytes!("../../assets/logo-128.png"),
        image::ImageFormat::Png,
    ) {
        Ok(icon) => Some(Arc::new(icon.into_rgba8())),
        Err(e) => {
            tracing::warn!("the window icon would not decode: {e}");
            None
        }
    }
}

gpui_kit::assets::icon_assets!(
    RailIcons,
    [
        Tag,
        Link,
        ListTree,
        Sparkles,
        MessageCircleQuestionMark,
        CornerLeftUp,
        Folder,
        CalendarDays,
        FileText,
        Paperclip,
        Plus,
        Circle,
        CircleQuestionMark
    ]
);

/// The kit's default icons with the rail's extras.
pub struct AppAssets;

impl AssetSource for AppAssets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        match RailIcons.load(path)? {
            Some(bytes) => Ok(Some(bytes)),
            None => gpui_kit::assets::Assets.load(path),
        }
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut paths = gpui_kit::assets::Assets.list(path)?;
        paths.extend(RailIcons.list(path)?);
        paths.sort();
        paths.dedup();
        Ok(paths)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::assets::IconName;

    use crate::workspace::rail_icon;

    /// Every rail icon resolves, whether the default bundle or the extras
    /// carry it: a missing SVG would paint an empty cell.
    #[test]
    fn every_drawer_view_icon_is_bundled() {
        for view in kimun_notes::logic::nav::DrawerView::ALL {
            let icon: IconName = rail_icon(view);
            let path = icon.path();
            assert!(
                AppAssets.load(&path).unwrap().is_some(),
                "{view:?}: {path} is not bundled"
            );
        }
    }

    /// The title bar's window controls use the default bundle's icons.
    #[test]
    fn every_window_control_icon_is_bundled() {
        for icon in crate::workspace::WINDOW_CONTROL_ICONS {
            let path = icon.path();
            assert!(
                AppAssets.load(&path).unwrap().is_some(),
                "{path} is not bundled"
            );
        }
    }

    /// The X11 window icon decodes, at the size it is embedded at.
    #[cfg(target_os = "linux")]
    #[test]
    fn the_window_icon_decodes() {
        let icon = window_icon().expect("the embedded icon decodes");
        assert_eq!((icon.width(), icon.height()), (128, 128));
    }

    /// Every FILES row icon resolves too.
    #[test]
    fn every_files_row_icon_is_bundled() {
        for icon in crate::workspace::FILES_ICONS {
            let path = icon.path();
            assert!(
                AppAssets.load(&path).unwrap().is_some(),
                "{path} is not bundled"
            );
        }
    }
}
