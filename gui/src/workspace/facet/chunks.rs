//! Chunk planning of the facet view.
//!
//! It owns which chunks show and the read-only editor of each chunk. It
//! does not draw. Each editor measures its own height (module
//! `editor_view::measure`), so nothing here sizes a chunk.

use gpui_kit::{AppContext as _, Context};
use kimun_core::note::Reveal;

use super::FacetView;
use super::members::chunk_plan;
use crate::editor_view::{EditorEvent, EditorView};

impl FacetView {
    /// Show what the picks select, nothing revealed.
    pub(super) fn select(&mut self, cx: &mut Context<Self>) {
        self.members.select(self.bar.picks());
        self.plan(cx);
    }

    /// Plan the chunks for every member's `shown` and bring the editors in
    /// step: one per drawn chunk, an editor whose text is unchanged left
    /// alone.
    pub(super) fn plan(&mut self, cx: &mut Context<Self>) {
        #[cfg(test)]
        {
            self.plans_run += 1;
        }
        let (old_plans, old_refs) = self.members.plan(self.limit);
        let len = self.members.len();
        self.chunks.truncate(len);
        self._chunk_events.truncate(len);
        for i in 0..len {
            let plan = self
                .members
                .chunk(i)
                .expect("every chunk below `len` has a plan");
            match self.chunks.get(i) {
                Some(editor) => {
                    let old = chunk_plan(&old_plans, &old_refs, i);
                    if old.is_none_or(|old| old.text != plan.text) {
                        editor.update(cx, |editor, cx| editor.set_text_read_only(&plan.text, cx));
                    }
                }
                None => {
                    let editor = cx.new(|cx| EditorView::new_read_only(&plan.text, cx));
                    let events = cx.subscribe(&editor, |this, editor, event: &EditorEvent, cx| {
                        let EditorEvent::ReadOnlyFollow { target } = event else {
                            return;
                        };
                        if let Some(i) = this.chunks.iter().position(|c| *c == editor) {
                            this.follow(i, target.clone(), cx);
                        }
                    });
                    self.chunks.push(editor);
                    self._chunk_events.push(events);
                }
            }
        }
        self.close_list_if_gone(cx);
        // A plan can change the picks, the summary and the counts. The bar
        // eases to its new height only when what it draws changed.
        let shape = self.bar_shape();
        self.bar_motion.mark_if_shape_changed(shape);
        cx.notify();
    }

    /// Reveal one block hidden in gap `g`: next to the chunk above it
    /// ([`Reveal::Below`]) or below it ([`Reveal::Above`]), inside that
    /// chunk's note. Nothing when the gap has no chunk on that side.
    pub(super) fn reveal(&mut self, g: usize, dir: Reveal, cx: &mut Context<Self>) {
        let gap = self.members.gaps.get(g);
        let chunk = match dir {
            Reveal::Below => gap.and_then(|gap| gap.above),
            Reveal::Above => gap.and_then(|gap| gap.below),
        };
        if let Some(chunk) = chunk {
            self.members.reveal(chunk, dir);
            self.plan(cx);
        }
    }

    /// Reveal every block hidden in gap `g`: `Show all`.
    pub(super) fn reveal_all(&mut self, g: usize, cx: &mut Context<Self>) {
        self.members.reveal_gap(g);
        self.plan(cx);
    }

    pub(super) fn show_more(&mut self, cx: &mut Context<Self>) {
        self.limit = self.limit.saturating_mul(2);
        self.plan(cx);
    }

    /// `Show N more`, while chunks are left out: N is what the next click
    /// shows, the limit or what is left when that is less.
    pub(crate) fn show_more_label(&self) -> Option<String> {
        let cut = self.members.cut;
        (cut > 0).then(|| format!("Show {} more", cut.min(self.limit)))
    }

    /// `100 of 340 pieces shown`, beside `Show N more` while it shows.
    pub(super) fn show_more_count(&self) -> Option<String> {
        let (drawn, cut) = (self.members.len(), self.members.cut);
        (cut > 0).then(|| format!("{drawn} of {} pieces shown", drawn + cut))
    }
}
