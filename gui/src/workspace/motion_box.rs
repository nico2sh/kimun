//! The three pieces what opens and closes draws with (ADR-0060): the
//! content fade, the frame, and the cover over a leaving element. The rail,
//! a slot, the strip and a tab label use all three. A fix to one of the
//! pieces is a fix to every widget that uses it.
//!
//! A frame follows one of two rules, named by [`Clip`]. The rail and a slot
//! clip at all times ([`Clip::Always`]): the rail sizes itself from the
//! frame value, and a slot takes the cell its split gives it. The strip and a tab label are their content at rest,
//! to the pixel, and clip and size only while they move
//! ([`Clip::WhileMoving`]).

use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{InteractiveElement as _, ParentElement, Styled, div};

/// When a frame clips what is inside it and takes the size it is given.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Clip {
    /// At all times.
    Always,
    /// Only while the frame moves (`true`). At rest nothing is clipped and
    /// nothing is sized: the frame is its content.
    WhileMoving(bool),
}

impl Clip {
    fn applies(self) -> bool {
        match self {
            Clip::Always => true,
            Clip::WhileMoving(moving) => moving,
        }
    }
}

pub(super) trait MotionBox: Styled + ParentElement + FluentBuilder + Sized {
    /// The content as visible as its content value says (0 to 1).
    fn faded(self, content: f32) -> Self {
        self.when(content < 1.0, |element| element.opacity(content))
    }

    /// The element as the frame of what opens and closes: a flex box that
    /// the cover can lie over. Where `clip` says, it clips what is inside
    /// and takes the size that `size` gives it. It grows from its end edge
    /// (`anchor_end`) or from its start edge.
    fn framed(self, clip: Clip, anchor_end: bool, size: impl FnOnce(Self) -> Self) -> Self {
        self.relative()
            .flex()
            .when(clip.applies(), |frame| size(frame.overflow_hidden()))
            .when(anchor_end, |frame| frame.justify_end())
    }

    /// While `leaving`, nothing under the element takes the pointer: a
    /// last child lies over it all and takes every hit. The only place that
    /// builds the cover.
    fn covered_while(self, leaving: bool) -> Self {
        self.when(leaving, |element| {
            element
                .relative()
                .child(div().absolute().top_0().left_0().size_full().occlude())
        })
    }
}

impl<T: Styled + ParentElement + FluentBuilder + Sized> MotionBox for T {}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::rc::Rc;

    use gpui_kit::base::TestSupportExt as _;
    use gpui_kit::test::TestWindowExt;
    use gpui_kit::{
        AppContext as _, Context, InteractiveElement, IntoElement, ParentElement, Render,
        StatefulInteractiveElement, Styled, TestAppContext, Window, WindowOptions, div, px,
    };

    use super::{Clip, MotionBox};

    struct Probe {
        plain: Rc<Cell<u32>>,
        covered: Rc<Cell<u32>>,
    }

    impl Render for Probe {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            let plain = self.plain.clone();
            let covered = self.covered.clone();
            div()
                .flex()
                .flex_col()
                .size_full()
                .child(
                    div()
                        .id("plain")
                        .test_support()
                        .w(px(100.))
                        .h(px(40.))
                        .on_click(move |_, _, _| plain.set(plain.get() + 1))
                        .covered_while(false),
                )
                .child(
                    div()
                        .id("covered")
                        .test_support()
                        .w(px(100.))
                        .h(px(40.))
                        .on_click(move |_, _, _| covered.set(covered.get() + 1))
                        .covered_while(true),
                )
        }
    }

    #[gpui_kit::test]
    fn a_cover_blocks_the_pointer_only_while_leaving(cx: &mut TestAppContext) {
        let plain = Rc::new(Cell::new(0));
        let covered = Rc::new(Cell::new(0));
        let probe = Probe {
            plain: plain.clone(),
            covered: covered.clone(),
        };
        let (handle, _) = cx
            .update(|cx| {
                gpui_kit::open_window(WindowOptions::default(), cx, |_, cx| cx.new(|_| probe))
            })
            .expect("open the probe window");
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.click("plain", cx);
            window.click("covered", cx);
        })
        .unwrap();
        assert_eq!(plain.get(), 1, "an uncovered element takes the click");
        assert_eq!(covered.get(), 0, "a covered element takes none");
    }

    struct FrameProbe {
        always: Rc<Cell<u32>>,
        resting: Rc<Cell<u32>>,
        moving: Rc<Cell<u32>>,
    }

    impl Render for FrameProbe {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            // Each frame is 100 px wide and holds a child 400 px wide, whose
            // middle lies outside the frame: a click there reaches the child
            // only if the frame does not clip.
            let child = |id: &'static str, hits: Rc<Cell<u32>>| {
                div()
                    .id(id)
                    .test_support()
                    .flex_none()
                    .w(px(400.))
                    .h(px(40.))
                    .on_click(move |_, _, _| hits.set(hits.get() + 1))
            };
            div()
                .flex()
                .flex_col()
                .size_full()
                .child(
                    div()
                        .w(px(100.))
                        .h(px(40.))
                        .framed(Clip::Always, false, |frame| frame.w(px(100.)))
                        .child(child("always", self.always.clone())),
                )
                .child(
                    div()
                        .w(px(100.))
                        .h(px(40.))
                        .framed(Clip::WhileMoving(false), false, |frame| frame.w(px(100.)))
                        .child(child("resting", self.resting.clone())),
                )
                .child(
                    div()
                        .w(px(100.))
                        .h(px(40.))
                        .framed(Clip::WhileMoving(true), false, |frame| frame.w(px(100.)))
                        .child(child("moving", self.moving.clone())),
                )
        }
    }

    #[gpui_kit::test]
    fn framed_clips_only_as_the_clip_says(cx: &mut TestAppContext) {
        let (always, resting, moving) = (
            Rc::new(Cell::new(0)),
            Rc::new(Cell::new(0)),
            Rc::new(Cell::new(0)),
        );
        let probe = FrameProbe {
            always: always.clone(),
            resting: resting.clone(),
            moving: moving.clone(),
        };
        let (handle, _) = cx
            .update(|cx| {
                gpui_kit::open_window(WindowOptions::default(), cx, |_, cx| cx.new(|_| probe))
            })
            .expect("open the probe window");
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.click("always", cx);
            window.click("resting", cx);
            window.click("moving", cx);
        })
        .unwrap();
        assert_eq!(
            always.get(),
            0,
            "an always-clipped frame hides its overflow"
        );
        assert_eq!(resting.get(), 1, "a resting frame clips nothing");
        assert_eq!(moving.get(), 0, "a moving frame clips");
    }
}
