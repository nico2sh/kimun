//! **Autocomplete in a query field**: the popup under FIND's field and the
//! search browser's / finder's, completing `#label`, the `<` `>` `=` note
//! operators of link filters, and a leading `?` into a saved search's query.
//! The TUI offers the same triggers in the same two places (its note browser
//! and its FIND panel); the rules are the shared engine's
//! ([`kimun_notes::logic::autocomplete`], mode `SearchQuery`).
//!
//! The owner view keeps one [`InputAutocomplete`] next to its field and
//! implements [`QueryFieldOwner`]; everything else is wired here, once, for
//! every owner:
//!
//! - the field's change: [`InputAutocomplete::on_change`], which says whether
//!   the owner should treat it as typing (an accept's own change is not);
//! - the field's other movement (a caret move, focus leaving):
//!   [`watch`] wakes the owner, [`show`] reconciles in its render;
//! - the keys the field's actions would take (Up, Down, Tab, Enter, Escape),
//!   before the field sees them: [`popup_keys`];
//! - the popup itself: [`show`].
//!
//! Accepting writes into the kit's field through its own edit path
//! (`replace_all`, so the field's undo history keeps it), then puts the caret
//! where the action says. A `?name` accept also reports the saved search's
//! name, which the owner pins as its breadcrumb.
//!
//! The popup hangs under the field, as wide as the field, and is drawn as a
//! layer above everything else so the list under it cannot cover it.

use std::sync::Arc;

use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::input::{
    Enter, Escape, IndentInline, InputState, MoveDown, MoveUp, Position,
};
use gpui_kit::{
    Anchor, AnyElement, App, Context, Entity, Focusable as _, InteractiveElement, IntoElement,
    MouseDownEvent, ParentElement as _, Pixels, Role, ScrollEvent, SharedString,
    StatefulInteractiveElement as _, Styled as _, Subscription, Window, anchored, deferred, div,
    point, px,
};
use kimun_core::NoteVault;
use kimun_notes::logic::autocomplete::{
    AutocompleteEngine, AutocompleteHost, AutocompleteMode, AutocompleteState, HandleKeyOutcome,
    PopupCommand, TriggerKind, TriggerOptions, VaultSuggestions,
};
use kimun_notes::logic::editor::snapshot::EditorSnapshot;
use kimun_notes::logic::message::Notifier;
use kimun_notes::settings::themes::Theme;

use crate::autocomplete::{PopupRow, popup_row, wheel_command};
use crate::theme;

/// What running a popup command did.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Ran {
    /// No popup is open, or the command is not the popup's: the field's key.
    Pass,
    /// The popup took it (moved, dismissed).
    Handled,
    /// A suggestion was written into the field. The saved search's name when
    /// a `?name` was accepted.
    Accepted(Option<String>),
}

/// The popup's id, for tests and the row ids.
pub(super) const POPUP_ID: &str = "input-autocomplete";

/// A one-line field as the engine's host: its text and caret.
struct FieldHost {
    text: String,
    /// The caret as a char column (the snapshot's unit).
    caret_col: usize,
}

impl FieldHost {
    fn of(input: &InputState) -> Self {
        let text = input.value().to_string();
        let byte = input.cursor().min(text.len());
        let caret_col = text
            .get(..byte)
            .map_or_else(|| text.chars().count(), |head| head.chars().count());
        Self { text, caret_col }
    }
}

impl AutocompleteHost for FieldHost {
    fn buffer_snapshot(&self) -> EditorSnapshot {
        // No stable revision: the engine rebuilds from this short line each time.
        EditorSnapshot::owned(
            vec![self.text.clone()],
            (0, self.caret_col),
            std::num::NonZeroU64::MIN,
        )
    }

    fn cache_key(&self) -> Option<std::num::NonZeroU64> {
        None
    }

    fn screen_anchor_for(&self, _byte_offset: usize) -> Option<(u16, u16)> {
        // The popup hangs under the field, not at the caret.
        Some((0, 0))
    }
}

/// The autocomplete of one query field. See the module docs.
pub(super) struct InputAutocomplete {
    engine: AutocompleteEngine,
    /// The text an accept just wrote: the field's change event for it is ours,
    /// already handled, and must not reopen the popup on the word completed.
    own_edit: Option<SharedString>,
    /// The caret when the popup last looked, so a move is noticed.
    caret: usize,
}

impl InputAutocomplete {
    /// Complete from `vault`'s labels, notes and saved searches; results wake
    /// the window through `redraw`.
    pub(super) fn new(vault: Arc<NoteVault>, redraw: Notifier) -> Self {
        let mut engine = AutocompleteEngine::new(
            Arc::new(VaultSuggestions { vault }),
            AutocompleteMode::SearchQuery,
        )
        .with_trigger_opts(TriggerOptions {
            // A query field has no Markdown headers, code spans or links.
            disambiguate_header: false,
            apply_exclusion_zone: false,
            ..TriggerOptions::default()
        });
        engine.set_redraw_callback(redraw);
        Self {
            engine,
            own_edit: None,
            caret: 0,
        }
    }

    /// The popup's state while it has something to offer. A list that is only
    /// what was typed already (`#rust` with `rust` the one label) offers
    /// nothing, so it stays out of the way of Enter and Down. A saved search
    /// always has something: accepting `?name` expands to its stored query,
    /// even when the name is typed in full.
    pub(super) fn state(&self) -> Option<&AutocompleteState> {
        self.engine
            .is_open()
            .then(|| self.engine.state())
            .flatten()
            .filter(|s| {
                s.kind == TriggerKind::SavedSearch
                    || s.items.iter().any(|item| item.display != s.query)
            })
    }

    /// Drop the popup and whatever query it has pending: the field's text was
    /// replaced from outside (a tag, a saved search, a sort), so what it was
    /// completing is gone.
    pub(super) fn close(&mut self) {
        self.engine.close();
        self.own_edit = None;
    }

    /// The suggestions shown, top first (tests).
    #[cfg(test)]
    pub(super) fn shown(&self) -> Vec<String> {
        self.state()
            .map(|s| s.items.iter().map(|i| i.display.clone()).collect())
            .unwrap_or_default()
    }

    /// The highlighted suggestion's position (tests).
    #[cfg(test)]
    pub(super) fn highlighted(&self) -> Option<usize> {
        self.state().map(|s| s.highlighted)
    }

    /// The field's text changed. `true` when it was typing (or paste, IME):
    /// the popup reconciles and the owner reacts. `false` when it is the
    /// change an accept just made, already handled by the accept: reconciling
    /// would reopen the popup on the word just completed.
    pub(super) fn on_change(&mut self, input: &InputState) -> bool {
        let value = input.value();
        if self.own_edit.take().is_some_and(|own| *own == *value) {
            return false;
        }
        self.caret = input.cursor();
        self.engine.sync(&FieldHost::of(input));
        true
    }

    /// Whether a move of the field's caret or focus is something to look at:
    /// only while the popup holds state, so an idle field's caret blink never
    /// wakes its owner.
    fn needs_look(&self, input: &InputState, focused: bool) -> bool {
        self.engine.state().is_some() && (!focused || input.cursor() != self.caret)
    }

    /// Everything but typing: drain finished queries, close when the field
    /// lost the keyboard, follow a caret move (closing when it left the
    /// trigger).
    fn settled(&mut self, input: &InputState, focused: bool) {
        // A frame is drawn after the accept's change event was delivered, so a
        // note still standing is for an event that never came (the accept
        // wrote what was there): it must not swallow a later real change.
        self.own_edit = None;
        self.engine.poll_results();
        if !focused {
            self.engine.close();
            return;
        }
        if self.engine.is_open() && input.cursor() != self.caret {
            self.caret = input.cursor();
            self.engine.refresh_if_open(&FieldHost::of(input));
        }
    }

    /// Run `command` on the open popup. An accept is written into `input`.
    fn run(
        &mut self,
        command: PopupCommand,
        input: &Entity<InputState>,
        window: &mut Window,
        cx: &mut App,
    ) -> Ran {
        if self.state().is_none() {
            return Ran::Pass;
        }
        let host = FieldHost::of(input.read(cx));
        match self.engine.handle_command(command, &host) {
            HandleKeyOutcome::NotHandled => Ran::Pass,
            HandleKeyOutcome::Consumed | HandleKeyOutcome::Dismissed => Ran::Handled,
            HandleKeyOutcome::Accepted(action) => {
                let mut text = host.text;
                if action.range.end > text.len()
                    || !text.is_char_boundary(action.range.start)
                    || !text.is_char_boundary(action.range.end)
                {
                    return Ran::Handled;
                }
                text.replace_range(action.range.clone(), &action.new_text);
                let cursor = action.new_cursor_byte.min(text.len());
                let character = text
                    .get(..cursor)
                    .map_or(0, |head| head.encode_utf16().count());
                self.own_edit = Some(SharedString::from(text.clone()));
                self.caret = cursor;
                input.update(cx, |input, cx| {
                    input.replace_all(text, window, cx);
                    input.set_cursor_position(
                        Position {
                            line: 0,
                            character: u32::try_from(character).unwrap_or(u32::MAX),
                        },
                        window,
                        cx,
                    );
                });
                Ran::Accepted(action.saved_search_name)
            }
        }
    }

    /// The pointer over row `offset` (from the window's top): highlight it.
    /// True when the highlight moved (a redraw is due).
    fn hover(&mut self, offset: usize) -> bool {
        let Some(state) = self.engine.state_mut() else {
            return false;
        };
        let (start, end) = state.visible_window();
        let idx = start + offset;
        if idx < end && state.highlighted != idx {
            state.highlighted = idx;
            return true;
        }
        false
    }
}

/// A view that owns a query field and an [`InputAutocomplete`] for it. What it
/// supplies is how to reach them and what an accept means to it; the popup's
/// keys, clicks, rendering and bookkeeping are shared.
pub(super) trait QueryFieldOwner: Sized + 'static {
    /// The field's autocomplete, when this view has one (a picker kind may not).
    fn autocomplete_mut(&mut self) -> Option<&mut InputAutocomplete>;

    /// The field.
    fn field(&self) -> &Entity<InputState>;

    /// A suggestion was written into the field; `name` is the saved search's
    /// name when a `?name` was accepted. The field's text moved: run it.
    fn accepted(&mut self, name: Option<String>, cx: &mut Context<Self>);

    /// Run `command` on the open popup, before the field sees the key. True
    /// when the popup took it.
    fn popup(
        &mut self,
        command: PopupCommand,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let input = self.field().clone();
        let Some(ac) = self.autocomplete_mut() else {
            return false;
        };
        match ac.run(command, &input, window, cx) {
            Ran::Pass => false,
            Ran::Handled => {
                cx.notify();
                true
            }
            Ran::Accepted(name) => {
                self.accepted(name, cx);
                true
            }
        }
    }
}

/// Wake `V` when the field's caret or focus moved under an open popup, so the
/// next render can follow it or close it.
pub(super) fn watch<V: QueryFieldOwner>(
    input: &Entity<InputState>,
    window: &mut Window,
    cx: &mut Context<V>,
) -> Subscription {
    cx.observe_in(input, window, |this: &mut V, input, window, cx| {
        let focused = input.read(cx).focus_handle(cx).is_focused(window);
        let look = this
            .autocomplete_mut()
            .is_some_and(|ac| ac.needs_look(input.read(cx), focused));
        if look {
            cx.notify();
        }
    })
}

/// Give an open popup the keys that are its own: Up, Down, Tab, Enter, Escape.
/// Register this **before** the owner's own capture handlers for the same
/// actions: when the popup takes a key it stops there, otherwise the owner's
/// handler runs as if there were no popup.
pub(super) fn popup_keys<V: QueryFieldOwner, E: InteractiveElement>(
    el: E,
    cx: &mut Context<V>,
) -> E {
    el.capture_action(cx.listener(|this: &mut V, _: &MoveUp, window, cx| {
        if this.popup(PopupCommand::Up, window, cx) {
            cx.stop_propagation();
        }
    }))
    .capture_action(cx.listener(|this: &mut V, _: &MoveDown, window, cx| {
        if this.popup(PopupCommand::Down, window, cx) {
            cx.stop_propagation();
        }
    }))
    .capture_action(cx.listener(|this: &mut V, _: &IndentInline, window, cx| {
        if this.popup(PopupCommand::Accept, window, cx) {
            cx.stop_propagation();
        }
    }))
    .capture_action(cx.listener(|this: &mut V, _: &Enter, window, cx| {
        if this.popup(PopupCommand::Accept, window, cx) {
            cx.stop_propagation();
        }
    }))
    .capture_action(cx.listener(|this: &mut V, _: &Escape, window, cx| {
        if this.popup(PopupCommand::Dismiss, window, cx) {
            cx.stop_propagation();
        }
    }))
}

/// Estimated height of `rows` popup rows, for deciding which side of the
/// field has room. Only the decision uses it: the popup is laid out by its
/// content.
fn estimated_height(rows: usize, window: &Window) -> Pixels {
    let row = window.rem_size() * 1.25 + px(4.);
    row * rows as f32 + px(6.)
}

/// Reconcile `owner`'s autocomplete with its field, and build the popup under
/// the field when it has something to show. Call from the owner's render.
pub(super) fn show<V: QueryFieldOwner>(
    owner: &mut V,
    t: &Theme,
    window: &mut Window,
    cx: &mut Context<V>,
) -> Option<AnyElement> {
    let input = owner.field().clone();
    let focused = input.read(cx).focus_handle(cx).is_focused(window);
    let field = input.read(cx).input_bounds();
    let ac = owner.autocomplete_mut()?;
    ac.settled(input.read(cx), focused);
    let state = ac.state()?;
    let (start, end) = state.visible_window();
    let rows = (start..end).map(|idx| {
        let offset = idx - start;
        popup_row(
            PopupRow {
                item: &state.items[idx],
                query: &state.query,
                offset,
                highlighted: idx == state.highlighted,
                accessible: true,
            },
            t,
            |row| row.w_full().gap_3().px_2().py_0p5().text_sm(),
            |secondary| secondary.text_xs().whitespace_nowrap().overflow_hidden(),
            cx.listener(move |this: &mut V, _: &MouseDownEvent, window, cx| {
                this.popup(PopupCommand::ClickRow(offset), window, cx);
                cx.stop_propagation();
            }),
            cx.listener(move |this: &mut V, _: &gpui_kit::MouseMoveEvent, _, cx| {
                if this.autocomplete_mut().is_some_and(|ac| ac.hover(offset)) {
                    cx.notify();
                }
            }),
        )
    });
    // The field's box is the last frame's: before its first paint it has no
    // width, so a popup that early takes a usable one.
    let width = if field.size.width > px(0.) {
        field.size.width
    } else {
        px(240.)
    };
    let list = div()
        .id(POPUP_ID)
        .test_support()
        .role(Role::ListBox)
        .aria_label("Suggestions")
        .occlude()
        .flex()
        .flex_col()
        .w(width)
        .py_0p5()
        .border_1()
        .border_color(theme::focus_border(t))
        .bg(theme::panel_background(t))
        .rounded_md()
        .overflow_hidden()
        .on_scroll(
            cx.listener(move |this: &mut V, event: &ScrollEvent, window, cx| {
                if let Some(command) =
                    wheel_command(event.delta.pixel_delta(window.line_height()).y)
                {
                    this.popup(command, window, cx);
                }
                // Over the popup the wheel never reaches the list under it.
                cx.stop_propagation();
            }),
        )
        .children(rows);
    // Under the field; above it when the window has no room below, so the
    // popup never covers what is being typed.
    let margin = px(8.);
    let gap = px(2.);
    let height = estimated_height(end - start, window);
    let flip = field.bottom() + gap + height > window.viewport_size().height - margin
        && field.top() - gap - height >= margin;
    let (at, corner) = if flip {
        (point(field.left(), field.top() - gap), Anchor::BottomLeft)
    } else {
        (point(field.left(), field.bottom() + gap), Anchor::TopLeft)
    };
    Some(
        deferred(
            anchored()
                .anchor(corner)
                .position(at)
                .snap_to_window_with_margin(margin)
                .child(list),
        )
        .with_priority(2)
        .into_any_element(),
    )
}
