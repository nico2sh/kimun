//! The choice of the editor face, and the app-wide face cache that every
//! editor group shares. This module does not load fonts: it asks
//! [`EditorFonts::resolve`] once per set of inputs.

use gpui_kit::{App, TextSystem};

use super::EditorView;
use crate::editor_font::FaceInputs;
use crate::fonts::EditorFonts;

impl EditorView {
    /// The editor's face for `inputs` (see [`EditorFonts`]). The view keeps
    /// its own copy for the frame-to-frame case. Behind it, one cache for the
    /// whole app: a new editor group (a split, a restored session) reuses the
    /// face another group chose instead of listing the machine's fonts on the
    /// UI thread again.
    pub(crate) fn editor_fonts(
        &mut self,
        inputs: &FaceInputs,
        text_system: &TextSystem,
        cx: &mut App,
    ) -> EditorFonts {
        match &self.fonts {
            Some((key, fonts)) if key == inputs => fonts.clone(),
            _ => {
                let fonts = shared_faces(inputs, text_system, cx);
                self.fonts = Some((inputs.clone(), fonts.clone()));
                fonts
            }
        }
    }
}

/// How many faces the app-wide cache keeps. A live font preview in
/// Preferences makes a new key per family tried: the oldest go first.
const SHARED_FACES: usize = 8;

/// The faces chosen so far, by inputs. One per app, shared by every editor
/// group.
#[derive(Default)]
struct FaceCache(Vec<(FaceInputs, EditorFonts)>);

impl gpui_kit::Global for FaceCache {}

/// The face for `inputs`, probing the machine's fonts only the first time
/// the app sees them (listing fonts is slow on macOS).
fn shared_faces(inputs: &FaceInputs, text_system: &TextSystem, cx: &mut App) -> EditorFonts {
    if let Some(cache) = cx.try_global::<FaceCache>()
        && let Some((_, fonts)) = cache.0.iter().find(|(key, _)| key == inputs)
    {
        return fonts.clone();
    }
    let fonts = EditorFonts::resolve(text_system, &inputs.kit_family, inputs.preferred.as_ref());
    let cache = cx.default_global::<FaceCache>();
    if cache.0.len() >= SHARED_FACES {
        cache.0.remove(0);
    }
    cache.0.push((inputs.clone(), fonts.clone()));
    fonts
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::SharedString;

    /// Degraded: the headless GPUI text system has no fonts, so every
    /// family resolves to the same fallback and the chosen family cannot
    /// tell the calls apart. What is pinned is the cache key: a new
    /// `preferred` with the same kit family must miss the cache and be
    /// stored, so a key on the kit family alone fails here.
    #[gpui_kit::test]
    fn a_new_preferred_family_is_a_new_face_cache_key(cx: &mut gpui_kit::TestAppContext) {
        use gpui_kit::AppContext as _;
        let text_system = cx.text_system().clone();
        let view = cx.new(|cx| EditorView::new(None, false, cx));
        let inputs = |preferred: Option<&str>| FaceInputs {
            kit_family: "Kit Mono".into(),
            preferred: preferred.map(SharedString::from),
        };
        let key = |view: &EditorView| view.fonts.as_ref().map(|(key, _)| key.clone());
        view.update(cx, |view, cx| {
            for preferred in [Some("Alpha Mono"), Some("Beta Mono"), None] {
                view.editor_fonts(&inputs(preferred), &text_system, cx);
                assert_eq!(key(view), Some(inputs(preferred)));
            }
        });
    }

    /// A second editor group asks for the face the first one chose: the app
    /// holds one entry for the pair, and the cache stays within its bound
    /// however many families are tried.
    #[gpui_kit::test]
    fn editor_groups_share_one_face_cache(cx: &mut gpui_kit::TestAppContext) {
        use gpui_kit::AppContext as _;
        let text_system = cx.text_system().clone();
        let first = cx.new(|cx| EditorView::new(None, false, cx));
        let second = cx.new(|cx| EditorView::new(None, false, cx));
        let inputs = |preferred: Option<String>| FaceInputs {
            kit_family: "Kit Mono".into(),
            preferred: preferred.map(SharedString::from),
        };
        let entries = |cx: &mut gpui_kit::TestAppContext| {
            cx.update(|cx| cx.try_global::<FaceCache>().map_or(0, |c| c.0.len()))
        };
        for view in [&first, &second] {
            view.update(cx, |view, cx| {
                view.editor_fonts(&inputs(None), &text_system, cx);
            });
        }
        assert_eq!(entries(cx), 1, "two groups, one entry");
        first.update(cx, |view, cx| {
            for n in 0..(SHARED_FACES * 2) {
                view.editor_fonts(&inputs(Some(format!("Mono {n}"))), &text_system, cx);
            }
        });
        assert_eq!(entries(cx), SHARED_FACES, "the cache keeps its bound");
    }
}
