//! The one placeholder piece of **Remote image**s (CONTEXT.md). It paints
//! every remote look that is not an image (the placeholder, the insecure
//! note, the blocked redirect, the load under way, the failure) and finds
//! the button under a press. The editor and the read-only views call it.
//! None of them paints a placeholder of its own.
//!
//! A look takes [`remote_rows`] rows: the text row, then the button row.
//! A surface that paints row by row (a preview row of a `uniform_list`)
//! passes the whole look as `rect` and its own row as `clip`. Each row then
//! paints its slice, and only the buttons inside its clip count as hits.

// The surfaces call this piece in the next steps of the remote images work.
// Until then the release build does not use it.
#![cfg_attr(not(test), allow(dead_code))]

use gpui_kit::{
    App, Bounds, ContentMask, Font, Pixels, Point, Rgba, SharedString, TextAlign, TextRun, Window,
    fill, point, px, size,
};
use kimun_notes::settings::themes::Theme;

use crate::image_core::{RemoteLook, remote_rows};
use crate::theme;

/// The padding of a button on each side, and the gap between two buttons,
/// in ems of the font size.
const PAD_EMS: f32 = 0.5;
/// The space above and below a button inside its row, in pixels.
const BUTTON_INSET: f32 = 2.0;
/// The corner radius of a button, in pixels.
const BUTTON_RADIUS: f32 = 3.0;

/// The action of a placeholder button.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RemoteAction {
    /// "Load": fetch the address once, or follow its blocked redirect once.
    Load,
    /// "Always load from" the host: allow the host, then load.
    AlwaysLoad,
    /// "Retry": forget the failure and load again.
    Retry,
}

/// How the piece paints: the colours and the font of the surface.
#[derive(Clone, Debug)]
pub(crate) struct PlaceholderStyle {
    /// The text row, muted as the note of a vault image.
    pub note: Rgba,
    /// The label of a button.
    pub button: Rgba,
    /// The box behind a button: the empty box of a vault image.
    pub fill: Rgba,
    pub font: Font,
    pub font_size: Pixels,
}

impl PlaceholderStyle {
    /// The roles of the vault image: its note for the text and its empty
    /// box behind each button. A label is in the text colour, so it reads
    /// as an action and not as disabled.
    pub(crate) fn of(theme: &Theme, font: Font, font_size: Pixels) -> Self {
        Self {
            note: theme::image_note(theme),
            button: theme::foreground(theme),
            fill: theme::image_placeholder(theme),
            font,
            font_size,
        }
    }
}

/// The text row of `look`.
fn text_of(look: &RemoteLook) -> String {
    match look {
        RemoteLook::Placeholder { host, .. } => format!("Remote image from {host}"),
        RemoteLook::RedirectBlocked { host, .. } => format!("Remote image redirects to {host}"),
        RemoteLook::Loading { host } => format!("Loading image from {host}"),
        RemoteLook::Insecure => "Not loaded: http is not secure".to_string(),
        RemoteLook::Failed { note, .. } => note.clone(),
    }
}

/// The buttons `look` offers, left to right: each label and its action.
fn buttons_of(look: &RemoteLook) -> Vec<(String, RemoteAction)> {
    let mut buttons = Vec::new();
    match look {
        RemoteLook::Placeholder { host, actions }
        | RemoteLook::RedirectBlocked { host, actions } => {
            if actions.load {
                buttons.push(("Load".to_string(), RemoteAction::Load));
            }
            if actions.always {
                buttons.push((format!("Always load from {host}"), RemoteAction::AlwaysLoad));
            }
        }
        RemoteLook::Failed { retry: true, .. } => {
            buttons.push(("Retry".to_string(), RemoteAction::Retry));
        }
        RemoteLook::Failed { retry: false, .. }
        | RemoteLook::Loading { .. }
        | RemoteLook::Insecure => {}
    }
    buttons
}

fn has_area(bounds: &Bounds<Pixels>) -> bool {
    bounds.size.width > px(0.) && bounds.size.height > px(0.)
}

/// Paints `look` over `rect`, the [`remote_rows`] rows of the embed,
/// clipped to `clip`. The first row holds the host or the note in muted
/// text. The second row holds the buttons the look offers. It returns the
/// bounds of each button inside `clip`, for [`action_at`]. Call it in the
/// paint phase.
pub(crate) fn paint_remote_look(
    window: &mut Window,
    cx: &mut App,
    rect: Bounds<Pixels>,
    clip: Bounds<Pixels>,
    look: &RemoteLook,
    style: &PlaceholderStyle,
) -> Vec<(Bounds<Pixels>, RemoteAction)> {
    let visible = rect.intersect(&clip);
    if !has_area(&visible) {
        return Vec::new();
    }
    let row = rect.size.height / remote_rows() as f32;
    let pad = style.font_size * PAD_EMS;
    let inset = px(BUTTON_INSET).min(row / 4.);
    let shape = |window: &mut Window, text: String, color: Rgba| {
        let run = TextRun {
            len: text.len(),
            font: style.font.clone(),
            color: color.into(),
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        window
            .text_system()
            .shape_line(SharedString::from(text), style.font_size, &[run], None)
    };
    let mut hits = Vec::new();
    window.with_content_mask(Some(ContentMask { bounds: visible }), |window| {
        let note = shape(window, text_of(look), style.note);
        let _ = note.paint(rect.origin, row, TextAlign::Left, None, window, cx);
        let top = rect.origin.y + row;
        let mut x = rect.origin.x;
        for (label, action) in buttons_of(look) {
            let line = shape(window, label, style.button);
            let button = Bounds::new(
                point(x, top + inset),
                size(line.width + pad * 2., row - inset * 2.),
            );
            window.paint_quad(fill(button, style.fill).corner_radii(px(BUTTON_RADIUS)));
            let _ = line.paint(point(x + pad, top), row, TextAlign::Left, None, window, cx);
            let hit = button.intersect(&visible);
            if has_area(&hit) {
                hits.push((hit, action));
            }
            x = button.right() + pad;
        }
    });
    hits
}

/// The button under `point`, from the hits [`paint_remote_look`] returned.
pub(crate) fn action_at(
    hits: &[(Bounds<Pixels>, RemoteAction)],
    point: Point<Pixels>,
) -> Option<RemoteAction> {
    hits.iter()
        .find(|(bounds, _)| bounds.contains(&point))
        .map(|(_, action)| *action)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::image_core::{Actions, RemoteLook};
    use gpui_kit::{Styled as _, TestAppContext, canvas, font, point, px, size};
    use std::cell::RefCell;
    use std::rc::Rc;

    fn style() -> PlaceholderStyle {
        PlaceholderStyle {
            note: gpui_kit::rgba(0x808080ff),
            button: gpui_kit::rgba(0x000000ff),
            fill: gpui_kit::rgba(0xeeeeeeff),
            font: font("Zed Mono"),
            font_size: px(14.),
        }
    }

    fn both() -> Actions {
        Actions {
            load: true,
            always: true,
        }
    }

    /// The hits of `look` painted over `rect` and clipped to `clip`, in a
    /// real paint pass.
    fn hits_of(
        cx: &mut TestAppContext,
        look: &RemoteLook,
        rect: Bounds<Pixels>,
        clip: Bounds<Pixels>,
    ) -> Vec<(Bounds<Pixels>, RemoteAction)> {
        let out = Rc::new(RefCell::new(Vec::new()));
        let (o, look) = (out.clone(), look.clone());
        cx.add_empty_window().draw(
            point(px(0.), px(0.)),
            size(px(800.), px(200.)),
            move |_, _| {
                canvas(
                    |_, _, _| {},
                    move |_, _, window, cx| {
                        *o.borrow_mut() =
                            paint_remote_look(window, cx, rect, clip, &look, &style());
                    },
                )
                .size_full()
            },
        );
        out.take()
    }

    /// The two rows of a look: 600 by 40 at (10, 20).
    const RECT_AT: (f32, f32, f32, f32) = (10., 20., 600., 40.);

    fn rect() -> Bounds<Pixels> {
        let (x, y, w, h) = RECT_AT;
        Bounds::new(point(px(x), px(y)), size(px(w), px(h)))
    }

    #[gpui_kit::test]
    fn paint_remote_look_returns_one_hit_per_offered_button_and_none_for_insecure(
        cx: &mut TestAppContext,
    ) {
        let host = "img.example.com".to_string();
        let cases = [
            (
                RemoteLook::Placeholder {
                    host: host.clone(),
                    actions: both(),
                },
                vec![RemoteAction::Load, RemoteAction::AlwaysLoad],
            ),
            (
                RemoteLook::Placeholder {
                    host: host.clone(),
                    actions: Actions {
                        load: true,
                        always: false,
                    },
                },
                vec![RemoteAction::Load],
            ),
            (
                RemoteLook::RedirectBlocked {
                    host: "tracker.example.net".into(),
                    actions: both(),
                },
                vec![RemoteAction::Load, RemoteAction::AlwaysLoad],
            ),
            (
                RemoteLook::Failed {
                    note: "Load failed: HTTP 404".into(),
                    retry: true,
                },
                vec![RemoteAction::Retry],
            ),
            (
                RemoteLook::Failed {
                    note: "Load failed: not a web address".into(),
                    retry: false,
                },
                vec![],
            ),
            (RemoteLook::Loading { host: host.clone() }, vec![]),
            (RemoteLook::Insecure, vec![]),
        ];
        for (look, want) in cases {
            let hits = hits_of(cx, &look, rect(), rect());
            let actions: Vec<RemoteAction> = hits.iter().map(|(_, a)| *a).collect();
            assert_eq!(actions, want, "{look:?}");
            // The buttons sit in the second row, inside the rect, left to
            // right, apart.
            for (b, _) in &hits {
                assert!(b.size.width > px(0.) && b.size.height > px(0.), "{look:?}");
                assert!(
                    b.origin.y >= rect().origin.y + rect().size.height / 2.,
                    "{look:?}"
                );
                assert!(b.bottom() <= rect().bottom() && b.right() <= rect().right());
                assert!(b.origin.x >= rect().origin.x);
            }
            for pair in hits.windows(2) {
                assert!(pair[0].0.right() < pair[1].0.origin.x, "{look:?}");
            }
        }
    }

    #[gpui_kit::test]
    fn a_slice_paints_only_the_buttons_in_its_clip(cx: &mut TestAppContext) {
        let look = RemoteLook::Placeholder {
            host: "img.example.com".into(),
            actions: both(),
        };
        let row = rect().size.height / 2.;
        let first = Bounds::new(rect().origin, size(rect().size.width, row));
        let second = Bounds::new(
            point(rect().origin.x, rect().origin.y + row),
            size(rect().size.width, row),
        );
        assert!(hits_of(cx, &look, rect(), first).is_empty());
        let full = hits_of(cx, &look, rect(), rect());
        assert_eq!(hits_of(cx, &look, rect(), second), full);
    }

    #[test]
    fn action_at_finds_the_button_under_a_point_and_none_outside() {
        let load = Bounds::new(point(px(10.), px(40.)), size(px(50.), px(18.)));
        let always = Bounds::new(point(px(70.), px(40.)), size(px(150.), px(18.)));
        let hits = vec![
            (load, RemoteAction::Load),
            (always, RemoteAction::AlwaysLoad),
        ];
        assert_eq!(
            action_at(&hits, point(px(20.), px(45.))),
            Some(RemoteAction::Load)
        );
        assert_eq!(
            action_at(&hits, point(px(100.), px(50.))),
            Some(RemoteAction::AlwaysLoad)
        );
        // Between the buttons, above them, and past the last one.
        assert_eq!(action_at(&hits, point(px(65.), px(45.))), None);
        assert_eq!(action_at(&hits, point(px(20.), px(30.))), None);
        assert_eq!(action_at(&hits, point(px(300.), px(45.))), None);
        assert_eq!(action_at(&[], point(px(20.), px(45.))), None);
    }

    #[test]
    fn the_text_of_each_look() {
        let host = "img.example.com".to_string();
        assert_eq!(
            text_of(&RemoteLook::Placeholder {
                host: host.clone(),
                actions: both()
            }),
            "Remote image from img.example.com"
        );
        assert_eq!(
            text_of(&RemoteLook::RedirectBlocked {
                host: "t.example.net".into(),
                actions: both()
            }),
            "Remote image redirects to t.example.net"
        );
        assert_eq!(
            text_of(&RemoteLook::Loading { host }),
            "Loading image from img.example.com"
        );
        assert_eq!(
            text_of(&RemoteLook::Insecure),
            "Not loaded: http is not secure"
        );
        assert_eq!(
            text_of(&RemoteLook::Failed {
                note: "Load failed: timed out".into(),
                retry: true
            }),
            "Load failed: timed out"
        );
    }

    #[test]
    fn the_style_takes_the_roles_of_the_vault_image() {
        for t in Theme::builtins() {
            let style = PlaceholderStyle::of(&t, font("Zed Mono"), px(14.));
            assert_eq!(style.note, theme::image_note(&t), "{}", t.name);
            assert_eq!(style.fill, theme::image_placeholder(&t), "{}", t.name);
            assert_eq!(style.button, theme::foreground(&t), "{}", t.name);
        }
    }

    #[test]
    fn the_labels_of_the_buttons() {
        let look = RemoteLook::Placeholder {
            host: "img.example.com".into(),
            actions: both(),
        };
        assert_eq!(
            buttons_of(&look),
            vec![
                ("Load".to_string(), RemoteAction::Load),
                (
                    "Always load from img.example.com".to_string(),
                    RemoteAction::AlwaysLoad
                ),
            ]
        );
        let failed = RemoteLook::Failed {
            note: "x".into(),
            retry: true,
        };
        assert_eq!(
            buttons_of(&failed),
            vec![("Retry".to_string(), RemoteAction::Retry)]
        );
    }
}
