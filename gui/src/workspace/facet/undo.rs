//! The undo of a clear in the facet view.
//!
//! It owns the stack of cleared pick sets and the timed `Undo` offer.
//! It does not save to the session.

use gpui_kit::{
    Context, Div, ParentElement as _, Role, StatefulInteractiveElement as _, Styled as _, div,
};
use kimun_core::note::Facet;
use kimun_notes::settings::themes::Theme;

use super::keys::Stop;
use super::render::muted_note;
use super::timed::Timed;
use super::{CLEARED_ID, FacetEvent, FacetView, UNDO_ID};
use crate::workspace::facet_ui::chip;

/// The most cleared pick sets Ctrl+Z can bring back.
const UNDO_MAX: usize = 20;
/// How long `Picks cleared` and its `Undo` button stay after a clear.
const UNDO_SHOWN: std::time::Duration = std::time::Duration::from_secs(8);

/// The pick sets that clears removed, oldest first, at most [`UNDO_MAX`],
/// and the timed offer of the last one.
#[derive(Default)]
pub(super) struct Undo {
    /// The cleared pick sets, oldest first. Ctrl+Z brings back the last one.
    sets: Vec<Vec<Facet>>,
    /// While it runs, the summary row says `Picks cleared` and offers
    /// `Undo`. It ends after [`UNDO_SHOWN`]. A pick, an unpick or an undo
    /// ends it sooner.
    shown: Timed,
}

impl Undo {
    /// Put `picks` on the stack, dropping the oldest set past [`UNDO_MAX`].
    pub(super) fn push(&mut self, picks: Vec<Facet>) {
        self.sets.push(picks);
        while self.sets.len() > UNDO_MAX {
            self.sets.remove(0);
        }
    }

    /// Take the last cleared set off the stack.
    pub(super) fn pop(&mut self) -> Option<Vec<Facet>> {
        self.sets.pop()
    }

    /// The cleared sets, oldest first (tests).
    #[cfg(test)]
    pub(super) fn sets(&self) -> &[Vec<Facet>] {
        &self.sets
    }

    /// Whether the summary row offers `Undo`: the timer runs and a set is
    /// on the stack.
    pub(super) fn offered(&self) -> bool {
        self.shown.is_active() && !self.sets.is_empty()
    }

    /// Offer `Undo` for [`UNDO_SHOWN`]. Then `expire` runs on the view.
    pub(super) fn offer(
        &mut self,
        cx: &mut Context<FacetView>,
        expire: impl FnOnce(&mut FacetView, &mut Context<FacetView>) + 'static,
    ) {
        self.shown.start(UNDO_SHOWN, cx, expire);
    }

    /// Whether the timer of the offer runs (tests).
    #[cfg(test)]
    pub(super) fn timer_active(&self) -> bool {
        self.shown.is_active()
    }

    /// Stop the offer of `Undo` and its timer.
    pub(super) fn hide(&mut self) {
        self.shown.cancel();
    }
}

impl FacetView {
    /// Unpick everything: the whole note again, reveals reset. The picks
    /// go on the undo stack, and the summary row offers `Undo` for a while.
    pub(super) fn clear(&mut self, cx: &mut Context<Self>) {
        if self.bar.picks().is_empty() {
            return;
        }
        self.undo.push(self.bar.picks().to_vec());
        self.bar.clear();
        self.end_refresh_report();
        self.undo.offer(cx, |this, _| {
            this.undo.hide();
            // `Picks cleared` leaves the bar. The direct mark moves it.
            // The kept shape follows, so the next plan sees no change.
            this.bar_motion.mark();
            let shape = this.bar_shape();
            this.bar_motion.mark_if_shape_changed(shape);
        });
        self.select(cx);
        cx.emit(FacetEvent::PicksChanged);
    }

    /// Ctrl+Z, `Undo`: bring back the picks of the last clear, less those
    /// the notes no longer hold. Picks made since the clear go on the stack
    /// first, unless they equal the set brought back. False when no clear
    /// is left to undo.
    pub(in crate::workspace) fn undo_clear(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(picks) = self.undo.pop() else {
            return false;
        };
        // Picks made since the clear go on the stack, so the next Ctrl+Z
        // brings them back. Picks equal to the set brought back do not: the
        // next Ctrl+Z would change nothing.
        if !self.bar.picks().is_empty() && self.bar.picks() != picks.as_slice() {
            self.undo.push(self.bar.picks().to_vec());
        }
        self.bar.set_picks(picks);
        self.bar.prune(&self.members.merged);
        self.after_pick(cx);
        true
    }

    /// The summary row just after a clear: `Picks cleared` and the `Undo`
    /// button that brings them back.
    pub(super) fn render_cleared(&self, t: &Theme, cx: &mut Context<Self>) -> Div {
        const TEXT: &str = "Picks cleared";
        div()
            .flex()
            .flex_none()
            .w_full()
            .items_center()
            .gap_2()
            .child(muted_note(CLEARED_ID, Role::Status, TEXT.into(), t))
            .child(self.button(
                Stop::Undo,
                chip(UNDO_ID.into(), "Undo", false, t).aria_label("Undo clear"),
                t,
                cx,
            ))
    }
}
