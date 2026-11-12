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
//! nobody, so closing Preferences neither cancels nor repeats it.

use std::sync::Arc;

use gpui_kit::{App, AppContext as _, Global, TextSystem, font, px};

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

/// The installed families that are monospace, sorted case-insensitively.
///
/// Names starting with `.` are the platform's hidden faces and are skipped.
/// The advances come from the same call as
/// [`CellMetrics::from_text_system`](crate::grid::CellMetrics::from_text_system),
/// at one fixed size. A family the text system cannot load resolves to its
/// fallback face; it is skipped rather than judged by that face.
pub fn list(text_system: &TextSystem) -> Vec<String> {
    let size = px(PROBE_SIZE);
    // No family has this name: it resolves to the fallback face.
    let fallback = text_system.resolve_font(&font("\u{1}kimun-no-such-family"));
    let mut names: Vec<String> = text_system
        .all_font_names()
        .into_iter()
        .filter(|name| !name.starts_with('.'))
        .filter(|name| {
            let id = text_system.resolve_font(&font(name.clone()));
            id != fallback
                && is_monospace(|c| {
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
    let probe = cx.background_spawn(async move { list(&text_system) });
    cx.spawn(async move |cx| {
        let families = probe.await;
        cx.update(|cx| cx.set_global(MonoFamilies::Ready(families.into())));
    })
    .detach();
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
