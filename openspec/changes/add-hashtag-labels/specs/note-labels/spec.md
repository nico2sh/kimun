## ADDED Requirements

### Requirement: Hashtag tokens in note text SHALL be extracted as labels

When a note is indexed, the system SHALL scan the note body for hashtag tokens of the form `#<name>` and treat each unique token as a label attached to that note. A hashtag token starts with `#` followed by one or more characters from the allowed label character set (ASCII letters, digits, and underscore). Matching SHALL be case-insensitive and labels SHALL be normalized to lowercase before storage.

#### Scenario: Single hashtag in body produces one label
- **WHEN** a note containing the text `This is #Important` is indexed
- **THEN** the note is associated with the label `important`

#### Scenario: Duplicate hashtags collapse to a single association
- **WHEN** a note contains `#todo and later #todo and #TODO`
- **THEN** the note is associated with the label `todo` exactly once

#### Scenario: Hashtag inside a code fence is not a label
- **WHEN** a note contains `` `#notalabel` `` inside an inline code span or a fenced code block
- **THEN** no label is created for `notalabel`

#### Scenario: Hashtag inside YAML or TOML frontmatter is not a label
- **WHEN** a note opens with a `---...---` or `+++...+++` frontmatter block containing `#wip`
- **THEN** no label is created for `wip`; both `---` and `+++` delimiters are honored with CRLF tolerance

#### Scenario: Hashtag inside an HTML block or inline HTML span is not a label
- **WHEN** a note contains `<!-- #internal -->` or `<span data-tag="#bar">` in an HTML region
- **THEN** no label is created for `internal` or `bar`

#### Scenario: Hashtag inside a markdown link body is not a label
- **WHEN** a note contains `[docs](https://example.com#section)`
- **THEN** no label is created for `section`; URL fragments inside link bodies are excluded

#### Scenario: Hashtag adjacent to non-label characters terminates the label
- **WHEN** a note contains `#tag-with-dash`
- **THEN** the label `tag` is created and `-with-dash` is treated as following text

#### Scenario: Hashtag immediately followed by a Unicode alphanumeric character is not a label
- **WHEN** a note contains `#naïve` (the regex matches `#na` but the next char `ï` is alphanumeric)
- **THEN** no label is created; a `#` must not start a label when the character immediately after the ASCII match is alphanumeric (Unicode-aware) or `_`

#### Scenario: Hashtag immediately preceded by a Unicode alphanumeric character is not a label
- **WHEN** a note contains `café#draft`
- **THEN** no label is created for `draft`; the word-boundary rule applies to Unicode characters before `#` as well as ASCII

#### Scenario: Label names use the ASCII character set only
- **WHEN** a hashtag is scanned, the label name is formed by the pattern `[A-Za-z0-9_]+`
- **THEN** non-ASCII letters (e.g. `ï`, `é`, `ü`) cannot appear in a label name; any such character terminates the token and triggers the trailing word-boundary check above

#### Scenario: Long search queries are truncated at 8 KB
- **WHEN** a search query longer than 8192 bytes is submitted
- **THEN** the query is truncated on a UTF-8 character boundary to at most 8192 bytes before processing

### Requirement: Labels SHALL be persisted in a queryable database table

The system SHALL persist labels in a database table with an index on the label name so that lookups by label are fast and do not require scanning notes. The table SHALL associate each label name with the set of notes that carry it. The schema SHALL allow a single note to carry many labels and a single label to be carried by many notes.

#### Scenario: Label table is created on first run
- **WHEN** a vault is opened with no existing database
- **THEN** the database contains a labels table with an index on the label-name column

#### Scenario: Reindex rebuilds labels for an updated note
- **WHEN** a note that previously had label `draft` is edited to remove the hashtag and saved
- **THEN** after reindex the note is no longer associated with the label `draft`

#### Scenario: Deleting a note removes its label associations
- **WHEN** a note is deleted from the vault
- **THEN** the note's label associations are removed and any label with no remaining notes is removed from the table

### Requirement: Database schema version SHALL be bumped so existing vaults rebuild labels

The system SHALL bump the database schema version so that vaults created before this change trigger a full reindex on first open after upgrade and populate the new label table from existing notes. No user data SHALL be lost during the rebuild.

#### Scenario: Pre-existing vault populates labels on upgrade
- **WHEN** a vault whose database was created before this change is opened
- **THEN** the system rebuilds the index and the labels table is populated from current note contents

### Requirement: Core public API SHALL expose label lookups

The core public API SHALL expose a method to list every label in the vault and a method to list every note carrying a given label. Both methods SHALL use the label index and SHALL NOT scan note bodies. Label arguments SHALL be normalized (lowercased) by the API before lookup.

#### Scenario: List all labels in vault
- **WHEN** the caller asks the core API for all labels in the vault
- **THEN** the API returns each distinct label name exactly once

#### Scenario: List notes for a given label
- **WHEN** the caller asks the core API for the notes carrying label `important`
- **THEN** the API returns each note path that has been indexed with that label

#### Scenario: Label lookup is case-insensitive
- **WHEN** the caller passes `Important` as the label argument
- **THEN** the API returns the same result as passing `important`
