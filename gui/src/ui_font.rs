//! The UI's text face: Inter 4.1 (SIL OFL, `assets/fonts/inter/`), embedded
//! in the binary and registered with the text system before the first
//! window renders. The kit theme's `font_family` names it
//! ([`crate::theme::apply_to_kit`]), and the kit's window root sets that
//! family on the whole window, so every view and kit component inherits it.
//! The Edit surface and code-like runs keep the kit's monospace family.
//!
//! **Static faces, not the variable font.** GPUI resolves a weight or a slant
//! to a *face* of the family (on Linux `find_best_match` scores the
//! family's fontdb faces by weight and style, and loads the chosen face at
//! its default instance, `Weight::NORMAL`: no `wght` variation is applied).
//! `InterVariable.ttf` is one face, so bold and semibold would paint
//! regular. Each weight the UI asks for is therefore its own static face:
//! Regular, Medium, SemiBold, Bold, Italic and BoldItalic (the Ask answer's
//! bold-italic runs), all under the one family name "Inter".
//!
//! **Fallback.** A face that is not a font file, a backend that refuses the
//! bytes, or a family that does not show up once added: a warning is logged
//! once and the kit keeps its previous family (the system UI font); the app
//! starts either way.

use std::borrow::Cow;
use std::sync::OnceLock;

use gpui_kit::{App, FontFeatures, SharedString, TextSystem};

/// The family every embedded face declares.
pub const FAMILY: &str = "Inter";

/// The embedded faces, one per weight/slant the UI uses.
pub const FACES: &[&[u8]] = &[
    include_bytes!("../../assets/fonts/inter/Inter-Regular.ttf"),
    include_bytes!("../../assets/fonts/inter/Inter-Medium.ttf"),
    include_bytes!("../../assets/fonts/inter/Inter-SemiBold.ttf"),
    include_bytes!("../../assets/fonts/inter/Inter-Bold.ttf"),
    include_bytes!("../../assets/fonts/inter/Inter-Italic.ttf"),
    include_bytes!("../../assets/fonts/inter/Inter-BoldItalic.ttf"),
];

/// Why the faces were not registered.
#[derive(Debug)]
pub enum RegisterError {
    /// A face does not start with a TrueType/OpenType signature.
    NotAFont { index: usize },
    /// The platform text system refused the bytes.
    Rejected(String),
    /// The bytes were taken, but the family is not among the installed ones
    /// (cosmic-text skips a file it cannot parse without an error).
    Missing,
}

impl std::fmt::Display for RegisterError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotAFont { index } => write!(f, "face {index} is not a font file"),
            Self::Rejected(e) => write!(f, "the text system refused the faces: {e}"),
            Self::Missing => write!(f, "the family {FAMILY:?} is not installed after adding it"),
        }
    }
}

/// Whether `bytes` starts like a font file: TrueType (`00 01 00 00`, `true`)
/// or CFF OpenType (`OTTO`).
pub fn has_font_signature(bytes: &[u8]) -> bool {
    matches!(
        bytes.get(..4),
        Some([0, 1, 0, 0]) | Some(b"OTTO") | Some(b"true")
    )
}

/// Add `faces` to `text_system` and check that [`FAMILY`] is installed
/// afterwards. Nothing is added when any face lacks a font signature.
pub fn register(text_system: &TextSystem, faces: &[&'static [u8]]) -> Result<(), RegisterError> {
    if let Some(index) = faces.iter().position(|f| !has_font_signature(f)) {
        return Err(RegisterError::NotAFont { index });
    }
    text_system
        .add_fonts(faces.iter().map(|f| Cow::Borrowed(*f)).collect())
        .map_err(|e| RegisterError::Rejected(format!("{e:#}")))?;
    if text_system.all_font_names().iter().any(|n| n == FAMILY) {
        Ok(())
    } else {
        Err(RegisterError::Missing)
    }
}

/// The registered UI family, once [`install`] has run and succeeded.
static UI_FAMILY: OnceLock<Option<SharedString>> = OnceLock::new();

/// Register the embedded faces with the app's text system, once per
/// process. On failure, log a warning and leave the kit's family alone.
pub fn install(cx: &App) {
    UI_FAMILY.get_or_init(|| match register(cx.text_system(), FACES) {
        Ok(()) => Some(SharedString::new_static(FAMILY)),
        Err(e) => {
            tracing::warn!(
                "the UI font {FAMILY} was not registered ({e}); keeping the system font"
            );
            None
        }
    });
}

/// The UI family to name on the kit theme: `None` before [`install`] or when
/// it failed (the kit's own family stays).
pub fn family() -> Option<SharedString> {
    UI_FAMILY.get().cloned().flatten()
}

/// Tabular figures (OpenType `tnum`): every digit one advance, for numbers
/// that change in place or line up in a column (the status row's counters,
/// the Preferences steppers, durations). Inter's default figures are
/// proportional.
pub fn tabular_figures() -> FontFeatures {
    FontFeatures(std::sync::Arc::new(vec![("tnum".into(), 1)]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_embedded_face_is_a_truetype_file() {
        assert_eq!(FACES.len(), 6);
        for (i, face) in FACES.iter().enumerate() {
            assert!(face.len() > 100_000, "face {i} is embedded whole");
            assert_eq!(&face[..4], &[0, 1, 0, 0], "face {i}: TrueType magic");
        }
    }

    #[test]
    fn font_signatures() {
        assert!(has_font_signature(b"OTTO...."));
        assert!(has_font_signature(b"true...."));
        assert!(has_font_signature(&[0, 1, 0, 0, 9]));
        assert!(!has_font_signature(b"wOF2...."));
        assert!(!has_font_signature(&[0, 1]));
    }
}

/// Tests against a real text system: Linux's backend (cosmic-text), the one
/// a test can build without a display.
#[cfg(all(test, target_os = "linux"))]
pub(crate) mod cosmic_tests {
    use super::*;
    use gpui_kit::{Font, FontWeight, font};
    use std::sync::Arc;

    /// A text system with no system fonts, only what is added to it: what
    /// resolves is what was registered.
    pub(crate) fn bare_text_system() -> (TextSystem, Arc<gpui_wgpu::CosmicTextSystem>) {
        let platform = Arc::new(gpui_wgpu::CosmicTextSystem::new_without_system_fonts(
            FAMILY,
        ));
        (TextSystem::new(platform.clone()), platform)
    }

    #[test]
    fn the_family_registers_with_a_distinct_face_per_weight_and_slant() {
        let (ts, _) = bare_text_system();
        register(&ts, FACES).expect("Inter registers");
        assert!(ts.all_font_names().iter().any(|n| n == FAMILY));
        let regular = font(FAMILY);
        let faces = [
            regular.clone(),
            Font {
                weight: FontWeight::MEDIUM,
                ..regular.clone()
            },
            Font {
                weight: FontWeight::SEMIBOLD,
                ..regular.clone()
            },
            regular.clone().bold(),
            regular.clone().italic(),
            regular.clone().bold().italic(),
        ];
        let ids: Vec<_> = faces.iter().map(|f| ts.resolve_font(f)).collect();
        for (i, a) in ids.iter().enumerate() {
            for b in &ids[i + 1..] {
                assert_ne!(a, b, "every weight/slant is its own face: {ids:?}");
            }
        }
    }

    /// The Welcome wordmark's `FontWeight::BOLD` paints the Bold face: wider
    /// than Regular, and the face Inter-Bold declares (weight 700).
    #[test]
    fn a_bold_run_resolves_to_the_real_bold_face() {
        let (ts, _) = bare_text_system();
        register(&ts, FACES).unwrap();
        let size = gpui_kit::px(100.);
        let regular = ts.resolve_font(&font(FAMILY));
        let bold = ts.resolve_font(&font(FAMILY).bold());
        assert_ne!(regular, bold);
        let w = |id| ts.advance(id, size, 'm').unwrap().width;
        assert!(w(bold) > w(regular), "{:?} vs {:?}", w(bold), w(regular));
        // Only Bold is registered: the bold run still finds it (not Regular).
        let (only_bold, _) = bare_text_system();
        register(&only_bold, &[FACES[3]]).unwrap();
        let id = only_bold.resolve_font(&font(FAMILY).bold());
        assert_eq!(only_bold.advance(id, size, 'm').unwrap().width, w(bold));
    }

    /// Bad bytes register nothing and say why; the caller keeps its family.
    #[test]
    fn bad_bytes_fall_back_without_registering() {
        let (ts, _) = bare_text_system();
        let garbage: &'static [u8] = b"definitely not a font file";
        assert!(matches!(
            register(&ts, &[FACES[0], garbage]),
            Err(RegisterError::NotAFont { index: 1 })
        ));
        assert!(ts.all_font_names().is_empty(), "nothing was added");
        // Right signature, corrupt tables: cosmic-text skips it silently,
        // the family check catches it.
        let corrupt: &'static [u8] = &[0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        assert!(matches!(
            register(&ts, &[corrupt]),
            Err(RegisterError::Missing)
        ));
    }

    /// `tnum` gives every digit the same advance in Inter, whose default
    /// figures are proportional (so counters would jitter as they change).
    #[test]
    fn tabular_figures_give_every_digit_one_advance() {
        use gpui_kit::PlatformTextSystem as _;
        let (ts, platform) = bare_text_system();
        register(&ts, FACES).unwrap();
        let size = gpui_kit::px(100.);
        let width = |text: &str, features: FontFeatures| {
            let font_id = ts.resolve_font(&Font {
                features,
                ..font(FAMILY)
            });
            let run = gpui_kit::FontRun {
                len: text.len(),
                font_id,
            };
            platform.layout_line(text, size, &[run]).width
        };
        assert_ne!(
            width("1111", FontFeatures::default()),
            width("0000", FontFeatures::default()),
            "Inter's default figures are proportional"
        );
        assert_eq!(
            width("1111", tabular_figures()),
            width("0000", tabular_figures())
        );
    }
}
