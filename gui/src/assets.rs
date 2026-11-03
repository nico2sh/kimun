//! The app's asset source: the kit's default component icons plus the few
//! Lucide icons the rail and the FILES rows need that the default bundle
//! leaves out. One icon
//! family (Lucide) throughout; only the listed SVGs are embedded.

use std::borrow::Cow;

use gpui_kit::{AssetSource, Result, SharedString};

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
        Circle
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
