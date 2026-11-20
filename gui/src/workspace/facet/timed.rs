//! A timed note of the facet view.
//!
//! It owns the timer that ends a note after a while. The `Undo` offer after
//! a clear and the report of a refresh use it. It owns the spawn, the update
//! of the view and the redraw. The caller says what the end changes.

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use gpui_kit::{Context, Task};

use super::FacetView;

/// The timer of one note. A new start replaces the timer that runs.
/// Dropping the struct cancels the timer, so a dropped view runs no end.
#[derive(Default)]
pub(super) struct Timed {
    /// The timer. `None` before a start and after a cancel.
    task: Option<Task<()>>,
    /// Set by the timer when its time passed, before the expire callback
    /// runs. Each start gets a new flag, so an old timer cannot set it.
    fired: Rc<Cell<bool>>,
}

impl Timed {
    /// Run `expire` on the view after `after`, then redraw the view. A
    /// timer that runs already is cancelled. The `expire` callback ends the
    /// note.
    pub(super) fn start(
        &mut self,
        after: Duration,
        cx: &mut Context<FacetView>,
        expire: impl FnOnce(&mut FacetView, &mut Context<FacetView>) + 'static,
    ) {
        let fired = Rc::new(Cell::new(false));
        self.fired = fired.clone();
        self.task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(after).await;
            fired.set(true);
            // A view that is gone has nothing to end.
            this.update(cx, |view, cx| {
                expire(view, cx);
                cx.notify();
            })
            .ok();
        }));
    }

    /// Stop the timer. Its `expire` callback does not run.
    pub(super) fn cancel(&mut self) {
        self.task = None;
        self.fired = Rc::default();
    }

    /// Whether a timer runs: it was started, is not cancelled and its time
    /// has not passed.
    pub(super) fn is_active(&self) -> bool {
        self.task.is_some() && !self.fired.get()
    }
}
