## ADDED Requirements

### Requirement: Hashtag tokens SHALL be highlighted distinctly in the TUI editor

When the TUI text editor renders a line that contains a hashtag token (matching the label character set described in `note-labels`), the `#` character and the label name SHALL be highlighted with a style distinct from generic markdown links so that labels are visually recognizable.

#### Scenario: Hashtag rendered with label highlight
- **WHEN** the editor renders a line containing `Read up on #rust later`
- **THEN** the `#rust` span is rendered with the label highlight style and the rest of the line uses the default style

#### Scenario: Hashtag in code span is not highlighted as label
- **WHEN** the editor renders a line containing `` use `#foo` literally ``
- **THEN** `#foo` is rendered as code, not as a label

### Requirement: Following a hashtag SHALL open the search modal pre-filled with the label query

When the cursor is positioned on a hashtag token and the user triggers the "follow link" action (default keybinding `Ctrl+G`), the system SHALL open the same search modal that is opened by the "search notes" action (default keybinding `Ctrl+K`) and SHALL pre-fill its query input with `#<label>` so the user immediately sees notes carrying that label.

#### Scenario: Follow link on hashtag opens search modal
- **WHEN** the cursor is on `#important` and the user presses the follow-link key
- **THEN** the search modal opens with its query field pre-filled with `#important` and the results list shows notes carrying that label

#### Scenario: Follow link on regular wikilink retains existing behavior
- **WHEN** the cursor is on `[[Some Note]]` and the user presses the follow-link key
- **THEN** the system opens the linked note (existing behavior is unchanged)

#### Scenario: Follow link on plain text does nothing new
- **WHEN** the cursor is on plain text with no link or hashtag and the user presses the follow-link key
- **THEN** the search modal is not opened by this change (existing behavior is preserved)

### Requirement: The search modal SHALL accept a pre-filled query when opened programmatically

The note-browser search modal SHALL accept an initial query string when opened. When provided, the query field SHALL contain that string on open, the cursor SHALL be positioned at the end, and the results list SHALL reflect the initial query without requiring an extra keystroke.

#### Scenario: Modal opened with initial query
- **WHEN** the search modal is opened with the initial query `#important`
- **THEN** the query field displays `#important`, the cursor is at the end, and results matching that query are listed

#### Scenario: Modal opened with no initial query
- **WHEN** the search modal is opened by the regular search-notes action with no initial query
- **THEN** the query field is empty (existing behavior is unchanged)
