# Right-click open actions (GUI)

Status: waiting for approval. Size: small change. Scope: GUI only.

## Intent

A right-click on a link, a note row, or a file row gives you the same choices a tab bar and a split give you. You can open the target in the current view, in a new tab, or in a split. You do not need to open it first and then move it.

## Goals

- A new Link menu opens from a right-click on a link or label in a note.
- Every list row that has a menu today gains open actions and a few helper actions.
- Open in new tab and open in split reuse the actions that exist today.

## Non-goals

- No new chord. The Link menu opens from a right-click only.
- No change to the Note menu, to the TUI, or to what a right-click does with a selection.
- No Reveal in file browser item on links.

## Constraints

- The GUI is a sibling frontend. File and path work stays in core, as `CLAUDE.md` says.
- The Link menu is its own menu with its own glossary term. `CONTEXT.md` already has the entry.
- A note is open in at most one group. If you pick new tab or split for a note that is open, the app focuses the existing tab or group. No second copy opens.

## Approaches

1. Extend what exists. The editor draws its own text, so a link has no element that can hold a menu. The editor already sends a context-menu request with the pointer position. The workspace finds the link at that position and shows a Link menu on the Note menu's overlay. Rows extend `with_file_menu`. Chosen.
2. Build a separate Link menu with the gpui-kit `context_menu`, anchored by hand. This avoids the Note menu code. But it gives the editor two menu mechanisms with separate focus, dismissal, and tests.
3. Move every menu onto the Note menu overlay. This gives one system. But it replaces menus that work today and touches every list for no gain.

The approach is my call, because the request named the outcome and not the method.

## Decisions

Link menu items by target kind:

- Note link (wikilink or markdown), including one to a missing note or one that matches several notes: Open, Open in new tab, Open in split right, Open in split down, Copy link. A missing note shows the create dialog first. Several matches show the chooser first. The note then opens where you chose.
- Link to an attachment: Open, Open in new tab, Open in default app, Copy link. No split items, because only notes open in a split today.
- Web link: Open in browser, Copy link.
- Label (hashtag): Search for the label, Copy label.

Link menu behavior:

- If there is no selection, a right-click on a target opens it. If there is a selection, the right-click copies it, as today.
- It replaces the Note menu for that click. A right-click anywhere else opens the Note menu, as today.
- It works in a facet view too. There, a plain Open repeats Open in new tab, which is what a click does in a facet view. So the facet view menu has no plain Open item. This is my call.
- Items that cannot act are hidden, never grayed out, as in the Note menu.
- Copy link copies the target as written in the note.

Row menu, for every list that uses `with_file_menu` (file browser, search results, LINKS, SEM, pinned notes):

- Note row, in this order:
  - Open, Open in new tab, Open in split right, Open in split down.
  - A separator, then Copy path, Pin note or Unpin note, and Reveal in file browser.
  - A separator, then Rename, Move, and Delete.
- Reveal in file browser does not show on the file browser itself.
- Folder row: Copy path above Rename, Move, and Delete. No open items, because a click on a folder opens it.
- Attachment row: Open, Open in new tab, Open in default app, then the same helper and file items. No split items.

## Left to the builder

- Replace the `new_tab: bool` of `follow_link_from` with a placement value (here, new tab, split with an axis). Pass it through the link chooser and the create dialog.
- Add an editor method that returns the follow target under a pointer position, not under the caret.
- Write the menu item lists as pure functions of a small context, as `menu_items` does for the Note menu, so tests run without a window.
- Find the existing action for Reveal in file browser. If none exists, add one.
- Copy through the existing clipboard seam.
- Add headless tests with `window.right_click`, as the existing menu tests do. Cover each target kind, a facet view, a selection, a note that is already open, and a missing note.
