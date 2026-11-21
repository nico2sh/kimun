# GUI feature discoverability: design brief

Status: draft, 2026-11-21. Nothing is built.

## Problem

Five GUI features have a keyboard route and no visible entry point: pinned notes, saved searches, the quick note, the journal entry and the facet view. A user finds them only in the documentation or in the keyboard reference. The audit below lists where each one shows today.

| Feature | Visible today | Missing |
| --- | --- | --- |
| Pinned notes | Dialog from leader `f p` and a palette row. Pin only with leader `m p`. The empty text names the leader sequence. | Note menu item, any on-screen entry, any sign that a pin exists. |
| Saved searches | F3, leader, palette row `find · save query…`. | A Save and a Saved control in the Find panel. |
| Quick note | Ctrl+Shift+W and a palette row. | Any on-screen entry. |
| Journal entry | Ctrl+J, leader, palette. | Any on-screen entry. |
| Facet view | Funnel button in the group header, with a tooltip that names the chord. | A label that says what the result is. A teaching empty state. |

Other gaps found in the same pass: the empty editor group does not list actions, the title bar has one tooltip, and the palette is the safety net but nothing says so.

## Users and tone

From `.impeccable.md`: writers who keep plain-text notes and read at a desk. The interface is calm, quiet and focused. The text is the main thing. These rules from that file bind every decision here.

1. Text first. A new control is quieter than the text.
2. Structure by space and weight before lines and boxes.
3. A control says what it does by its shape and place, not only by a tooltip.
4. Everything works from the keyboard, with a visible focus ring.
5. Colors and sizes come from the theme and shared constants.

Two rules apply to this work, from the impeccable guidelines. First, the entry point is small and the detail is one step in. Second, no modal tour or tip. An empty state must teach: it says what the feature is for, then how to start.

## The decision to make first: where do pinned notes live

The rail holds views. Each rail cell shows a view in a slot, and the cell reads as selected while its view is open. `DrawerView` lives in the shared `kimun_notes::logic` crate. A new view therefore also appears in the TUI rail and in `view_registry`. Pinned notes today are a modal dialog, not a view. The dialog has at most nine rows and a fixed height.

Option A: make pinned notes a drawer view. The rail stays consistent. The cost is a slot for nine rows, a change in shared logic, and a rewrite of the dialog into a panel. The TUI already has its own pinned overlay. The two frontends then drift apart, or the TUI changes for no TUI reason.

Option B: put a cell in the rail that opens the dialog. This is quick. The cell never shows a selected state. This breaks the rule that a rail cell means a view. It also sets a precedent. The journal and the quick note then ask for cells too, and the rail becomes a toolbar.

Option C: keep the rail for views only. Show pinned notes as a section at the top of the Files panel, which is a view already. The section appears when at least one note is pinned. Users who never pin see no change. The note menu gets the pin and unpin items. The dialog stays for reordering.

Recommendation: option C. It adds no chrome for users who never pin. It makes a pin visible as a result of pinning. It leaves the rail with one meaning. If a later effort wants pins as a strip across the workspace, review option A then.

For actions (journal, quick note, new note), do not use the rail. Use the Files "New" menu, which already lists what you can make, with a chord for each. Add "Journal entry" and "Quick note" as items. If a later audit shows that users do not find the menu, a second surface is the next step. The candidate is a small action row at the foot of the rail, drawn apart from the view cells and with no selected state. Do not build it now.

## Changes, in build order

1. Note menu. Add "Pin note" and "Unpin note" (the label follows the state), each with its chord on the right, as the Files "New" menu does. Show a short flash after the action, the way the TUI says "pinned as 1 of 9".
2. Find panel. Add two ghost buttons beside the query field: "Save" and "Saved". Each has a tooltip built with `tooltip_with_chord`. Save is disabled while the query is empty.
3. Files "New" menu. Add "Journal entry" and "Quick note" below the note and directory items, with their chords.
4. Pinned section in the Files panel. A quiet heading, "Pinned", and up to nine rows in pin order, each with its digit. A click opens the note. A row whose note is missing shows dimmed, as the dialog does. The section header has a "Manage" text link that opens the existing dialog.
5. Empty states that teach.
   - Empty editor group: three or four lines, each an action and its chord: open a note, today's journal, quick note, command palette.
   - Pinned dialog and section: say what pinning does, then the chord.
   - Saved searches: say what a saved search is, then name the Save button.
   - New facet or collection view: name what the user can narrow by.
6. Facet view wording. Change the button tooltip and the menu item to describe the result ("Show note in parts"), and keep "Facet view" as the term in the keyboard reference and the documentation.
7. Palette hint. One line in the empty editor group says the palette lists every command. No modal, no tour, no first-run tip.

## Out of scope

A tour, a tip of the day, a toolbar, new rail cells, and any TUI change. The TUI keeps its own routes.

## Constraints from the repository

All colors come from the theme, with no literals. All text uses the existing tooltip and chip helpers in `chrome.rs`. Pin reads and writes stay in core through `logic::pinned`, and the UI never touches a file. The section in the Files panel must read pins off the UI thread, as the dialog does, and must not block the first paint of the panel.

## Open questions

1. Decide if the Files pinned section can collapse, and if it remembers its state.
2. Does "Show note in parts" read clearly to a new user, or does it need a user test with two or three people?
3. Decide if the Saved button in the Find panel shows a count.

## Verification

Each change gets a headless test in the pattern of the existing `*_tests.rs` files, and a manual pass in the running app for the focus ring, the tooltip text and the empty-state layout in both a light and a dark theme.
