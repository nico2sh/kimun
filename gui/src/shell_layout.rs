//! The shell's toolkit-free half (ADR-0050): which views the rail offers,
//! the empty slot's hint and the sizing floors in rems. The round trip
//! between the layout tree and the `[gui]` table is [`crate::shell_tree`];
//! the views' titles and names are `logic::view_registry`'s. The workspace
//! view renders and routes; everything here is plain data and tested
//! without a window.

use kimun_notes::logic::nav::DrawerView;
use kimun_notes::logic::rag_caps::Caps;

/// The rail's width, in rems: one icon cell plus its inset.
pub const RAIL_REMS: f32 = 3.0;
/// The width of a slot opened empty, in rems (a slot showing a view takes
/// the view's minimum, `view_registry`): room for a short label and its
/// icon.
pub const SLOT_MIN_REMS: f32 = 10.0;
/// An editor group's minimum width, in rems: a readable line of text. The
/// editor never collapses; the slots beside it do first.
pub const EDITOR_MIN_REMS: f32 = 20.0;

/// The views the rail offers when the kimün server can do `caps`, top to
/// bottom, in the TUI rail's order; the last (settings) is pinned to the
/// bottom, as in the TUI. SEM is offered only while the server can search
/// (`caps.semantic`), ASK (the Thread, ADR-0053) only while it can answer
/// (`caps.ask`); the rail is rebuilt from this list whenever a
/// capability flips. A view shown in a slot stays there when its cell
/// goes.
pub fn rail_views(caps: Caps) -> &'static [DrawerView] {
    use DrawerView::*;
    // One fixed list per capability combination: the rail is read on every
    // frame and every rail key, and never needs to own its views.
    const BOTH: &[DrawerView] = &[Files, Find, Semantic, Ask, Tags, Links, Outline, Config];
    const SEMANTIC: &[DrawerView] = &[Files, Find, Semantic, Tags, Links, Outline, Config];
    const ASK: &[DrawerView] = &[Files, Find, Ask, Tags, Links, Outline, Config];
    const NEITHER: &[DrawerView] = &[Files, Find, Tags, Links, Outline, Config];
    match (caps.semantic, caps.ask) {
        (true, true) => BOTH,
        (true, false) => SEMANTIC,
        (false, true) => ASK,
        (false, false) => NEITHER,
    }
}

/// The hint an empty slot shows: how a view gets there.
/// `right` and `left` are the move chords as the keymap binds them (`None`
/// when unbound, and then left out).
pub fn empty_slot_hint(right: Option<&str>, left: Option<&str>) -> String {
    let chords: Vec<&str> = [right, left].into_iter().flatten().collect();
    if chords.is_empty() {
        "No view here. Pick one from the rail.".to_string()
    } else {
        format!(
            "No view here. Move one with {}, or pick one from the rail.",
            chords.join(" / ")
        )
    }
}

/// Whether the window hosts `view` as a slot view the leader may open:
/// every view the rail can offer, whether or not its cell shows right now.
pub fn hosts(view: DrawerView) -> bool {
    rail_views(Caps {
        semantic: true,
        ask: true,
    })
    .contains(&view)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn caps(semantic: bool, ask: bool) -> Caps {
        Caps { semantic, ask }
    }

    #[test]
    fn the_empty_slot_hint_names_the_bound_chords() {
        assert_eq!(
            empty_slot_hint(Some("Alt+Shift+Right"), Some("Alt+Shift+Left")),
            "No view here. Move one with Alt+Shift+Right / Alt+Shift+Left, \
             or pick one from the rail."
        );
        assert_eq!(
            empty_slot_hint(None, Some("Alt+Shift+Left")),
            "No view here. Move one with Alt+Shift+Left, or pick one from the rail."
        );
        assert_eq!(
            empty_slot_hint(None, None),
            "No view here. Pick one from the rail."
        );
    }

    #[test]
    fn the_rail_offers_sem_and_ask_only_while_the_server_can() {
        use DrawerView::*;
        let without = [Files, Find, Tags, Links, Outline, Config];
        assert_eq!(rail_views(caps(false, false)), without);
        assert_eq!(
            rail_views(caps(false, true)),
            [Files, Find, Ask, Tags, Links, Outline, Config]
        );
        let with = [Files, Find, Semantic, Tags, Links, Outline, Config];
        assert_eq!(rail_views(caps(true, false)), with);
        // The TUI's ITEMS order: ASK right after SEM.
        assert_eq!(
            rail_views(caps(true, true)),
            [Files, Find, Semantic, Ask, Tags, Links, Outline, Config]
        );
        // Settings stays pinned last either way.
        assert_eq!(rail_views(caps(true, true)).last(), Some(&Config));
        // SEM and ASK are hosted with their cells gone.
        assert!(hosts(Semantic));
        assert!(hosts(Ask));
    }
}
