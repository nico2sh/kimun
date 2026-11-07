//! `ThemeColor` -> GPUI. `Rgb` maps directly; `Ansi` and `Reset` are
//! terminal-relative and have no terminal here, so they get a fixed meaning:
//! the xterm 16-colour palette, the standard 6x6x6 cube and 24-step grey ramp
//! for 16-255, and, for `Reset`, the colour the *token* implies.
//!
//! Two things a naive mapping gets wrong, both handled here:
//! - **Light themes.** A `Reset` background/foreground falls back to a light or
//!   dark pair depending on the theme's *other* explicit colour, never to a
//!   fixed dark pair.
//! - **Selection.** A `Reset` selection background must not collapse into the
//!   background (an invisible selection); it becomes a tint of the foreground
//!   over the background. An explicit selection equal to the background gets
//!   the same treatment.

use gpui_kit::Rgba;
use kimun_notes::logic::editor::markdown::runs::{Role, Run};
use kimun_notes::logic::editor::markdown::{ElementKind, MOD_BOLD, MOD_ITALIC, MOD_STRIKE};
use kimun_notes::logic::editor::overlay::OverlayKind;
use kimun_notes::settings::themes::{Theme, ThemeColor};

/// What a `ThemeColor::Reset` resolves against.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Token {
    Background,
    Foreground,
    SelectionBackground,
}

type Rgb8 = (u8, u8, u8);

const ANSI16: [Rgb8; 16] = [
    (0, 0, 0),
    (205, 0, 0),
    (0, 205, 0),
    (205, 205, 0),
    (0, 0, 238),
    (205, 0, 205),
    (0, 205, 205),
    (229, 229, 229),
    (127, 127, 127),
    (255, 0, 0),
    (0, 255, 0),
    (255, 255, 0),
    (92, 92, 255),
    (255, 0, 255),
    (0, 255, 255),
    (255, 255, 255),
];

const DARK_BG: Rgb8 = (30, 30, 30);
const DARK_FG: Rgb8 = (220, 220, 220);
const LIGHT_BG: Rgb8 = (250, 250, 250);
const LIGHT_FG: Rgb8 = (30, 30, 30);

/// How far the foreground is mixed into the background for a derived selection.
const SELECTION_MIX: f32 = 0.22;

fn ansi(n: u8) -> Rgb8 {
    match n {
        0..=15 => ANSI16[n as usize],
        16..=231 => {
            let i = n - 16;
            let step = |v: u8| if v == 0 { 0 } else { 55 + 40 * v };
            (step(i / 36), step((i / 6) % 6), step(i % 6))
        }
        232..=255 => {
            let v = 8 + 10 * (n - 232);
            (v, v, v)
        }
    }
}

/// `None` for `Reset`.
fn explicit(c: &ThemeColor) -> Option<Rgb8> {
    match c {
        ThemeColor::Rgb(r, g, b) => Some((*r, *g, *b)),
        ThemeColor::Ansi(n) => Some(ansi(*n)),
        ThemeColor::Reset => None,
    }
}

/// Perceived brightness, 0.0 (black) ..= 1.0 (white).
fn luma((r, g, b): Rgb8) -> f32 {
    (0.299 * f32::from(r) + 0.587 * f32::from(g) + 0.114 * f32::from(b)) / 255.0
}

fn mix(a: Rgb8, b: Rgb8, t: f32) -> Rgb8 {
    let m = |x: u8, y: u8| (f32::from(x) + (f32::from(y) - f32::from(x)) * t).round() as u8;
    (m(a.0, b.0), m(a.1, b.1), m(a.2, b.2))
}

/// The theme's plain background and text, with `Reset` filled in from whichever
/// of the two is explicit so a light theme stays light.
fn base(theme: &Theme) -> (Rgb8, Rgb8) {
    match (explicit(&theme.bg), explicit(&theme.fg)) {
        (Some(bg), Some(fg)) => (bg, fg),
        (Some(bg), None) => (bg, if luma(bg) > 0.5 { LIGHT_FG } else { DARK_FG }),
        (None, Some(fg)) => (if luma(fg) > 0.5 { DARK_BG } else { LIGHT_BG }, fg),
        (None, None) => (DARK_BG, DARK_FG),
    }
}

fn resolve(color: &ThemeColor, token: Token, theme: &Theme) -> Rgb8 {
    let (bg, fg) = base(theme);
    let derived_selection = mix(bg, fg, SELECTION_MIX);
    match (explicit(color), token) {
        // An explicit selection identical to the page would be invisible.
        (Some(c), Token::SelectionBackground) if c == bg => derived_selection,
        (Some(c), _) => c,
        // `Reset` means "the terminal's default". The nearest honest equivalent
        // is the theme's own plain background / text colour, never transparent.
        (None, Token::Background) => bg,
        (None, Token::Foreground) => fg,
        (None, Token::SelectionBackground) => derived_selection,
    }
}

fn rgba((r, g, b): Rgb8) -> Rgba {
    Rgba {
        r: f32::from(r) / 255.0,
        g: f32::from(g) / 255.0,
        b: f32::from(b) / 255.0,
        a: 1.0,
    }
}

/// Resolve any theme colour to an opaque GPUI colour.
pub fn to_gpui(color: &ThemeColor, token: Token, theme: &Theme) -> Rgba {
    rgba(resolve(color, token, theme))
}

/// The window background.
pub fn background(theme: &Theme) -> Rgba {
    to_gpui(&theme.bg, Token::Background, theme)
}

/// Primary text.
pub fn foreground(theme: &Theme) -> Rgba {
    to_gpui(&theme.fg, Token::Foreground, theme)
}

/// Background of a selected row or selected text; always distinguishable from
/// [`background`].
pub fn selection_background(theme: &Theme) -> Rgba {
    to_gpui(&theme.selection_bg, Token::SelectionBackground, theme)
}

/// The caret. `Reset` (terminal default) falls back to the bright text colour,
/// as the TUI does for its preview-current highlight.
pub fn caret(theme: &Theme) -> Rgba {
    match theme.cursor {
        ThemeColor::Reset => to_gpui(&theme.fg_bright, Token::Foreground, theme),
        ref c => to_gpui(c, Token::Foreground, theme),
    }
}

/// The vim mode label in the mode line: the colour the TUI footer gives the
/// modal-mode label.
pub fn mode_label(theme: &Theme) -> Rgba {
    to_gpui(&theme.accent, Token::Foreground, theme)
}

/// Secondary text (the mode line's pending-input hint): the TUI footer's hint
/// colour.
pub fn secondary(theme: &Theme) -> Rgba {
    to_gpui(&theme.fg_secondary, Token::Foreground, theme)
}

/// Quiet information (the find bar's match count and key hints): the TUI
/// find bar's `gray`.
pub fn muted(theme: &Theme) -> Rgba {
    to_gpui(&theme.gray, Token::Foreground, theme)
}

/// A problem the user has to act on (no match, an invalid regex, the armed
/// replace-all confirm): the TUI find bar's `red`.
pub fn error(theme: &Theme) -> Rgba {
    to_gpui(&theme.red, Token::Foreground, theme)
}

/// A check that passed (a free name in the rename and move dialogs): the
/// TUI dialogs' `green`.
pub fn available(theme: &Theme) -> Rgba {
    to_gpui(&theme.green, Token::Foreground, theme)
}

/// A note with unsaved edits (the Footer's save-state dot): the TUI
/// footer's `● modified` colour.
pub fn unsaved(theme: &Theme) -> Rgba {
    to_gpui(&theme.yellow, Token::Foreground, theme)
}

/// A step that replaces something kept (the save-search dialog's
/// "Overwrites …" hint): the TUI dialog's yellow warning.
pub fn caution(theme: &Theme) -> Rgba {
    to_gpui(&theme.yellow, Token::Foreground, theme)
}

/// The hairline that separates docked chrome (the find bar) from the text.
pub fn divider(theme: &Theme) -> Rgba {
    to_gpui(&theme.border_dim, Token::Foreground, theme)
}

// ---- The shell (rail, drawer, focus) ----------------------------------------

/// How far the foreground is mixed into the background for a derived hover.
const HOVER_MIX: f32 = 0.08;

/// The rail and drawer surface: the TUI's panel background (the page when
/// the theme leaves it to the terminal).
pub fn panel_background(theme: &Theme) -> Rgba {
    to_gpui(&theme.bg_panel, Token::Background, theme)
}

/// The focused panel's edge and the active rail view's marker: the TUI's
/// focused-border colour.
pub fn focus_border(theme: &Theme) -> Rgba {
    to_gpui(&theme.focus_border, Token::Foreground, theme)
}

/// Bright text: the rail cell under the keyboard cursor.
pub fn bright(theme: &Theme) -> Rgba {
    to_gpui(&theme.fg_bright, Token::Foreground, theme)
}

/// A pointer or cursor sitting on a control: the theme's soft background,
/// or a tint of the text over the page when that would not show (left to
/// the terminal, or equal to the panel it sits on).
pub fn hover_background(theme: &Theme) -> Rgba {
    let panel = resolve(&theme.bg_panel, Token::Background, theme);
    match explicit(&theme.bg_soft) {
        Some(soft) if soft != panel => rgba(soft),
        _ => {
            let (_, fg) = base(theme);
            rgba(mix(panel, fg, HOVER_MIX))
        }
    }
}

/// How far the accent is mixed into the panel for a selected item.
const SELECTED_MIX: f32 = 0.2;

/// A selected item's fill (the rail's shown view): the panel tinted toward
/// the focus colour, so it reads as chosen, not as hovered.
pub fn selected_background(theme: &Theme) -> Rgba {
    let panel = resolve(&theme.bg_panel, Token::Background, theme);
    let accent = resolve(&theme.focus_border, Token::Foreground, theme);
    rgba(mix(panel, accent, SELECTED_MIX))
}

// ---- The which-key popup -------------------------------------------------------

/// A key in the which-key popup (the pressed keys, each row's key): the
/// TUI's which-key keycap colour.
pub fn key_cap(theme: &Theme) -> Rgba {
    to_gpui(&theme.yellow, Token::Foreground, theme)
}

/// A row of the which-key popup that opens a group rather than running a
/// command: the TUI's which-key group colour.
pub fn key_group(theme: &Theme) -> Rgba {
    to_gpui(&theme.aqua, Token::Foreground, theme)
}

// ---- The kit's own theme ------------------------------------------------------

/// Give the kit's components (the command palette's dialog, its filter
/// input, its rows) kimün's colours instead of the kit's stock white. Only
/// the roles the GUI's kit-drawn surfaces read are set; the rest keep the
/// kit's values.
pub fn apply_to_kit(theme: &Theme, kit: &mut gpui_kit::component::Theme) {
    let surface = panel_background(theme);
    let text = foreground(theme);
    let hover = hover_background(theme);
    let focus = focus_border(theme);
    // One tone for the dialog frame and its list (`popover`), so no seam shows
    // where the frame is taller than the rows: the panel surface, as the drawers.
    kit.background = surface.into();
    kit.foreground = text.into();
    kit.popover = surface.into();
    kit.popover_foreground = text.into();
    kit.border = divider(theme).into();
    kit.input = divider(theme).into();
    kit.muted = hover.into();
    kit.muted_foreground = muted(theme).into();
    kit.accent = hover.into();
    kit.accent_foreground = text.into();
    kit.secondary = hover.into();
    kit.secondary_foreground = text.into();
    kit.primary = focus.into();
    kit.primary_foreground = background(theme).into();
    kit.ring = focus.into();
    // A focused kit input gets a ring colour border; the kit's extra tinted ring around
    // it reads as a doubled border. Keep the border only.
    kit.focus_ring = false;
    kit.selection = selection_background(theme).into();
    kit.caret = caret(theme).into();
    kit.list_even = surface.into();
    kit.list_hover = hover.into();
    kit.list_active = selected_background(theme).into();
    kit.list_active_border = focus.into();
    kit.secondary_hover = hover.into();
    kit.secondary_active = selected_background(theme).into();
    // Tabs (LINKS' segmented control): the bar sits on the panel, the segmented
    // track is the soft background, the active segment the selected tint.
    kit.tab_bar = surface.into();
    kit.tab_bar_segmented = hover.into();
    kit.tab_active = selected_background(theme).into();
    kit.tab_foreground = muted(theme).into();
    kit.tab_active_foreground = bright(theme).into();
    // The notification toasts (the flashes, N1): their icon takes the kind's
    // colour, the card the `popover` surface with the `border` hairline, the
    // message the `foreground`, its close button the ghost Button's roles.
    kit.danger = error(theme).into();
    kit.warning = caution(theme).into();
    kit.info = accent(theme).into();
    kit.success = available(theme).into();
    // The default Button (the attachment view's `Open externally`, the
    // dialogs' Cancel): its fill, label, hover and press; its border is the
    // `input` role above. Its fill is the panel surface, so on the editor's
    // page it reads as a control, in a dialog as the dialog's own tone.
    kit.button = surface.into();
    kit.button_foreground = text.into();
    kit.button_hover = hover.into();
    kit.button_active = selected_background(theme).into();
    // The ghost Button (the search browser's `?`) and its tooltip read only
    // roles mapped above: `secondary_foreground`, hover `accent` /
    // `accent_foreground`, press `button_active`; the tooltip `popover`,
    // `popover_foreground`, `border` and `muted_foreground`.
    // The kit Switch (the sort dialog's "Folders first"): checked, the
    // `primary` role above; off, its track; the thumb the page colour, so it
    // reads on both.
    kit.switch = muted(theme).into();
    kit.switch_thumb = background(theme).into();
    // The kit Select (the properties form's Type, the sort dialog's Sort by
    // and Order) reads only roles mapped
    // above: its trigger `input` / `background` / `foreground` / `ring`, its
    // menu `popover` / `popover_foreground`, its rows `accent`, `list_hover`,
    // `list_active`, `selection` and `muted_foreground`.
    // The primary Button (the file dialogs' commit, the setup assistant's
    // Next and Finish) and the danger Button (the assistant's Quit and
    // Discard): kimün's focus and error colours, the label on the page
    // colour, hover and press a step toward the page.
    let page = background(theme);
    let (hovered, pressed) = (0.15, 0.3);
    kit.button_primary = focus.into();
    kit.button_primary_foreground = page.into();
    kit.button_primary_hover = mix_rgba(focus, page, hovered).into();
    kit.button_primary_active = mix_rgba(focus, page, pressed).into();
    let danger = error(theme);
    kit.button_danger = danger.into();
    kit.button_danger_foreground = page.into();
    kit.button_danger_hover = mix_rgba(danger, page, hovered).into();
    kit.button_danger_active = mix_rgba(danger, page, pressed).into();
    // The kit Radio (the setup assistant's Editor backend and Updates
    // choices, Preferences' sort and backend choices) reads only roles mapped
    // above: checked `primary` with its mark `primary_foreground`, unchecked
    // `input` (border and fill), the label `foreground`, a disabled label
    // `muted_foreground`, and the focus `ring`.
    // The kit Checkbox (Preferences' update check) reads the same roles:
    // checked `primary` with its tick `primary_foreground`, unchecked the
    // `input` border on `input_background()` (derived from `input` and
    // `background`), the label `foreground`, a disabled label
    // `muted_foreground`, the focus `ring`.
    // The kit Spinner (Preferences' reindex progress panel, and every
    // Button's `loading`) is an Icon drawn in the text colour it is given,
    // else the kit `foreground`: mapped above.
}

// ---- FILES rows --------------------------------------------------------------

/// The open-note marker and the `Create:` row: the TUI's accent.
pub fn accent(theme: &Theme) -> Rgba {
    to_gpui(&theme.accent, Token::Foreground, theme)
}

/// A folder's glyph and name: the TUI's directory colour.
pub fn directory(theme: &Theme) -> Rgba {
    to_gpui(&theme.color_directory, Token::Foreground, theme)
}

/// A journal entry's date line: the TUI's journal-date colour.
pub fn journal_date(theme: &Theme) -> Rgba {
    to_gpui(&theme.color_journal_date, Token::Foreground, theme)
}

/// A query's match in the search browser's preview: the colour the editor
/// gives the same needles once the note opens (the TUI's search match).
pub fn search_match(theme: &Theme) -> Rgba {
    to_gpui(&theme.color_search_match, Token::Foreground, theme)
}

/// A tag's glyph and name in TAGS: the colour a `#tag` has in the editor.
pub fn tag(theme: &Theme) -> Rgba {
    to_gpui(&theme.color_tag, Token::Foreground, theme)
}

// ---- Edit surface: Role -> style --------------------------------------------

/// How one run of the **Editor frame** paints: the GPUI counterpart of the
/// TUI's ratatui `Style` (`tui/src/components/text_editor/markdown_style.rs`
/// `role_style` + `view.rs` `run_style`), whose semantics it mirrors.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CellStyle {
    pub fg: Rgba,
    /// `None` = the page background shows through.
    pub bg: Option<Rgba>,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strike: bool,
}

/// How far a dimmed (completed-task) foreground is pulled toward the page.
const DIM_MIX: f32 = 0.45;

pub(crate) fn fg_of(c: &ThemeColor, theme: &Theme) -> Rgba {
    to_gpui(c, Token::Foreground, theme)
}

/// A background colour; `Reset` means "no background of its own".
fn bg_of(c: &ThemeColor, theme: &Theme) -> Option<Rgba> {
    match c {
        ThemeColor::Reset => None,
        c => Some(to_gpui(c, Token::Background, theme)),
    }
}

/// The style a [`Role`] paints with (mirror of the TUI's `role_style`).
fn role_style(role: Role, theme: &Theme) -> CellStyle {
    let mut s = CellStyle {
        fg: foreground(theme),
        bg: None,
        bold: false,
        italic: false,
        underline: false,
        strike: false,
    };
    match role {
        Role::Plain | Role::CodeText | Role::CodeBoxPad => {}
        Role::Sigil | Role::Revealed | Role::Rule | Role::ElementSigil(_) => {
            s.fg = fg_of(&theme.gray, theme);
        }
        Role::BlockquoteBar => s.fg = fg_of(&theme.blockquote_bar, theme),
        Role::Element(kind) => match kind {
            // Weight only: the GUI has a real bold face, so no colour cue
            // (the TUI needs one; a terminal cell can't always embolden).
            ElementKind::Bold => s.bold = true,
            ElementKind::Italic => {
                s.fg = fg_of(&theme.fg_secondary, theme);
                s.italic = true;
            }
            ElementKind::Strikethrough => {
                s.fg = fg_of(&theme.fg_secondary, theme);
                s.strike = true;
            }
            ElementKind::InlineCode => {
                s.fg = fg_of(&theme.aqua, theme);
                s.bg = bg_of(&theme.bg_soft, theme);
            }
            ElementKind::Link => {
                s.fg = fg_of(&theme.accent, theme);
                s.underline = true;
            }
            ElementKind::Image => {
                s.fg = fg_of(&theme.accent, theme);
                s.italic = true;
            }
            ElementKind::HeadingH1 | ElementKind::HeadingH2 => {
                s.fg = fg_of(&theme.fg_bright, theme);
                s.bold = true;
            }
            ElementKind::HeadingH3 => {
                s.fg = fg_of(&theme.yellow, theme);
                s.bold = true;
            }
            ElementKind::Blockquote => s.fg = fg_of(&theme.fg_secondary, theme),
            ElementKind::WikiLink => {
                s.fg = fg_of(&theme.blue, theme);
                s.underline = true;
            }
            ElementKind::Label => {
                s.fg = fg_of(&theme.color_tag, theme);
                s.bold = true;
            }
        },
    }
    s
}

/// The style a [`Run`] paints with: role + emphasis mask, then the code-box
/// background, then each overlay in stacking order (mirror of the TUI's
/// `run_style` / `overlay_style`).
pub fn run_style(run: &Run, theme: &Theme) -> CellStyle {
    let mut s = role_style(run.role, theme);
    s.bold |= run.mods & MOD_BOLD != 0;
    s.italic |= run.mods & MOD_ITALIC != 0;
    s.strike |= run.mods & MOD_STRIKE != 0;
    if run.code_box {
        s.bg = bg_of(&theme.code_bg, theme);
    }
    let mut dim = false;
    for kind in run.overlays.iter() {
        match kind {
            OverlayKind::TaskBox => s.fg = fg_of(&theme.accent, theme),
            OverlayKind::TaskDone => {
                dim = true;
                s.strike = true;
            }
            OverlayKind::Needle | OverlayKind::Match => {
                s.fg = fg_of(&theme.color_search_match, theme);
                s.bold = true;
            }
            OverlayKind::Selection | OverlayKind::CurrentMatch => {
                s.bg = Some(selection_background(theme));
            }
            OverlayKind::Preview => s.bg = bg_of(&theme.color_replace_preview, theme),
            // Clickable cells, for the pointer (`RowPaint::targets`); no ink.
            OverlayKind::FollowTarget => {}
            OverlayKind::PreviewCurrent => {
                // The match about to be replaced: inverse video in the search
                // accent, so it stands apart from the other previewed matches
                // (which share the preview background) at a glance.
                let (bg, _) = base(theme);
                s.bg = Some(fg_of(&theme.color_search_match, theme));
                s.fg = rgba(bg);
                s.bold = true;
            }
        }
    }
    // A terminal's DIM applies to whatever foreground the cell ends with.
    if dim {
        let (bg, _) = base(theme);
        s.fg = mix_rgba(s.fg, rgba(bg), DIM_MIX);
    }
    s
}

fn mix_rgba(a: Rgba, b: Rgba, t: f32) -> Rgba {
    let m = |x: f32, y: f32| x + (y - x) * t;
    Rgba {
        r: m(a.r, b.r),
        g: m(a.g, b.g),
        b: m(a.b, b.b),
        a: 1.0,
    }
}

/// The theme the window is drawn in, shared by every view: they read it
/// when they render ([`current`]) instead of keeping a copy, so a theme
/// change is one [`set_current`] and no view can be left on the old one.
struct AppTheme(std::sync::Arc<Theme>);

impl gpui_kit::Global for AppTheme {}

/// Make `theme` the one every view draws in, and redraw the window. Cached
/// views follow through [`observe`]. Needs the kit initialised; the kit's
/// own component colours are the caller's ([`apply_to_kit`]).
pub(crate) fn set_current(cx: &mut gpui_kit::App, theme: Theme) {
    cx.set_global(AppTheme(std::sync::Arc::new(theme)));
}

/// The theme in effect. Cheap to take (a shared pointer); take it once per
/// render and borrow from it. The default theme before [`set_current`] has
/// run (a view built outside the app, in a test).
pub(crate) fn current(cx: &gpui_kit::App) -> std::sync::Arc<Theme> {
    cx.try_global::<AppTheme>()
        .map(|t| t.0.clone())
        .unwrap_or_default()
}

/// Redraw `view` whenever the theme changes. Only for a view the window
/// keeps cached (it is not drawn again when its parent is): every other
/// view reads [`current`] as it renders.
pub(crate) fn observe<T: 'static>(cx: &mut gpui_kit::Context<T>) -> gpui_kit::Subscription {
    cx.observe_global::<AppTheme>(|_, cx| cx.notify())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The dialog surface reads the kit's per-component tokens, not its
    /// colours: `Theme::update` must carry kimün's panel surface (the dialog's one tone) to both.
    #[gpui_kit::test]
    fn the_kit_dialog_surface_takes_the_panel_surface(cx: &mut gpui_kit::TestAppContext) {
        let t = theme(
            ThemeColor::Rgb(10, 20, 30),
            ThemeColor::Rgb(200, 200, 200),
            ThemeColor::Rgb(60, 60, 90),
        );
        let want: gpui_kit::Hsla = panel_background(&t).into();
        cx.update(|cx| {
            gpui_kit::init(cx);
            gpui_kit::component::Theme::update(cx, |kit| apply_to_kit(&t, kit));
            let kit = gpui_kit::component::Theme::global(cx);
            assert_eq!(kit.background, want);
            assert_eq!(kit.tokens.background.color, want);
            // The tab bar follows too (it reads its own colours, not the page's).
            assert_eq!(kit.tab_bar, want);
            assert_eq!(kit.tokens.tab_bar.color, want);
            assert_eq!(kit.tab_active_foreground, bright(&t).into());
            // The Select (the properties form's Type): trigger, menu, rows.
            let hsla = |c: Rgba| -> gpui_kit::Hsla { c.into() };
            assert_eq!(kit.input, hsla(divider(&t)));
            assert_eq!(kit.ring, hsla(focus_border(&t)));
            assert_eq!(kit.popover, want);
            assert_eq!(kit.popover_foreground, hsla(foreground(&t)));
            assert_eq!(kit.accent, hsla(hover_background(&t)));
            assert_eq!(kit.list_hover, hsla(hover_background(&t)));
            assert_eq!(kit.list_active, hsla(selected_background(&t)));
            assert_eq!(kit.selection, hsla(selection_background(&t)));
            assert_eq!(kit.muted_foreground, hsla(muted(&t)));
            // The default Button, through its tokens (what it paints with).
            assert_eq!(kit.tokens.button.color, want);
            assert_eq!(kit.button_foreground, hsla(foreground(&t)));
            assert_eq!(kit.tokens.button_hover.color, hsla(hover_background(&t)));
            assert_eq!(
                kit.tokens.button_active.color,
                hsla(selected_background(&t))
            );
            // The ghost Button and its tooltip (the search browser's `?`).
            assert_eq!(kit.secondary_foreground, hsla(foreground(&t)));
            assert_eq!(kit.tokens.accent.color, hsla(hover_background(&t)));
            assert_eq!(kit.accent_foreground, hsla(foreground(&t)));
            assert_eq!(kit.tokens.popover.color, want);
            assert_eq!(kit.border, hsla(divider(&t)));
            // The Switch (the sort dialog's "Folders first"), through its
            // tokens: on is `primary`, off the track, and the thumb.
            assert_eq!(kit.tokens.primary.color, hsla(focus_border(&t)));
            assert_eq!(kit.tokens.switch.color, hsla(muted(&t)));
            assert_eq!(kit.tokens.switch_thumb.color, hsla(background(&t)));
            // The notification toasts: the kinds' icons, the card's surface
            // and hairline (the card reads `tokens.popover`), the message.
            assert_eq!(kit.info, hsla(accent(&t)));
            assert_eq!(kit.success, hsla(available(&t)));
            assert_eq!(kit.warning, hsla(caution(&t)));
            assert_eq!(kit.danger, hsla(error(&t)));
            assert_eq!(kit.tokens.popover.color, want);
            assert_eq!(kit.border, hsla(divider(&t)));
            assert_eq!(kit.foreground, hsla(foreground(&t)));
            // The primary and danger Buttons (the setup assistant's Next /
            // Finish, Quit / Discard), through their tokens.
            assert_eq!(kit.tokens.button_primary.color, hsla(focus_border(&t)));
            assert_eq!(kit.button_primary_foreground, hsla(background(&t)));
            assert_ne!(
                kit.tokens.button_primary_hover.color,
                hsla(focus_border(&t))
            );
            assert_eq!(kit.tokens.button_danger.color, hsla(error(&t)));
            assert_eq!(kit.button_danger_foreground, hsla(background(&t)));
            // The Radio (the assistant's small choices): checked, unchecked,
            // its mark and a disabled label.
            assert_eq!(kit.tokens.primary.color, hsla(focus_border(&t)));
            assert_eq!(kit.primary_foreground, hsla(background(&t)));
            assert_eq!(kit.input, hsla(divider(&t)));
            assert_eq!(kit.muted_foreground, hsla(muted(&t)));
            // The Checkbox (Preferences' update check): checked fill and
            // its tick, the unchecked box (border `input`, fill derived
            // from `input` / `background`), the label and a disabled one.
            assert_eq!(kit.primary, hsla(focus_border(&t)));
            assert_eq!(kit.foreground, hsla(foreground(&t)));
            assert_eq!(kit.background, hsla(panel_background(&t)));
            assert_ne!(
                kit.input_background(),
                hsla(focus_border(&t)),
                "the empty box never reads as checked"
            );
            // The Spinner (Preferences' reindex progress): its icon falls
            // back to the kit's `foreground`.
            assert_eq!(kit.foreground, hsla(foreground(&t)));
        });
    }

    /// WCAG relative luminance of an sRGB colour.
    fn luminance(c: Rgba) -> f32 {
        let lin = |v: f32| {
            if v <= 0.03928 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * lin(c.r) + 0.7152 * lin(c.g) + 0.0722 * lin(c.b)
    }

    fn contrast(a: Rgba, b: Rgba) -> f32 {
        let (la, lb) = (luminance(a), luminance(b));
        (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
    }

    /// Every built-in theme, light and dark, keeps a toast readable on its
    /// card (the panel surface). The message is the drawers' own text on the
    /// same surface: at least 3:1 on all of them (the lowest, Solarized
    /// Light, is 3.6). The kinds' icons are the theme's own palette, as the
    /// editor and the TUI use it, and only repeat what the message says;
    /// some light themes keep them faint (Kanagawa Lotus' yellow is 1.8), so
    /// the floor here is that they show at all.
    #[test]
    fn toasts_read_on_every_builtin_theme() {
        for t in Theme::builtins() {
            let card = panel_background(&t);
            let text = contrast(foreground(&t), card);
            assert!(text >= 3.0, "{}: message {text:.2}", t.name);
            for (role, c) in [
                ("info", accent(&t)),
                ("success", available(&t)),
                ("warning", caution(&t)),
                ("error", error(&t)),
            ] {
                let ratio = contrast(c, card);
                assert!(ratio >= 1.5, "{}: {role} icon {ratio:.2}", t.name);
            }
        }
    }

    fn theme(bg: ThemeColor, fg: ThemeColor, sel: ThemeColor) -> Theme {
        Theme {
            bg,
            fg,
            selection_bg: sel,
            ..Theme::default()
        }
    }

    #[test]
    fn rgb_passes_through() {
        let t = Theme::default();
        assert_eq!(
            to_gpui(&ThemeColor::Rgb(255, 0, 51), Token::Foreground, &t),
            Rgba {
                r: 1.0,
                g: 0.0,
                b: 0.2,
                a: 1.0
            }
        );
    }

    #[test]
    fn ansi_cube_and_ramp_match_xterm() {
        assert_eq!(ansi(16), (0, 0, 0));
        assert_eq!(ansi(21), (0, 0, 255));
        assert_eq!(ansi(196), (255, 0, 0));
        assert_eq!(ansi(232), (8, 8, 8));
        assert_eq!(ansi(255), (238, 238, 238));
    }

    #[test]
    fn reset_is_opaque() {
        let t = Theme::default();
        for tok in [
            Token::Background,
            Token::Foreground,
            Token::SelectionBackground,
        ] {
            assert_eq!(to_gpui(&ThemeColor::Reset, tok, &t).a, 1.0);
        }
    }

    #[test]
    fn reset_background_follows_the_theme_not_a_constant() {
        let t = theme(
            ThemeColor::Rgb(10, 20, 30),
            ThemeColor::Reset,
            ThemeColor::Reset,
        );
        assert_eq!(
            to_gpui(&ThemeColor::Reset, Token::Background, &t),
            rgba((10, 20, 30))
        );
    }

    #[test]
    fn light_theme_with_reset_bg_stays_light() {
        // Dark explicit text, Reset page: page must be light, not the dark default.
        let t = theme(
            ThemeColor::Reset,
            ThemeColor::Rgb(20, 20, 20),
            ThemeColor::Reset,
        );
        assert!(luma(resolve(&t.bg, Token::Background, &t)) > 0.5);
    }

    #[test]
    fn light_theme_with_reset_fg_gets_dark_text() {
        let t = theme(
            ThemeColor::Rgb(250, 250, 240),
            ThemeColor::Reset,
            ThemeColor::Reset,
        );
        assert!(luma(resolve(&t.fg, Token::Foreground, &t)) < 0.5);
    }

    #[test]
    fn reset_selection_is_visible_on_dark_and_light() {
        for (bg, fg) in [
            (ThemeColor::Rgb(20, 20, 20), ThemeColor::Rgb(230, 230, 230)),
            (ThemeColor::Rgb(250, 250, 250), ThemeColor::Rgb(20, 20, 20)),
            (ThemeColor::Reset, ThemeColor::Reset),
        ] {
            let t = theme(bg, fg, ThemeColor::Reset);
            assert_ne!(selection_background(&t), background(&t));
        }
    }

    #[test]
    fn explicit_selection_equal_to_background_is_still_visible() {
        let c = ThemeColor::Rgb(40, 40, 40);
        let t = theme(c.clone(), ThemeColor::Rgb(230, 230, 230), c);
        assert_ne!(selection_background(&t), background(&t));
    }

    #[test]
    fn distinct_explicit_selection_is_kept() {
        let t = theme(
            ThemeColor::Rgb(0, 0, 0),
            ThemeColor::Rgb(255, 255, 255),
            ThemeColor::Rgb(1, 2, 3),
        );
        assert_eq!(selection_background(&t), rgba((1, 2, 3)));
    }
    #[test]
    fn hover_is_visible_on_the_panel_it_sits_on() {
        for (panel, soft) in [
            (ThemeColor::Reset, ThemeColor::Reset),
            (ThemeColor::Rgb(40, 40, 40), ThemeColor::Rgb(40, 40, 40)),
            (ThemeColor::Rgb(40, 40, 40), ThemeColor::Rgb(60, 60, 60)),
        ] {
            let t = Theme {
                bg_panel: panel,
                bg_soft: soft,
                ..Theme::default()
            };
            assert_ne!(hover_background(&t), panel_background(&t));
            assert_ne!(selected_background(&t), panel_background(&t));
            assert_ne!(selected_background(&t), hover_background(&t));
        }
    }

    // ---- run_style ----

    fn run(role: Role, mods: u8) -> Run {
        Run::new("x", role, mods)
    }

    fn rgb_theme() -> Theme {
        Theme {
            bg: ThemeColor::Rgb(0, 0, 0),
            fg: ThemeColor::Rgb(200, 200, 200),
            gray: ThemeColor::Rgb(100, 100, 100),
            accent: ThemeColor::Rgb(10, 20, 250),
            yellow: ThemeColor::Rgb(250, 250, 0),
            code_bg: ThemeColor::Rgb(30, 30, 30),
            bg_soft: ThemeColor::Reset,
            selection_bg: ThemeColor::Rgb(60, 60, 90),
            ..Theme::default()
        }
    }

    #[test]
    fn plain_text_is_the_theme_foreground_with_no_background() {
        let t = rgb_theme();
        let s = run_style(&run(Role::Plain, 0), &t);
        assert_eq!(s.fg, foreground(&t));
        assert_eq!(s.bg, None);
        assert!(!(s.bold || s.italic || s.underline || s.strike));
    }

    #[test]
    fn roles_mirror_the_tui_table() {
        let t = rgb_theme();
        let sigil = run_style(&run(Role::Sigil, 0), &t);
        assert_eq!(sigil.fg, rgba((100, 100, 100)));
        let bold = run_style(&run(Role::Element(ElementKind::Bold), 0), &t);
        assert_eq!((bold.fg, bold.bold), (foreground(&t), true));
        let h3 = run_style(&run(Role::Element(ElementKind::HeadingH3), 0), &t);
        assert_eq!((h3.fg, h3.bold), (rgba((250, 250, 0)), true));
        let link = run_style(&run(Role::Element(ElementKind::Link), 0), &t);
        assert_eq!(link.fg, rgba((10, 20, 250)));
        let wiki = run_style(&run(Role::Element(ElementKind::WikiLink), 0), &t);
        assert_eq!(wiki.fg, fg_of(&t.blue, &t));
        // Underlined like the TUI editor (user decision, M3 review).
        assert!(link.underline && wiki.underline);
        assert!(!(link.bold || link.strike || wiki.bold || wiki.strike));
        // InlineCode's bg_soft is Reset here: no background of its own.
        let code = run_style(&run(Role::Element(ElementKind::InlineCode), 0), &t);
        assert_eq!(code.bg, None);
    }

    #[test]
    fn emphasis_mask_adds_modifiers() {
        let t = rgb_theme();
        let s = run_style(&run(Role::Plain, MOD_BOLD | MOD_ITALIC | MOD_STRIKE), &t);
        assert!(s.bold && s.italic && s.strike);
    }

    #[test]
    fn code_box_then_selection_stack_in_order() {
        let t = rgb_theme();
        let mut r = run(Role::CodeText, 0);
        r.code_box = true;
        assert_eq!(run_style(&r, &t).bg, Some(rgba((30, 30, 30))));
        r.overlays.insert(OverlayKind::Selection);
        assert_eq!(run_style(&r, &t).bg, Some(selection_background(&t)));
    }

    #[test]
    fn done_task_is_dimmed_toward_the_page_and_struck() {
        let t = rgb_theme();
        let mut r = run(Role::Plain, 0);
        r.overlays.insert(OverlayKind::TaskDone);
        let s = run_style(&r, &t);
        assert!(s.strike);
        assert!(s.fg.r < foreground(&t).r && s.fg.r > 0.0);
    }

    #[test]
    fn reset_caret_falls_back_to_bright_text() {
        let t = Theme {
            cursor: ThemeColor::Reset,
            fg_bright: ThemeColor::Rgb(255, 255, 255),
            ..rgb_theme()
        };
        assert_eq!(caret(&t), rgba((255, 255, 255)));
    }

    /// A view that reads [`current`] as it renders, and one that opted into
    /// [`observe`] (a cached view): one `set_current` reaches both, and
    /// `current` is the default theme until something installs one.
    struct Probe {
        drawn_in: Vec<String>,
        _theme: Option<gpui_kit::Subscription>,
    }

    impl gpui_kit::Render for Probe {
        fn render(
            &mut self,
            _: &mut gpui_kit::Window,
            cx: &mut gpui_kit::Context<Self>,
        ) -> impl gpui_kit::IntoElement {
            self.drawn_in.push(current(cx).name.clone());
            gpui_kit::div()
        }
    }

    #[gpui_kit::test]
    fn one_set_current_redraws_the_observing_view_in_the_new_theme(
        cx: &mut gpui_kit::TestAppContext,
    ) {
        cx.update(gpui_kit::init);
        assert_eq!(
            cx.read(|cx| current(cx).name.clone()),
            Theme::default().name,
            "before any is installed"
        );
        let (view, cx) = cx.add_window_view(|_, cx| Probe {
            drawn_in: Vec::new(),
            _theme: Some(observe(cx)),
        });
        cx.run_until_parked();
        let other = Theme {
            name: "Some Other Theme".to_string(),
            ..Theme::default()
        };
        cx.update(|_, cx| set_current(cx, other));
        cx.run_until_parked();
        view.read_with(cx, |p, _| {
            assert_eq!(
                p.drawn_in.last().map(String::as_str),
                Some("Some Other Theme"),
                "drawn again in the new theme: {:?}",
                p.drawn_in
            );
        });
    }
}
