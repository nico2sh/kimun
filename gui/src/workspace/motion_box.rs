//! The three pieces what opens and closes draws with (ADR-0060): the
//! content fade, the clip while the frame moves, and the cover over a
//! leaving element. The strip and a tab label use all three. The rail and a
//! slot use the fade and the cover. Their frames clip at all times and size
//! themselves, so they do not use `clipped_while`. A fix to one of the
//! pieces is a fix to every widget that uses it.

use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{InteractiveElement as _, ParentElement, Styled, div};

pub(super) trait MotionBox: Styled + ParentElement + FluentBuilder + Sized {
    /// The content as visible as its content value says (0 to 1).
    fn faded(self, content: f32) -> Self {
        self.when(content < 1.0, |element| element.opacity(content))
    }

    /// Clip what is inside while the frame `moving`, sized by `size`. At
    /// rest nothing is clipped and nothing is sized: the element is its
    /// content, to the pixel.
    fn clipped_while(self, moving: bool, size: impl FnOnce(Self) -> Self) -> Self {
        self.when(moving, |element| size(element.overflow_hidden()))
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

    use super::MotionBox;

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
}
