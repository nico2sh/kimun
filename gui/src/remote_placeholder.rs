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

use gpui_kit::{
    App, Bounds, ContentMask, CursorStyle, Font, Hitbox, HitboxBehavior, Pixels, Point, Rgba,
    ShapedLine, SharedString, TextAlign, TextRun, Window, WindowTextSystem, fill, point, px, size,
};
use kimun_notes::settings::themes::Theme;

use crate::image_core::{RemoteLook, remote_rows};
use crate::remote_image::RemoteImages;
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
        RemoteLook::Failed { note, .. } => one_line(note),
    }
}

/// `text` with each control character (a newline, a carriage return, a
/// tab, and the others) as one space. A shaped line must hold no newline,
/// and a failure note can come from an error text of several lines.
fn one_line(text: &str) -> String {
    text.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect()
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

/// `text` on one line, shaped in the font of `style`.
fn shape(
    text_system: &WindowTextSystem,
    style: &PlaceholderStyle,
    text: String,
    color: Rgba,
) -> ShapedLine {
    let text = one_line(&text);
    let run = TextRun {
        len: text.len(),
        font: style.font.clone(),
        color: color.into(),
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    text_system.shape_line(SharedString::from(text), style.font_size, &[run], None)
}

/// A button of a look as laid out: its label, its whole box, the part of
/// the box inside the clip (where a press and the hand pointer act), and
/// its action.
struct Button {
    label: ShapedLine,
    bounds: Bounds<Pixels>,
    hit: Option<Bounds<Pixels>>,
    action: RemoteAction,
}

/// The buttons of `look` laid out over `rect`, the [`remote_rows`] rows of
/// the embed, clipped to `clip`: left to right in the second row. The one
/// layout that paint, the press test and the hand pointer all use.
fn lay_out_buttons(
    text_system: &WindowTextSystem,
    rect: Bounds<Pixels>,
    clip: Bounds<Pixels>,
    look: &RemoteLook,
    style: &PlaceholderStyle,
) -> Vec<Button> {
    let visible = rect.intersect(&clip);
    if !has_area(&visible) {
        return Vec::new();
    }
    let row = rect.size.height / remote_rows() as f32;
    let pad = style.font_size * PAD_EMS;
    let inset = px(BUTTON_INSET).min(row / 4.);
    let top = rect.origin.y + row;
    let mut x = rect.origin.x;
    let mut buttons = Vec::new();
    for (label, action) in buttons_of(look) {
        let label = shape(text_system, style, label, style.button);
        let bounds = Bounds::new(
            point(x, top + inset),
            size(label.width + pad * 2., row - inset * 2.),
        );
        let hit = Some(bounds.intersect(&visible)).filter(has_area);
        x = bounds.right() + pad;
        buttons.push(Button {
            label,
            bounds,
            hit,
            action,
        });
    }
    buttons
}

/// The bounds of each button of `look` inside `clip`, as
/// [`paint_remote_look`] returns them. The prepaint phase can call it,
/// before the look is painted.
pub(crate) fn button_areas(
    text_system: &WindowTextSystem,
    rect: Bounds<Pixels>,
    clip: Bounds<Pixels>,
    look: &RemoteLook,
    style: &PlaceholderStyle,
) -> Vec<(Bounds<Pixels>, RemoteAction)> {
    lay_out_buttons(text_system, rect, clip, look, style)
        .into_iter()
        .filter_map(|b| b.hit.map(|hit| (hit, b.action)))
        .collect()
}

/// A hitbox over each of `areas`, the button areas of [`button_areas`].
/// Call it in the prepaint phase, then [`show_hand_over`] in the paint
/// phase, so a button shows the pointing hand, as a link does.
pub(crate) fn insert_button_hitboxes(window: &mut Window, areas: &[Bounds<Pixels>]) -> Vec<Hitbox> {
    areas
        .iter()
        .map(|area| window.insert_hitbox(*area, HitboxBehavior::Normal))
        .collect()
}

/// The pointing hand over each button hitbox of
/// [`insert_button_hitboxes`]. Call it in the paint phase, after the
/// surface sets its own pointer: a later request wins where two overlap.
pub(crate) fn show_hand_over(window: &mut Window, hitboxes: &[Hitbox]) {
    for hitbox in hitboxes {
        window.set_cursor_style(CursorStyle::PointingHand, hitbox);
    }
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
    let text_system = window.text_system().clone();
    let buttons = lay_out_buttons(&text_system, rect, clip, look, style);
    window.with_content_mask(Some(ContentMask { bounds: visible }), |window| {
        let note = shape(&text_system, style, text_of(look), style.note);
        let _ = note.paint(rect.origin, row, TextAlign::Left, None, window, cx);
        let top = rect.origin.y + row;
        for button in &buttons {
            window.paint_quad(fill(button.bounds, style.fill).corner_radii(px(BUTTON_RADIUS)));
            let at = point(button.bounds.origin.x + pad, top);
            let _ = button
                .label
                .paint(at, row, TextAlign::Left, None, window, cx);
        }
    });
    buttons
        .into_iter()
        .filter_map(|b| b.hit.map(|hit| (hit, b.action)))
        .collect()
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

/// Runs the button `action` of the placeholder of the remote embed `url`
/// in the shared store. Every surface that shows a placeholder calls it.
///
/// - "Load" lets a blocked redirect follow once, then loads.
/// - "Retry" forgets the failure, then loads.
/// - "Always load from" loads nothing here. It returns `host`, and the
///   surface asks its owner to allow the host first and to load after. With
///   no host it loads.
///
/// `on_done` runs on the UI thread when the fetch ends.
pub(crate) fn run_action(
    url: &str,
    host: Option<String>,
    action: RemoteAction,
    cx: &mut App,
    on_done: impl FnOnce(&mut App) + 'static,
) -> Option<String> {
    let mut store = cx.default_global::<RemoteImages>().clone();
    match action {
        // On a blocked redirect, "Load" follows it once. Elsewhere the
        // permission call does nothing.
        RemoteAction::Load => store.allow_redirect_once(url),
        RemoteAction::Retry => store.retry(url),
        RemoteAction::AlwaysLoad => {
            if host.is_some() {
                return host;
            }
        }
    }
    store.load(url, cx, on_done);
    None
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

    /// The button areas, the hitboxes of the hand pointer (both from the
    /// prepaint phase) and the press hits (from the paint phase) of `look`
    /// over `rect`, clipped to `clip`, in one real frame.
    #[allow(clippy::type_complexity)]
    fn areas_hands_and_hits(
        cx: &mut TestAppContext,
        look: &RemoteLook,
        rect: Bounds<Pixels>,
        clip: Bounds<Pixels>,
    ) -> (
        Vec<(Bounds<Pixels>, RemoteAction)>,
        Vec<Bounds<Pixels>>,
        Vec<(Bounds<Pixels>, RemoteAction)>,
    ) {
        let out = Rc::new(RefCell::new((Vec::new(), Vec::new(), Vec::new())));
        let (o, look) = (out.clone(), look.clone());
        cx.add_empty_window().draw(
            point(px(0.), px(0.)),
            size(px(800.), px(200.)),
            move |_, _| {
                let (prepaint_look, o2) = (look.clone(), o.clone());
                canvas(
                    move |_, window, _| {
                        let areas = button_areas(
                            window.text_system(),
                            rect,
                            clip,
                            &prepaint_look,
                            &style(),
                        );
                        let bounds: Vec<_> = areas.iter().map(|(b, _)| *b).collect();
                        let hitboxes = insert_button_hitboxes(window, &bounds);
                        o2.borrow_mut().0 = areas;
                        o2.borrow_mut().1 = hitboxes.iter().map(|h| h.bounds).collect();
                        hitboxes
                    },
                    move |_, hitboxes, window, cx| {
                        show_hand_over(window, &hitboxes);
                        o.borrow_mut().2 =
                            paint_remote_look(window, cx, rect, clip, &look, &style());
                    },
                )
                .size_full()
            },
        );
        out.take()
    }

    /// The hand pointer shows where a press acts: over each button, in the
    /// editor, the facet view and the preview alike. (The test window
    /// cannot read the pointer it shows: the areas are what is checked.)
    #[gpui_kit::test]
    fn the_hand_pointer_areas_are_the_press_areas(cx: &mut TestAppContext) {
        let looks = [
            RemoteLook::Placeholder {
                host: "img.example.com".into(),
                actions: both(),
            },
            RemoteLook::RedirectBlocked {
                host: "t.example.net".into(),
                actions: both(),
            },
            RemoteLook::Failed {
                note: "Load failed: HTTP 404".into(),
                retry: true,
            },
            RemoteLook::Loading {
                host: "img.example.com".into(),
            },
        ];
        let row = rect().size.height / 2.;
        let second = Bounds::new(
            point(rect().origin.x, rect().origin.y + row),
            size(rect().size.width, row),
        );
        // Half a button row: the hand stays inside the clip.
        let half = Bounds::new(
            point(rect().origin.x, rect().origin.y + row),
            size(px(30.), row / 2.),
        );
        for look in &looks {
            for clip in [rect(), second, half] {
                let (areas, hands, hits) = areas_hands_and_hits(cx, look, rect(), clip);
                assert_eq!(areas, hits, "{look:?} {clip:?}");
                let press: Vec<_> = hits.iter().map(|(b, _)| *b).collect();
                assert_eq!(hands, press, "{look:?} {clip:?}");
            }
        }
        let (_, hands, _) = areas_hands_and_hits(cx, &looks[0], rect(), rect());
        assert_eq!(hands.len(), 2, "Load and Always load from");
        let (_, hands, _) = areas_hands_and_hits(cx, &looks[2], rect(), rect());
        assert_eq!(hands.len(), 1, "Retry");
        let (_, hands, _) = areas_hands_and_hits(cx, &looks[3], rect(), rect());
        assert!(hands.is_empty(), "no button, no hand");
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

    #[gpui_kit::test]
    fn a_failure_note_with_control_characters_paints_on_one_line(cx: &mut TestAppContext) {
        for note in [
            "Load failed: first line\nsecond line\nthird",
            "Load failed: a\r\nb\tc\u{7}d",
        ] {
            let look = RemoteLook::Failed {
                note: note.into(),
                retry: true,
            };
            let text = text_of(&look);
            assert!(!text.chars().any(char::is_control), "{text:?}");
            assert_eq!(text.chars().count(), note.chars().count());
            let hits = hits_of(cx, &look, rect(), rect());
            assert_eq!(
                hits.iter().map(|(_, a)| *a).collect::<Vec<_>>(),
                [RemoteAction::Retry]
            );
        }
    }

    #[test]
    fn action_at_takes_the_left_edge_of_a_button_and_not_its_right_edge() {
        let load = Bounds::new(point(px(10.), px(40.)), size(px(50.), px(18.)));
        let hits = vec![(load, RemoteAction::Load)];
        assert_eq!(
            action_at(&hits, point(px(10.), px(45.))),
            Some(RemoteAction::Load)
        );
        assert_eq!(action_at(&hits, point(px(60.), px(45.))), None);
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

    /// Every GUI source file but this one, with its text.
    fn other_sources() -> Vec<(std::path::PathBuf, String)> {
        let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut found = Vec::new();
        let mut dirs = vec![src];
        while let Some(dir) = dirs.pop() {
            for entry in std::fs::read_dir(dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    dirs.push(path);
                } else if path.extension().is_some_and(|e| e == "rs")
                    && !path.ends_with("src/remote_placeholder.rs")
                {
                    let text = std::fs::read_to_string(&path).unwrap();
                    found.push((path, text));
                }
            }
        }
        found
    }

    /// The editor, the facet view (its chunks are read-only editors) and
    /// the preview paint a remote look only through this piece, and find a
    /// press only with `action_at`. No other file paints a placeholder text
    /// or button, or has a paint or hit test of its own.
    #[test]
    fn the_editor_and_the_read_only_views_use_the_one_placeholder_piece() {
        // Built from parts, so this file does not match itself.
        let needles = [
            ["\"Remote image from", " {"].concat(),
            ["\"Remote image redirects to", " {"].concat(),
            ["\"Loading image from", " {"].concat(),
            ["\"Always load from", " {"].concat(),
            ["\"Not loaded: http", " is not secure"].concat(),
            ["fn paint_remote", "_look"].concat(),
            ["fn action", "_at("].concat(),
            ["fn buttons", "_of("].concat(),
        ];
        let sources = other_sources();
        let mut found = Vec::new();
        for (path, text) in &sources {
            for needle in &needles {
                if text.contains(needle.as_str()) {
                    found.push(format!("{}: {needle}", path.display()));
                }
            }
        }
        assert!(found.is_empty(), "{found:#?}");
        // Each surface paints and hit-tests through the piece.
        let calls = |file: &str, call: &str| {
            sources
                .iter()
                .any(|(path, text)| path.ends_with(file) && text.contains(call))
        };
        let paint = ["paint_remote", "_look("].concat();
        let hit = ["action", "_at("].concat();
        assert!(calls("src/editor_element.rs", &paint), "the editor element");
        assert!(calls("src/workspace/search.rs", &paint), "the preview");
        assert!(calls("src/editor_view/remote.rs", &hit), "the editor press");
        assert!(calls("src/workspace/search.rs", &hit), "the preview press");
    }
}
