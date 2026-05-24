## ADDED Requirements

### Requirement: Search syntax SHALL accept `#<label>` as a label filter

The search query parser SHALL recognize a token of the form `#<name>` (where `<name>` matches the label character set) as a label filter and SHALL constrain results to notes carrying that label. The `#` prefix is the canonical short form. Multiple label filters in the same query SHALL be combined as AND (a note must carry every label listed).

#### Scenario: Single label filter
- **WHEN** the user submits the search query `#important`
- **THEN** only notes carrying the label `important` are returned

#### Scenario: Two label filters combine as AND
- **WHEN** the user submits `#important #todo`
- **THEN** only notes carrying both the `important` label and the `todo` label are returned

#### Scenario: Label filter combined with a free-text term
- **WHEN** the user submits `meeting #important`
- **THEN** results contain notes that carry the `important` label and also match the free-text term `meeting` per existing search rules

### Requirement: Search syntax SHALL accept `lb:<label>` as an equivalent long form

The search query parser SHALL recognize the prefix `lb:` followed by a label name as exactly equivalent to the `#<label>` short form. This prefix follows the same convention as existing search operator prefixes.

#### Scenario: Long-form filter is equivalent to short form
- **WHEN** the user submits `lb:important`
- **THEN** the result set is identical to submitting `#important`

#### Scenario: Long form combines with short form
- **WHEN** the user submits `lb:important #todo`
- **THEN** results contain notes carrying both labels

### Requirement: Label filters SHALL be matched case-insensitively

Label arguments in queries SHALL be normalized (lowercased) before being matched against the label index, so `#Important`, `#important`, and `lb:IMPORTANT` all return the same results.

#### Scenario: Mixed case in query
- **WHEN** the user submits `#Important`
- **THEN** notes carrying the label `important` are returned

### Requirement: Label filtering SHALL use the label index

Searches that include a label filter SHALL be served by the label index in the database and SHALL NOT require scanning note bodies for hashtag text.

#### Scenario: Label filter uses index, not body scan
- **WHEN** a search query contains a label filter
- **THEN** the database query plan resolves matching notes through the labels table

### Requirement: Unknown labels SHALL return an empty result set

A label filter referencing a label that no note carries SHALL produce an empty result set, not an error.

#### Scenario: Filter on nonexistent label
- **WHEN** the user submits `#nosuchlabel` and no note carries that label
- **THEN** the search returns zero results and reports no error
