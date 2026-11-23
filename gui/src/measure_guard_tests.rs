//! The guard that lets prepaint of a text-sized editor skip its own frame
//! preparation. The measure prepares the frame in layout. Nothing may edit
//! the editor between that layout and prepaint. These tests break that
//! rule on purpose, and check that a change made outside that window is
//! drawn with the current rows in the next frame.

use gpui_kit::base::TestSupportExt;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    AppContext, Context, Entity, InteractiveElement, IntoElement, ParentElement, Render, Styled,
    TestAppContext, VisualTestContext, Window, canvas, div, px, size,
};
use kimun_notes::logic::editor::rope_buffer::CursorMove;

use crate::editor_view::EditorView;
use crate::measure_tests::{assert_as_tall_as_rows, init, piece, piece_id, visual_rows};

/// What a hook does to the piece.
type Hook = Box<dyn Fn(&mut EditorView)>;

/// One text-sized piece, with a hook that runs in the prepaint of an
/// element drawn before the piece, and one in the paint of an element
/// drawn after it. Layout of the whole window runs before any prepaint.
/// So the first hook runs between the measure and the piece's prepaint.
struct Hooked {
    piece: Entity<EditorView>,
    before_prepaint: Option<Hook>,
    after_paint: Option<Hook>,
}

impl Render for Hooked {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let early = self.piece.clone();
        let late = self.piece.clone();
        let before = self.before_prepaint.take();
        let after = self.after_paint.take();
        let hook_before = canvas(
            move |_, _, cx| {
                if let Some(hook) = &before {
                    early.update(cx, |view, _| hook(view));
                }
            },
            |_, _, _, _| {},
        )
        .size_0();
        let hook_after = canvas(
            |_, _, _| {},
            move |_, _, _, cx| {
                if let Some(hook) = &after {
                    late.update(cx, |view, _| hook(view));
                }
            },
        )
        .size_0();
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(hook_before)
            .child(
                div()
                    .id(piece_id(0))
                    .test_support()
                    .flex()
                    .flex_col()
                    .flex_none()
                    .overflow_hidden()
                    .child(div().flex_none().child(self.piece.clone()))
                    .child(hook_after),
            )
    }
}

/// A window 300 px wide with one text-sized piece over `text`, drawn once.
fn open_hooked<'a>(
    cx: &'a mut TestAppContext,
    text: &str,
) -> (
    Entity<Hooked>,
    Entity<EditorView>,
    &'a mut VisualTestContext,
) {
    init(cx);
    let text = text.to_string();
    let (hooked, cx) = cx.add_window_view(move |_, cx| Hooked {
        piece: cx.new(|cx| piece(&text, true, cx)),
        before_prepaint: None,
        after_paint: None,
    });
    cx.simulate_resize(size(px(300.), px(600.)));
    let piece = cx.update(|_, cx| hooked.read(cx).piece.clone());
    (hooked, piece, cx)
}

/// A line with a long link that a caret on its label writes out.
fn linked() -> String {
    let url = "https://example.com/a/very/long/path/that/goes/on/and/on/0123456789";
    format!("intro\nsee [label]({url}) and more words after it\n")
}

/// Put the caret on the link's label, on the second row, as a press does:
/// the caret is the user's, so its row shows as written.
fn caret_on_link(view: &mut EditorView) {
    view.editor.session_mut().display.set_reveal_caret_row(true);
    let buffer = view.editor.buffer_mut();
    buffer.move_cursor(CursorMove::Down);
    for _ in 0..5 {
        buffer.move_cursor(CursorMove::Forward);
    }
}

#[gpui_kit::test]
#[cfg(debug_assertions)]
#[should_panic(expected = "changed between its measure and its prepaint")]
fn an_edit_between_measure_and_prepaint_trips_the_debug_assertion(cx: &mut TestAppContext) {
    let (hooked, _piece, cx) = open_hooked(cx, &linked());
    hooked.update(cx, |hooked, cx| {
        hooked.before_prepaint = Some(Box::new(|view| {
            view.editor
                .set_text("new text after the measure".to_string());
        }));
        cx.notify();
    });
    cx.update(|window, cx| window.render_frame(cx));
}

#[gpui_kit::test]
#[cfg(debug_assertions)]
#[should_panic(expected = "changed between its measure and its prepaint")]
fn a_caret_move_between_measure_and_prepaint_trips_the_debug_assertion(cx: &mut TestAppContext) {
    let (hooked, _piece, cx) = open_hooked(cx, &linked());
    hooked.update(cx, |hooked, cx| {
        hooked.before_prepaint = Some(Box::new(caret_on_link));
        cx.notify();
    });
    cx.update(|window, cx| window.render_frame(cx));
}

#[gpui_kit::test]
fn a_caret_move_in_a_frame_is_drawn_with_current_rows(cx: &mut TestAppContext) {
    let (hooked, piece, cx) = open_hooked(cx, &linked());
    assert_as_tall_as_rows(cx, &piece, "the first frame");
    let folded = cx.update(|_, cx| visual_rows(&piece, cx));
    // The caret moves in the paint of this frame, after the piece's prepaint.
    hooked.update(cx, |hooked, cx| {
        hooked.after_paint = Some(Box::new(caret_on_link));
        cx.notify();
    });
    cx.update(|window, cx| window.render_frame(cx));
    // The next frame lays the moved caret out: the link shows written out.
    cx.update(|window, cx| window.render_frame(cx));
    let unfolded = cx.update(|_, cx| visual_rows(&piece, cx));
    assert!(
        unfolded > folded,
        "the link unfolds: {folded} to {unfolded}"
    );
    assert_as_tall_as_rows(cx, &piece, "the next frame");
    let drawn: String = cx.update(|_, cx| {
        let painted = piece.read(cx).painted.clone().expect("painted");
        painted.row_paint.iter().map(|row| row.text()).collect()
    });
    assert!(
        drawn.contains("example.com"),
        "the drawn text is current: {drawn}"
    );
}
