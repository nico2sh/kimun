//! The **Footer** docked under the editor Panel (M5b-2): the open note's
//! path as a breadcrumb, whether it has unsaved edits, what the vault is
//! doing (indexing), and whether notes may be saved. G5 completed it with what the TUI's second status line shows
//! of the document: the caret's line and column, the find bar's place among
//! its matches, and the open note's backlink and property counts. The
//! backlink count is a button (M7a G1): it shows LINKS on its Backlinks tab;
//! the property count too (M7a G2): it opens the properties dialog.
//!
//! **Narrow windows.** The path takes only the room the pinned segments
//! leave, so it shrinks first, with an ellipsis; of the pinned segments the
//! kimün server's state is cut first, then the note's counts; the caret,
//! the matches and the save, read-only and indexing states keep their
//! width.
//!
//! **The kimün server** (M7c G9): its state in the TUI's words ("rag:
//! online", "rag: offline", …), muted, at the right end; nothing when no
//! server is configured.
//!
//! **Flashes** are not drawn here: the
//! [`Workspace`](crate::workspace::Workspace) shows them as the kit's
//! toasts, in the window's bottom-right corner (N1).
//!
//! The vim mode is not repeated here: the editor's own mode line, the row
//! right above the Footer, shows it with the pending command.
//!
//! The state is plain data owned by the
//! [`Workspace`](crate::workspace::Workspace), plus the [`DocFacts`] it reads off the editor and the note
//! session each frame; [`render`] draws both with the kit's `StatusBar` in
//! kimün's theme.

use gpui_kit::assets::IconName;
use gpui_kit::base::TestSupportExt;
use gpui_kit::component::Icon;
use gpui_kit::component::status_bar::StatusBar;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, ClickEvent, InteractiveElement, IntoElement, ParentElement, Role,
    SharedString, StatefulInteractiveElement, Styled, Window, div, rems,
};
use kimun_core::nfs::VaultPath;
use kimun_notes::logic::editor::find_bar::MatchPosition;
use kimun_notes::logic::layout_tree::RailPosition;
use kimun_notes::settings::themes::Theme;
use kimun_notes::util::git_status::GitSummary;

use crate::theme;

/// Element ids, for tests.
pub(crate) const ROW_ID: &str = "status-row";
pub(crate) const PATH_ID: &str = "status-path";
pub(crate) const SAVE_ID: &str = "status-save";
pub(crate) const INDEXING_ID: &str = "status-indexing";
/// The workspace's git summary.
pub(crate) const GIT_ID: &str = "status-git";
pub(crate) const READ_ONLY_ID: &str = "status-read-only";
pub(crate) const CARET_ID: &str = "status-caret";
pub(crate) const MATCHES_ID: &str = "status-matches";
pub(crate) const BACKLINKS_ID: &str = "status-backlinks";
pub(crate) const PROPERTIES_ID: &str = "status-properties";
pub(crate) const RAG_ID: &str = "status-rag";
pub(crate) const ZOOM_ID: &str = "status-zoom";
pub(crate) const ZOOM_RESTORE_ID: &str = "status-zoom-restore";
/// The button that hides and shows the rail (ADR-0055 §8).
pub(crate) const RAIL_TOGGLE_ID: &str = "status-rail-toggle";

/// How much faster than the other shrinking segments the server's state
/// gives up its room in a narrow window: by far the most, so it goes first.
const RAG_SHRINK: f32 = 100.;

/// The save state's two labels.
pub(crate) const UNSAVED: &str = "Unsaved";
pub(crate) const SAVED: &str = "Saved";

/// What the footer shows.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct Status {
    /// The note in the editor, `None` before one opened.
    pub(crate) note: Option<VaultPath>,
    /// The attachment the editor Panel shows instead (no note is open then):
    /// its path is the breadcrumb, with no save state or counts.
    pub(crate) attachment: Option<VaultPath>,
    /// The editor holds edits newer than the last save.
    pub(crate) dirty: bool,
    /// The vault's index is being brought up to date.
    pub(crate) indexing: bool,
    /// Notes must not be written (the vault has names that differ only by
    /// case; Preferences > Workspaces says so and offers a way out). Saves
    /// honour it.
    pub(crate) read_only: bool,
    /// The workspace's git summary (the footer words it `git ✓`, `git ●3`);
    /// `None` when the workspace is not a repository (or `git` is missing).
    pub(crate) git: Option<GitSummary>,
    /// The kimün server's state as the TUI's footer words it
    /// (`RagStatus::label`: "rag: online", …); `None` with no server
    /// configured.
    pub(crate) rag: Option<&'static str>,
}

/// What the footer reads off the editor and the open note each frame: each
/// segment shows only when it has a value (the TUI's `DocState` rule).
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct DocFacts {
    /// The caret's 1-based (line, column), while the editor has the
    /// keyboard and its find bar is closed.
    pub(crate) caret: Option<(usize, usize)>,
    /// The find bar's place among its matches, while it is open and its
    /// pattern matches.
    pub(crate) find: Option<MatchPosition>,
    /// How many notes link to the open note, once loaded.
    pub(crate) backlinks: Option<usize>,
    /// How many properties the open note's front matter holds, once loaded.
    pub(crate) properties: Option<usize>,
}

/// The caret segment's text and its spoken form.
pub(crate) fn caret_label((line, col): (usize, usize)) -> (String, String) {
    (
        format!("Ln {line}, Col {col}"),
        format!("Line {line}, column {col}"),
    )
}

/// The matches segment's text and its spoken form: `3/12`, or the count
/// alone when the caret is on none of them.
pub(crate) fn matches_label(find: MatchPosition) -> (String, String) {
    let total = find.total;
    match find.current {
        Some(i) => (format!("{i}/{total}"), format!("Match {i} of {total}")),
        None => {
            let text = counted(total, "match", "matches");
            (text.clone(), text)
        }
    }
}

/// What the git segment's tooltip explains: the count is every path
/// `git status` lists, whatever became of it.
pub(crate) const GIT_TIP: &str = "Paths git sees as changed: modified, added, deleted or untracked";

/// `n` and the noun, singular for one.
pub(crate) fn counted(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// While the window is zoomed (ADR-0053): the footer's hint (`Zoomed · {chord}
/// to restore`) and what its restore button runs.
pub(crate) struct ZoomSegment<F> {
    pub(crate) hint: SharedString,
    pub(crate) on_restore: F,
}

/// The restore button's label.
pub(crate) const RESTORE: &str = "Restore";

/// The footer's button for the rail (ADR-0055 §8): it leads the row, so it
/// is on screen whenever an editor group or a zoomed slot is, which is
/// always. Its icon shows the rail's edge and is lit while the rail is
/// shown.
pub(crate) struct RailSegment<F> {
    pub(crate) position: RailPosition,
    pub(crate) hidden: bool,
    /// What the button says and its tooltip: `Hide rail (Ctrl+Shift+A)`.
    pub(crate) label: SharedString,
    pub(crate) on_toggle: F,
}

/// What leads the footer's row, in order: the rail's button and the
/// workspace chip (both only when the title bar is not there to carry
/// them), then the zoom hint and its restore button.
pub(crate) struct Leading<Z, R> {
    /// `None` while the title bar carries the button.
    pub(crate) rail: Option<RailSegment<R>>,
    pub(crate) workspace: Option<AnyElement>,
    pub(crate) zoom: Option<ZoomSegment<Z>>,
}

/// The footer for `status` and `facts`; a click on the backlink count runs
/// `on_backlinks`, one on the property count `on_properties`. With `zoom`,
/// the zoom hint and its restore button lead the row, after the rail's
/// button (`rail`).
pub(crate) fn render(
    status: &Status,
    facts: &DocFacts,
    theme: &Theme,
    on_backlinks: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    on_properties: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    leading: Leading<
        impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
        impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    >,
) -> impl IntoElement {
    let Leading {
        rail,
        workspace,
        zoom,
    } = leading;
    // Tabular figures: the caret's line and column, the match index and the
    // counts change in place, and should not jitter as their digits do.
    let label = |id: &'static str, text: SharedString, spoken: SharedString| {
        div()
            .id(id)
            .test_support()
            .role(Role::Status)
            .aria_label(spoken)
            .whitespace_nowrap()
            .font_features(crate::ui_font::tabular_figures())
            .child(text)
    };
    let fixed = |id: &'static str, (text, spoken): (String, String)| {
        label(id, text.into(), spoken.into()).flex_none()
    };
    // The note's counts give up their room before the live segments do.
    let count = |id: &'static str, text: String| {
        let text = SharedString::from(text);
        label(id, text.clone(), text)
            .min_w_0()
            .overflow_hidden()
            .text_ellipsis()
    };
    let note_open = status.note.is_some();
    // The path shrinks (cut short with an ellipsis) rather than pushing the
    // pinned labels out.
    let bar = StatusBar::new()
        .when_some(rail, |bar, rail| bar.child(rail_toggle(rail, theme)))
        // The workspace chip leads the row when the title bar is not there to
        // carry it.
        .when_some(workspace, |bar, chip| bar.child(chip))
        .when_some(zoom, |bar, zoom| bar.child(zoom_segment(zoom, theme)))
        .when_some(
            status.note.as_ref().or(status.attachment.as_ref()),
            |bar, path| bar.child(breadcrumb(path, theme)),
        )
        .when_some(facts.caret, |bar, caret| {
            bar.right(fixed(CARET_ID, caret_label(caret)))
        })
        .when_some(facts.find, |bar, find| {
            bar.right(fixed(MATCHES_ID, matches_label(find)))
        })
        .when_some(facts.backlinks.filter(|_| note_open), |bar, n| {
            let hover = theme::foreground(theme);
            bar.right(
                count(BACKLINKS_ID, counted(n, "backlink", "backlinks"))
                    .role(Role::Button)
                    .cursor_pointer()
                    .hover(move |style| style.text_color(hover))
                    .tooltip(|window, cx| Tooltip::new("Show backlinks").build(window, cx))
                    .on_click(on_backlinks),
            )
        })
        .when_some(facts.properties.filter(|_| note_open), |bar, n| {
            let hover = theme::foreground(theme);
            bar.right(
                count(PROPERTIES_ID, counted(n, "property", "properties"))
                    .role(Role::Button)
                    .cursor_pointer()
                    .hover(move |style| style.text_color(hover))
                    .tooltip(|window, cx| Tooltip::new("Edit properties").build(window, cx))
                    .on_click(on_properties),
            )
        })
        .when(note_open, |bar| bar.right(save_state(status.dirty, theme)))
        .when(status.read_only, |bar| {
            bar.right(
                label(READ_ONLY_ID, "Read-only".into(), "Read-only".into())
                    .flex_none()
                    .text_color(theme::error(theme)),
            )
        })
        .when_some(status.git, |bar, git| {
            bar.right(
                fixed(GIT_ID, (git.label(), git.spoken()))
                    .tooltip(|window, cx| Tooltip::new(GIT_TIP).build(window, cx)),
            )
        })
        .when(status.indexing, |bar| {
            bar.right(label(INDEXING_ID, "Indexing…".into(), "Indexing…".into()).flex_none())
        })
        // The server's state gives up its room first of all: it shrinks
        // far faster than the counts, with an ellipsis.
        .when_some(status.rag, |bar, rag| {
            bar.right(
                label(RAG_ID, rag.into(), rag.into())
                    .min_w_0()
                    .flex_shrink(RAG_SHRINK)
                    .overflow_hidden()
                    .text_ellipsis(),
            )
        })
        .bg(theme::panel_background(theme))
        .border_color(theme::divider(theme))
        .text_color(theme::muted(theme));
    div()
        .id(ROW_ID)
        .test_support()
        .role(Role::Footer)
        .aria_label("Status")
        .flex_none()
        .w_full()
        .child(bar)
}

/// The rail's button: its edge's panel icon, lit while the rail is shown,
/// and a tooltip that names the chord.
pub(crate) fn rail_toggle(
    rail: RailSegment<impl Fn(&ClickEvent, &mut Window, &mut App) + 'static>,
    theme: &Theme,
) -> impl IntoElement {
    // Hover is the cell's own surface, as the rail's side toggles have it:
    // the icon sets its own colour, so a hover colour on the wrapper would
    // never show.
    let hover = theme::hover_background(theme);
    let glyph = if rail.hidden {
        theme::muted(theme)
    } else {
        theme::accent(theme)
    };
    // No top-panel icon in the kit: the bottom one, turned over.
    let (icon, turned) = match rail.position {
        RailPosition::Left => (IconName::PanelLeft, false),
        RailPosition::Right => (IconName::PanelRight, false),
        RailPosition::Top => (IconName::PanelBottom, true),
        RailPosition::Bottom => (IconName::PanelBottom, false),
    };
    let tip = rail.label.clone();
    div()
        .id(RAIL_TOGGLE_ID)
        .test_support()
        .role(Role::Button)
        .aria_label(rail.label)
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        // A 20px target around the 16px icon, as high as the workspace chip:
        // it fits the footer (29px) and the title bar (34px) without growing
        // either (24px makes the footer 33px).
        .size_5()
        .rounded_md()
        .cursor_pointer()
        .hover(move |style| style.bg(hover))
        .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
        .on_click(rail.on_toggle)
        .child(
            Icon::new(icon)
                .size_4()
                .text_color(glyph)
                .when(turned, |icon| {
                    icon.rotate(gpui_kit::radians(std::f32::consts::PI))
                }),
        )
}

/// The zoom hint, in the foreground colour so it reads as the window's
/// state, then the restore button (the pointer's way out; Esc never is).
fn zoom_segment(
    zoom: ZoomSegment<impl Fn(&ClickEvent, &mut Window, &mut App) + 'static>,
    theme: &Theme,
) -> impl IntoElement {
    let hover = theme::foreground(theme);
    div()
        .flex()
        .flex_none()
        .flex_row()
        .items_center()
        .gap_2()
        .child(
            div()
                .id(ZOOM_ID)
                .test_support()
                .role(Role::Status)
                .aria_label(zoom.hint.clone())
                .whitespace_nowrap()
                .text_color(theme::foreground(theme))
                .child(zoom.hint),
        )
        .child(
            div()
                .id(ZOOM_RESTORE_ID)
                .test_support()
                .role(Role::Button)
                .aria_label(RESTORE)
                .whitespace_nowrap()
                .text_color(theme::accent(theme))
                .cursor_pointer()
                .hover(move |style| style.text_color(hover))
                .tooltip(|window, cx| Tooltip::new("Leave zoom").build(window, cx))
                .on_click(zoom.on_restore)
                .child(RESTORE),
        )
}

/// The breadcrumb's parts for `slices` (`VaultPath::get_slices`): every
/// folder up to two, else the first and the last folder around a `…`, then
/// the note's own name.
pub(crate) fn crumbs(mut slices: Vec<String>) -> Vec<String> {
    const ELLIPSIS: &str = "\u{2026}";
    let Some(name) = slices.pop() else {
        return Vec::new();
    };
    if slices.len() > 2 {
        let last = slices.pop().unwrap_or_default();
        slices.truncate(1);
        slices.push(ELLIPSIS.to_string());
        slices.push(last);
    }
    slices.push(name);
    slices
}

/// The note's path, folder by folder (see [`crumbs`]), the note's own name
/// brightest; chevrons between, as the kit's `Breadcrumb` draws them, in
/// kimün's tokens. Short of room, the folders shrink with an ellipsis and
/// the name never does. Not clickable yet (the FILES view, G3).
fn breadcrumb(note: &VaultPath, theme: &Theme) -> AnyElement {
    let parts_text = crumbs(note.get_slices());
    let last = parts_text.len().saturating_sub(1);
    let mut parts: Vec<AnyElement> = Vec::with_capacity(parts_text.len() * 2);
    for (i, slice) in parts_text.into_iter().enumerate() {
        if i > 0 {
            parts.push(
                Icon::new(IconName::ChevronRight)
                    .size_3()
                    .flex_none()
                    .text_color(theme::muted(theme))
                    .into_any_element(),
            );
        }
        let part = div().whitespace_nowrap().child(slice);
        let part = if i == last {
            part.flex_none().text_color(theme::foreground(theme))
        } else {
            part.min_w_0()
                .overflow_hidden()
                .text_ellipsis()
                .text_color(theme::muted(theme))
        };
        parts.push(part.into_any_element());
    }
    div()
        .id(PATH_ID)
        .test_support()
        .role(Role::Group)
        .aria_label(SharedString::from(note.to_string()))
        .flex()
        .flex_row()
        .items_center()
        .gap_1()
        .min_w_0()
        .overflow_hidden()
        .children(parts)
        .into_any_element()
}

/// "Unsaved" behind a dot in the unsaved colour, or a quiet "Saved". The
/// word carries the state; the dot only draws the eye to it.
fn save_state(dirty: bool, theme: &Theme) -> impl IntoElement {
    let text = if dirty { UNSAVED } else { SAVED };
    div()
        .id(SAVE_ID)
        .test_support()
        .role(Role::Status)
        .aria_label(text)
        .flex()
        .flex_none()
        .flex_row()
        .items_center()
        .gap_1()
        .whitespace_nowrap()
        .when(dirty, |row| {
            row.text_color(theme::foreground(theme)).child(
                div()
                    .flex_none()
                    .size(rems(0.375))
                    .rounded_full()
                    .bg(theme::unsaved(theme)),
            )
        })
        .child(text)
}

#[cfg(test)]
mod tests {
    use super::{MatchPosition, caret_label, counted, crumbs, matches_label};

    #[test]
    fn segment_labels_read_and_speak() {
        assert_eq!(
            caret_label((12, 4)),
            ("Ln 12, Col 4".into(), "Line 12, column 4".into())
        );
        let at = |current, total| matches_label(MatchPosition { current, total });
        assert_eq!(at(Some(3), 12), ("3/12".into(), "Match 3 of 12".into()));
        assert_eq!(at(None, 12), ("12 matches".into(), "12 matches".into()));
        assert_eq!(at(None, 1).0, "1 match");
        assert_eq!(counted(0, "backlink", "backlinks"), "0 backlinks");
        assert_eq!(counted(1, "property", "properties"), "1 property");
    }

    fn c(slices: &[&str]) -> Vec<String> {
        crumbs(slices.iter().map(|s| s.to_string()).collect())
    }

    #[test]
    fn short_paths_show_every_folder_and_the_name() {
        assert_eq!(c(&["todo.md"]), ["todo.md"]);
        assert_eq!(c(&["a", "b", "todo.md"]), ["a", "b", "todo.md"]);
        assert!(c(&[]).is_empty());
    }

    #[test]
    fn deep_paths_fold_the_middle_and_keep_the_name() {
        assert_eq!(
            c(&["a", "b", "c", "d", "todo.md"]),
            ["a", "\u{2026}", "d", "todo.md"]
        );
    }
}
