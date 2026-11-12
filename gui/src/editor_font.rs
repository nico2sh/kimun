//! The user's Edit surface font (`[gui] editor_font_family`,
//! `editor_font_size`), held as a GPUI global. Installed at startup and
//! re-applied when Preferences are saved; the Edit surface reads it through
//! [`face_inputs`] and [`font_size`] on every frame, so a change takes effect
//! on the next paint.
//!
//! Unset values fall back to the kit theme's monospace family and size: an
//! empty setting paints exactly as before the setting existed.

use gpui_kit::component::ActiveTheme;
use gpui_kit::{App, Global, Pixels, SharedString, px};
use kimun_notes::settings::gui::GuiSettings;

/// The user's editor font, as configured. `None` fields mean "the theme's".
#[derive(Clone, Default, PartialEq, Debug)]
pub struct EditorFontSetting {
    pub family: Option<SharedString>,
    pub size: Option<Pixels>,
}

impl Global for EditorFontSetting {}

impl EditorFontSetting {
    /// The setting `gui` holds.
    pub fn from_settings(gui: &GuiSettings) -> Self {
        Self {
            family: gui.editor_font_family().map(SharedString::from),
            size: gui.editor_font_size().map(|size| px(f32::from(size))),
        }
    }
}

/// Set the global from `gui`; a setting equal to the current one is not set
/// again (no needless global-change notification).
pub fn apply(gui: &GuiSettings, cx: &mut App) {
    let next = EditorFontSetting::from_settings(gui);
    if cx.try_global::<EditorFontSetting>() != Some(&next) {
        cx.set_global(next);
    }
}

/// What the Edit surface's face is chosen from: the theme's monospace
/// family (the starting point of the face choice,
/// [`crate::fonts::EditorFonts`]) and the user's preferred family. Also the
/// key of the view's face cache: the face is chosen again when it changes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FaceInputs {
    pub kit_family: SharedString,
    pub preferred: Option<SharedString>,
}

/// The Edit surface's [`FaceInputs`]. With [`font_size`], the only reader of
/// the theme's monospace values for the Edit surface.
pub fn face_inputs(cx: &App) -> FaceInputs {
    FaceInputs {
        kit_family: cx.theme().mono_font_family.clone(),
        preferred: cx
            .try_global::<EditorFontSetting>()
            .and_then(|s| s.family.clone()),
    }
}

/// The size the Edit surface paints at: the user's, else the theme's.
pub fn font_size(cx: &App) -> Pixels {
    cx.try_global::<EditorFontSetting>()
        .and_then(|s| s.size)
        .unwrap_or(cx.theme().mono_font_size)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::TestAppContext;

    /// The `[gui]` table holding `body`.
    fn gui(body: &str) -> GuiSettings {
        toml::from_str::<kimun_notes::settings::AppSettings>(&format!("[gui]\n{body}"))
            .unwrap()
            .gui
    }

    #[gpui_kit::test]
    fn without_a_global_the_theme_supplies_everything(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            let t = cx.theme();
            let expected = FaceInputs {
                kit_family: t.mono_font_family.clone(),
                preferred: None,
            };
            let size = t.mono_font_size;
            assert_eq!(face_inputs(cx), expected);
            assert_eq!(font_size(cx), size);
        });
    }

    #[gpui_kit::test]
    fn the_configured_family_and_size_are_handed_on(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            apply(
                &gui("editor_font_family = \"Fira Code\"\neditor_font_size = 18\n"),
                cx,
            );
            let inputs = face_inputs(cx);
            assert_eq!(inputs.kit_family, cx.theme().mono_font_family);
            assert_eq!(inputs.preferred, Some(SharedString::from("Fira Code")));
            assert_eq!(font_size(cx), px(18.));
        });
    }

    #[gpui_kit::test]
    fn an_unset_setting_keeps_the_theme_values(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            apply(&gui(""), cx);
            assert_eq!(
                cx.global::<EditorFontSetting>(),
                &EditorFontSetting::default()
            );
            let t = cx.theme();
            let expected = FaceInputs {
                kit_family: t.mono_font_family.clone(),
                preferred: None,
            };
            let size = t.mono_font_size;
            assert_eq!(face_inputs(cx), expected);
            assert_eq!(font_size(cx), size);
        });
    }

    #[gpui_kit::test]
    fn applying_an_unchanged_setting_does_not_reset_the_global(cx: &mut TestAppContext) {
        // Global observers run once the update that changed the global ends,
        // so each `apply` is its own update.
        let changes = std::rc::Rc::new(std::cell::Cell::new(0));
        let seen = changes.clone();
        let settings = gui("editor_font_size = 20\n");
        let _sub = cx.update(|cx| {
            gpui_kit::init(cx);
            apply(&settings, cx);
            cx.observe_global::<EditorFontSetting>(move |_| seen.set(seen.get() + 1))
        });
        cx.update(|cx| apply(&settings, cx));
        assert_eq!(changes.get(), 0, "an equal setting is not set again");
        cx.update(|cx| apply(&gui("editor_font_size = 21\n"), cx));
        assert_eq!(changes.get(), 1);
    }
}
