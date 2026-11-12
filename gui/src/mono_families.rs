//! The installed monospace font families, for the Preferences font dropdown.
//!
//! Spike result (2026-11-12, gpui-pre 0.3.11): `TextSystem` is `Send + Sync`.
//! Its fields are an `Arc<dyn PlatformTextSystem>` (the trait is declared
//! `Send + Sync`), `parking_lot` locks and atomics, and `App::text_system`
//! hands out an `Arc<TextSystem>`. The `text_system_moves_to_a_background_task`
//! test below holds the compile check. So the probe runs whole on
//! `cx.background_spawn`, not in chunks on the main thread.
//!
//! The probe runs once per session, started by [`ensure_loaded`] (never at
//! startup, never on the render path). The task is detached and owned by
//! nobody, so closing Preferences neither cancels nor repeats it. A probe
//! that panics leaves the state `Idle` again, so the next open retries.

use std::sync::Arc;

use gpui_kit::{App, AppContext as _, Global, SharedString, TextSystem, font, px};

/// The glyphs whose advances must match for a family to count as monospace.
pub const PROBE_GLYPHS: [char; 5] = ['i', 'm', 'W', '.', '0'];

/// `true` when every [`PROBE_GLYPHS`] glyph has a positive advance within 1
/// percent of the first one's. `advance` gives a glyph's advance, `None` when
/// the face has no such glyph.
pub fn is_monospace(advance: impl Fn(char) -> Option<f32>) -> bool {
    let Some(first) = advance(PROBE_GLYPHS[0]) else {
        return false;
    };
    if !(first.is_finite() && first > 0.0) {
        return false;
    }
    PROBE_GLYPHS[1..]
        .iter()
        .all(|&c| advance(c).is_some_and(|a| a > 0.0 && (a - first).abs() <= first * 0.01))
}

/// Whether `name` loaded as a face of its own, not as the text system's
/// fallback face standing in for it.
///
/// `is_fallback_face`: the name resolved to the same face as a name no
/// family has. That alone cannot tell a missing family from the fallback
/// family itself, which resolves to that face too. So a collision asks
/// `loaded_family`: the family the text system loaded that face for
/// directly (`TextSystem::get_font_for_id`). The text system never records
/// a substituted name as a direct load, so only the fallback family itself
/// matches.
pub fn loaded_as_itself(
    name: &str,
    is_fallback_face: bool,
    loaded_family: impl FnOnce() -> Option<SharedString>,
) -> bool {
    !is_fallback_face || loaded_family().is_some_and(|family| family.as_ref() == name)
}

/// The installed families that are monospace, sorted case-insensitively.
///
/// Names starting with `.` are the platform's hidden faces and are skipped.
/// The advances come from the same call as
/// [`CellMetrics::from_text_system`](crate::grid::CellMetrics::from_text_system),
/// at one fixed size. A family the text system cannot load resolves to its
/// fallback face; it is skipped rather than judged by that face
/// ([`loaded_as_itself`]).
pub fn list(text_system: &TextSystem) -> Vec<String> {
    let size = px(PROBE_SIZE);
    // No family has this name, so it forces the text system's fallback
    // font. Resolving it logs one "failed to load font" warning; the text
    // system caches the result, so that is about once per session.
    // `resolve_font` panics only if none of the system's fallbacks resolve,
    // and the app's own font resolution takes that same path.
    let fallback = text_system.resolve_font(&font("\u{1}kimun-no-such-family"));
    let mut names: Vec<String> = text_system
        .all_font_names()
        .into_iter()
        .filter(|name| !name.starts_with('.'))
        .filter(|name| {
            let id = text_system.resolve_font(&font(name.clone()));
            // Asked only on a collision: the lookup walks the font cache.
            loaded_as_itself(name, id == fallback, || {
                text_system.get_font_for_id(id).map(|f| f.family)
            }) && is_monospace(|c| {
                text_system
                    .advance(id, size, c)
                    .ok()
                    .map(|s| f32::from(s.width))
            })
        })
        .collect();
    names.sort_by_cached_key(|name| name.to_lowercase());
    names.dedup();
    names
}

/// The font size, in pixels, the probe measures advances at.
const PROBE_SIZE: f32 = 16.0;

/// The family list's load state, held as a GPUI global.
#[derive(Clone)]
pub enum MonoFamilies {
    Idle,
    Loading,
    Ready(Arc<[String]>),
}

impl Global for MonoFamilies {}

/// Start the one probe of the session, when none has started yet.
pub fn ensure_loaded(cx: &mut App) {
    ensure_loaded_with(cx, list);
}

fn ensure_loaded_with<F>(cx: &mut App, list: F)
where
    F: FnOnce(&TextSystem) -> Vec<String> + Send + 'static,
{
    if !matches!(state(cx), MonoFamilies::Idle) {
        return;
    }
    cx.set_global(MonoFamilies::Loading);
    let text_system = cx.text_system().clone();
    // A panic in the probe must not leave the state `Loading` for good.
    let probe = cx.background_spawn(async move {
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| list(&text_system))).ok()
    });
    cx.spawn(async move |cx| {
        let families = probe.await;
        cx.update(|cx| cx.set_global(after_probe(families)));
    })
    .detach();
}

/// The state a finished probe leaves: `Ready` with its list, or `Idle`
/// when it panicked (`None`), so the next [`ensure_loaded`] tries again.
fn after_probe(families: Option<Vec<String>>) -> MonoFamilies {
    match families {
        Some(families) => MonoFamilies::Ready(families.into()),
        None => {
            tracing::warn!("listing the monospace font families failed; trying again later");
            MonoFamilies::Idle
        }
    }
}

/// The current load state; `Idle` before [`ensure_loaded`] is first called.
pub fn state(cx: &App) -> MonoFamilies {
    cx.try_global::<MonoFamilies>()
        .cloned()
        .unwrap_or(MonoFamilies::Idle)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use gpui_kit::TestAppContext;

    /// An advance function over `table`, in `PROBE_GLYPHS` order.
    fn advances(table: [Option<f32>; 5]) -> impl Fn(char) -> Option<f32> {
        move |c| {
            let i = PROBE_GLYPHS.iter().position(|&g| g == c)?;
            table[i]
        }
    }

    #[test]
    fn is_monospace_table() {
        let cases: [(&str, [Option<f32>; 5], bool); 6] = [
            ("five equal advances", [Some(8.0); 5], true),
            (
                "i narrower than m",
                [Some(4.0), Some(8.0), Some(8.0), Some(8.0), Some(8.0)],
                false,
            ),
            (
                "a missing glyph",
                [Some(8.0), Some(8.0), None, Some(8.0), Some(8.0)],
                false,
            ),
            ("a zero advance", [Some(0.0); 5], false),
            (
                "within 1 percent",
                [Some(10.0), Some(10.09), Some(9.91), Some(10.0), Some(10.05)],
                true,
            ),
            (
                "2 percent apart",
                [Some(10.0), Some(10.2), Some(10.0), Some(10.0), Some(10.0)],
                false,
            ),
        ];
        for (name, table, expected) in cases {
            assert_eq!(is_monospace(advances(table)), expected, "{name}");
        }
    }

    #[test]
    fn a_family_is_loaded_unless_the_fallback_face_stands_in_for_it() {
        let never = || -> Option<SharedString> { panic!("asked only on a collision") };
        assert!(loaded_as_itself("Fira Code", false, never), "its own face");
        // The fallback family itself: the text system loaded the face for it.
        let fallback_family = || Some(SharedString::from("DejaVu Sans Mono"));
        assert!(loaded_as_itself("DejaVu Sans Mono", true, fallback_family));
        // A missing family: the face was loaded for another family.
        assert!(!loaded_as_itself("Gone", true, fallback_family));
        assert!(!loaded_as_itself("Gone", true, || None), "no direct load");
    }

    #[test]
    fn a_finished_probe_is_ready_and_a_panicked_one_is_idle_again() {
        assert!(matches!(
            after_probe(Some(vec!["A".into()])),
            MonoFamilies::Ready(list) if *list == ["A".to_string()]
        ));
        assert!(matches!(after_probe(None), MonoFamilies::Idle));
    }

    #[gpui_kit::test]
    fn a_panicking_probe_leaves_the_state_idle_and_a_later_open_retries(cx: &mut TestAppContext) {
        cx.update(|cx| ensure_loaded_with(cx, |_| panic!("probe failed")));
        cx.run_until_parked();
        assert!(matches!(cx.read(state), MonoFamilies::Idle));
        cx.update(|cx| ensure_loaded_with(cx, |_| vec!["Fira Code".to_string()]));
        cx.run_until_parked();
        assert!(matches!(cx.read(state), MonoFamilies::Ready(_)));
    }

    #[test]
    fn text_system_moves_to_a_background_task() {
        fn send_sync<T: Send + Sync>() {}
        send_sync::<TextSystem>();
    }

    #[gpui_kit::test]
    fn ensure_loaded_runs_the_probe_once(cx: &mut TestAppContext) {
        let calls = Arc::new(AtomicUsize::new(0));
        for _ in 0..3 {
            let calls = calls.clone();
            cx.update(|cx| {
                ensure_loaded_with(cx, move |_| {
                    calls.fetch_add(1, Ordering::SeqCst);
                    vec!["Fira Code".to_string()]
                })
            });
        }
        cx.run_until_parked();
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        cx.update(|cx| match state(cx) {
            MonoFamilies::Ready(families) => assert_eq!(&*families, ["Fira Code".to_string()]),
            _ => panic!("the state is not Ready"),
        });
    }
}
