//! The Edit surface's monospace face, chosen per machine so **bold** reads as
//! bold.
//!
//! GPUI resolves a weight or a slant to a face of the family. A family
//! installed as one variable font (Fedora's `NotoSansMono[wght].ttf`, the
//! kit's Linux fallback) has a single face, so its bold and italic resolve to
//! the regular face and paint upright and regular. The rule:
//!
//! 0. the user's preferred family, when it is installed (the name matches
//!    whatever its capital letters): kept whatever its faces, with
//!    synthetic bold when it has no real bold face;
//! 1. the kit theme's monospace family, when it has a real bold face;
//! 2. else the first installed well-known monospace family with real bold
//!    and italic faces, then one with a real bold face;
//! 3. else the kit's family with **synthetic bold**: the painter draws a bold
//!    run twice, the second copy a hairline to the right.
//!
//! GPUI has no glyph skew, so there is no synthetic italic: on a family with
//! no italic face italic text keeps only its colour (the TUI role colours
//! differ for every italic role).
//!
//! Every face keeps one cell = one fixed advance: the painter forces the cell
//! width on one-cell clusters whichever face draws them.

use gpui_kit::{Font, FontStyle, FontWeight, SharedString, TextSystem, font};
use kimun_notes::logic::preferences::same_font_family;

/// Well-known monospace families with separate bold (and mostly italic)
/// faces, tried in order after the kit's own family. Only installed ones
/// are probed.
pub const MONO_CANDIDATES: &[&str] = &[
    "DejaVu Sans Mono",
    "Adwaita Mono",
    "Cascadia Mono",
    "JetBrains Mono",
    "Ubuntu Mono",
    "Liberation Mono",
    "Noto Sans Mono",
    "Menlo",
    "SF Mono",
    "Monaco",
    "Consolas",
    "Courier New",
];

/// Which styled faces a family resolves to a face of their own.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FaceSupport {
    pub bold: bool,
    pub italic: bool,
}

/// The editor's monospace family and how it draws bold.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditorFonts {
    pub family: SharedString,
    /// The family has no bold face: bold is painted twice, offset.
    pub faux_bold: bool,
}

impl EditorFonts {
    /// The face a run paints in. With synthetic bold the weight stays
    /// regular: the painter adds the weight.
    pub fn face(&self, bold: bool, italic: bool) -> Font {
        let mut f = font(self.family.clone());
        if bold && !self.faux_bold {
            f.weight = FontWeight::BOLD;
        }
        if italic {
            f.style = FontStyle::Italic;
        }
        f
    }

    /// Whether a run painted `bold` needs the synthetic second copy.
    pub fn synthesizes(&self, bold: bool) -> bool {
        bold && self.faux_bold
    }

    /// Probe the machine's fonts (once per view; listing fonts is slow on
    /// macOS) and choose, starting from the kit theme's family.
    pub fn resolve(
        text_system: &TextSystem,
        kit_family: &SharedString,
        preferred: Option<&SharedString>,
    ) -> Self {
        let installed = text_system.all_font_names();
        choose(
            kit_family,
            preferred.map(|p| p.as_ref()),
            &installed,
            |family| {
                let regular = font(SharedString::from(family.to_string()));
                let id = text_system.resolve_font(&regular);
                FaceSupport {
                    bold: text_system.resolve_font(&regular.clone().bold()) != id,
                    italic: text_system.resolve_font(&regular.italic()) != id,
                }
            },
        )
    }
}

/// The choice rule (module docs), pure. `probe` is asked only about the kit
/// family and installed candidates.
pub fn choose(
    kit_family: &SharedString,
    preferred: Option<&str>,
    installed: &[String],
    probe: impl Fn(&str) -> FaceSupport,
) -> EditorFonts {
    // Family names match whatever their capital letters (the one rule,
    // shared with Preferences); the installed spelling is the one painted with.
    if let Some(p) = preferred
        && let Some(name) = installed
            .iter()
            .find(|n| same_font_family(Some(n), Some(p)))
    {
        return EditorFonts {
            family: SharedString::from(name.clone()),
            faux_bold: !probe(name).bold,
        };
    }
    let kit = probe(kit_family);
    if kit.bold {
        return EditorFonts {
            family: kit_family.clone(),
            faux_bold: false,
        };
    }
    let probed: Vec<(&str, FaceSupport)> = MONO_CANDIDATES
        .iter()
        .copied()
        .filter(|c| *c != kit_family.as_ref() && installed.iter().any(|n| n == c))
        .map(|c| (c, probe(c)))
        .collect();
    let pick = probed
        .iter()
        .find(|(_, s)| s.bold && s.italic)
        .or_else(|| probed.iter().find(|(_, s)| s.bold));
    match pick {
        Some((family, _)) => EditorFonts {
            family: SharedString::from(family.to_string()),
            faux_bold: false,
        },
        None => EditorFonts {
            family: kit_family.clone(),
            faux_bold: true,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn installed(names: &[&str]) -> Vec<String> {
        names.iter().map(|n| n.to_string()).collect()
    }

    fn table(faces: &'static [(&'static str, bool, bool)]) -> impl Fn(&str) -> FaceSupport {
        move |family| {
            faces
                .iter()
                .find(|(n, _, _)| *n == family)
                .map(|&(_, bold, italic)| FaceSupport { bold, italic })
                .unwrap_or_default()
        }
    }

    #[test]
    fn a_kit_family_with_a_bold_face_is_kept() {
        let f = choose(
            &"Menlo".into(),
            None,
            &installed(&["Menlo", "Liberation Mono"]),
            table(&[("Menlo", true, false), ("Liberation Mono", true, true)]),
        );
        assert_eq!(f.family.as_ref(), "Menlo");
        assert!(!f.faux_bold);
    }

    #[test]
    fn a_variable_kit_family_gives_way_to_a_full_installed_family() {
        // Noto Sans Mono as one variable font: no bold, no italic face.
        let f = choose(
            &"Noto Sans Mono".into(),
            None,
            &installed(&["Noto Sans Mono", "Ubuntu Mono", "Liberation Mono"]),
            table(&[
                ("Noto Sans Mono", false, false),
                ("Ubuntu Mono", true, false),
                ("Liberation Mono", true, true),
            ]),
        );
        // Bold and italic beats bold only, even later in the list.
        assert_eq!(f.family.as_ref(), "Liberation Mono");
        assert!(!f.faux_bold);
    }

    #[test]
    fn a_bold_only_family_is_taken_when_none_has_both() {
        let f = choose(
            &"X".into(),
            None,
            &installed(&["Ubuntu Mono"]),
            table(&[("Ubuntu Mono", true, false)]),
        );
        assert_eq!(f.family.as_ref(), "Ubuntu Mono");
    }

    #[test]
    fn uninstalled_candidates_are_never_probed_and_nothing_falls_back_to_faux_bold() {
        let probe = |family: &str| {
            assert!(family == "X" || family == "Liberation Mono", "{family}");
            FaceSupport::default()
        };
        let f = choose(
            &"X".into(),
            None,
            &installed(&["Liberation Mono", "Other"]),
            probe,
        );
        assert_eq!(f.family.as_ref(), "X");
        assert!(f.faux_bold);
    }

    #[test]
    fn a_preferred_family_with_a_bold_face_is_used() {
        let f = choose(
            &"Menlo".into(),
            Some("A"),
            &installed(&["A", "Menlo"]),
            table(&[("A", true, false), ("Menlo", true, true)]),
        );
        assert_eq!(f.family.as_ref(), "A");
        assert!(!f.faux_bold);
    }

    #[test]
    fn a_preferred_family_without_bold_keeps_the_family_and_fakes_bold() {
        let f = choose(
            &"X".into(),
            Some("A"),
            &installed(&["A", "Liberation Mono"]),
            table(&[("A", false, false), ("Liberation Mono", true, true)]),
        );
        assert_eq!(f.family.as_ref(), "A");
        assert!(f.faux_bold);
    }

    #[test]
    fn a_preferred_family_matches_ignoring_case_and_takes_the_installed_spelling() {
        let probe = |family: &str| {
            assert_eq!(family, "Fira Code", "probed by the installed spelling");
            FaceSupport {
                bold: true,
                italic: false,
            }
        };
        let f = choose(
            &"X".into(),
            Some("fira CODE"),
            &installed(&["Fira Code", "Liberation Mono"]),
            probe,
        );
        assert_eq!(f.family.as_ref(), "Fira Code");
        assert!(!f.faux_bold);
    }

    #[test]
    fn a_preferred_family_that_is_not_installed_falls_back_to_the_rule() {
        let faces: &'static [(&str, bool, bool)] =
            &[("X", false, false), ("Liberation Mono", true, true)];
        let inst = installed(&["Liberation Mono"]);
        let base = choose(&"X".into(), None, &inst, table(faces));
        let probe = |family: &str| {
            assert_ne!(family, "Gone");
            table(faces)(family)
        };
        let f = choose(&"X".into(), Some("Gone"), &inst, probe);
        assert_eq!(f, base);
    }

    #[test]
    fn faces_carry_weight_and_slant_unless_bold_is_synthetic() {
        let real = EditorFonts {
            family: "M".into(),
            faux_bold: false,
        };
        assert_eq!(real.face(true, true).weight, FontWeight::BOLD);
        assert_eq!(real.face(true, true).style, FontStyle::Italic);
        assert!(!real.synthesizes(true));
        let faux = EditorFonts {
            family: "M".into(),
            faux_bold: true,
        };
        assert_eq!(faux.face(true, false).weight, FontWeight::NORMAL);
        assert!(faux.synthesizes(true));
        assert!(!faux.synthesizes(false));
    }
}
