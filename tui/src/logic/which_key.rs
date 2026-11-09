//! The **which-key** popup's model: what a pending leader sequence shows
//! once the user hesitates — the pressed keys, the current group's caption,
//! and the group's children as `key → target` rows. Read straight off the
//! `LeaderEngine`, so it cannot drift from the tree. Each frontend draws it
//! docked above its footer.
//!
//! Reveal policy ([`revealed`]): hidden while a sequence is typed fluently;
//! shown once the user hesitates past `leader_timeout_ms`. Hidden the instant
//! the sequence fires or cancels (the engine simply stops being pending).

use std::time::{Duration, Instant};

use crate::keys::leader::{DisplayChild, LeaderEngine};

/// The controls hint at the header's right.
pub const CONTROLS: &str = "Esc cancel · BkSp up";

/// What the which-key popup shows for the engine's pending node.
#[derive(Debug, Clone)]
pub struct WhichKey {
    pressed: String,
    caption: String,
    rows: Vec<DisplayChild>,
}

impl WhichKey {
    /// The model for `engine`'s current node. `gateway` labels the leader
    /// gateway at the head of the pressed keys.
    pub fn new(engine: &LeaderEngine, gateway: &str) -> Self {
        let node = engine.current_node();
        let mut pressed = gateway.to_string();
        for c in engine.path() {
            pressed.push(' ');
            pressed.push(*c);
        }
        Self {
            pressed,
            caption: node.label().to_string(),
            rows: node.display_children(),
        }
    }

    /// The gateway label and the keys pressed since, e.g. `Ctrl+B f`.
    pub fn pressed(&self) -> &str {
        &self.pressed
    }

    /// The current group's label (the tree root's label at the root).
    pub fn caption(&self) -> &str {
        &self.caption
    }

    /// The current group's children, digit runs collapsed into one row.
    pub fn rows(&self) -> &[DisplayChild] {
        &self.rows
    }
}

/// Whether the which-key popup shows at `now`: the sequence is pending and
/// has not advanced for `timeout`.
pub fn revealed(engine: &LeaderEngine, now: Instant, timeout: Duration) -> bool {
    engine
        .pending_since()
        .is_some_and(|since| now.saturating_duration_since(since) >= timeout)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hidden_until_the_hesitation_timeout() {
        let mut e = LeaderEngine::new();
        let timeout = Duration::from_millis(400);
        assert!(!revealed(&e, Instant::now(), timeout), "idle never shows");
        e.start();
        let since = e.pending_since().unwrap();
        assert!(!revealed(&e, since, timeout), "fluent typing never shows");
        assert!(!revealed(&e, since + Duration::from_millis(399), timeout));
        assert!(revealed(&e, since + timeout, timeout));
        e.cancel();
        assert!(!revealed(&e, since + timeout, timeout), "gone on cancel");
    }

    #[test]
    fn the_model_follows_the_pending_node() {
        let mut e = LeaderEngine::new();
        e.start();
        let root = WhichKey::new(&e, "Ctrl+B");
        assert_eq!(root.pressed(), "Ctrl+B");
        // 13 non-digit root children plus the collapsed 1–9 row.
        assert_eq!(root.rows().len(), 14);
        e.feed('f');
        let find = WhichKey::new(&e, "Ctrl+B");
        assert_eq!(find.pressed(), "Ctrl+B f");
        assert_eq!(find.caption(), e.current_node().label());
        assert!(find.rows().iter().any(|r| r.keys == "f" && !r.is_group));
    }
}
