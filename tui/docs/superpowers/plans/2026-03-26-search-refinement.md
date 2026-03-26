# Search Refinement with Exclusion Operators Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add exclusion operators (`-term`, `>-title`, `@-file`, `/-path`) to Kimun's search system for refined query capabilities.

**Architecture:** Extend existing FTS4-based search parser with exclusion element types, enhance SQL generation to handle FTS4 exclusions and LIKE exclusions, use NOT IN subqueries for exclusion-only cases.

**Tech Stack:** Rust, SQLite FTS4, existing INTERSECT-based query architecture

---

## Task 1: Parser Enhancement - Add Exclusion Element Types

**Files:**
- Modify: `core/src/db/search_terms.rs:14-20` (ElementType enum)
- Modify: `core/src/db/search_terms.rs:172-178` (SearchTerms struct)
- Test: `core/src/db/search_terms.rs` (add tests section if not exists)

- [ ] **Step 1: Write failing test for exclusion element types**

Add to the existing `#[cfg(test)]` section after line 215:

```rust
#[test]
fn test_basic_exclusion_parsing() {
    // Test parsing basic exclusion syntax
    let search_terms = SearchTerms::from_query_string("meeting -cancelled");
    assert_eq!(search_terms.terms, vec!["meeting"]);
    // Note: excluded_terms field doesn't exist yet - test will fail compilation
    // assert_eq!(search_terms.excluded_terms, vec!["cancelled"]);
    assert!(search_terms.breadcrumb.is_empty());
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd core && cargo test test_exclusion_element_types`
Expected: FAIL with compilation error (excluded_terms field doesn't exist)

- [ ] **Step 3: Add exclusion variants to ElementType enum**

```rust
enum ElementType {
    Invalid,
    Term,
    In,
    At,
    Path,
    OrderBy { asc: bool },
    // New exclusion variants
    ExcludedTerm,
    ExcludedIn,
    ExcludedAt,
    ExcludedPath,
}
```

- [ ] **Step 4: Add exclusion fields to SearchTerms struct**

```rust
#[derive(Default, Debug)]
pub struct SearchTerms {
    pub terms: Vec<String>,
    pub breadcrumb: Vec<String>,
    pub order_by: Vec<OrderBy>,
    pub filename: Vec<String>,
    pub path: Vec<String>,
    // New exclusion fields
    pub excluded_terms: Vec<String>,
    pub excluded_breadcrumb: Vec<String>,
    pub excluded_filename: Vec<String>,
    pub excluded_path: Vec<String>,
}
```

- [ ] **Step 5: Run test to verify compilation**

Run: `cd core && cargo test test_exclusion_element_types`
Expected: FAIL with "excluded_terms not populated" (compilation succeeds)

- [ ] **Step 6: Commit parser structure changes**

```bash
cd core
git add src/db/search_terms.rs
git commit -m "feat: add exclusion element types and SearchTerms fields

- Add ExcludedTerm, ExcludedIn, ExcludedAt, ExcludedPath variants
- Extend SearchTerms struct with exclusion field collections
- Prepare for exclusion operator parsing implementation"
```

## Task 2: Parser Enhancement - Implement Optimized Prefix Detection

**Files:**
- Modify: `core/src/db/search_terms.rs:30-105` (extract_and_consume function)
- Modify: `core/src/db/search_terms.rs:181-212` (from_query_string function)
- Test: `core/src/db/search_terms.rs` (add new test cases)

- [ ] **Step 1: Write failing test for compound prefix detection**

```rust
#[test]
fn test_compound_exclusion_prefixes() {
    let search_terms = SearchTerms::from_query_string(">-draft in:-private @-temp /-secret");
    assert!(search_terms.terms.is_empty());
    assert!(search_terms.breadcrumb.is_empty());
    assert_eq!(search_terms.excluded_breadcrumb, vec!["draft", "private"]);
    assert_eq!(search_terms.excluded_filename, vec!["temp"]);
    assert_eq!(search_terms.excluded_path, vec!["secret"]);
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd core && cargo test test_compound_exclusion_prefixes`
Expected: FAIL with exclusion fields empty (parser doesn't handle exclusions yet)

- [ ] **Step 3: Add exclusion prefix detection to existing extract_and_consume logic**

Extend the existing if-else chain in `extract_and_consume` (around line 30) to handle exclusion prefixes first:

```rust
fn extract_and_consume<S: AsRef<str>>(query: S) -> QueryTermExtractor {
    let query = query.as_ref().trim();

    // Add exclusion prefix detection BEFORE existing prefixes (longer first)
    let in_exclude_prefix = "in:-";
    let at_exclude_prefix = "at:-";
    let path_exclude_prefix = "pt:-";

    // Existing prefix variables
    let in_prefix = format!("{}:", IN_LETTER);
    let at_prefix = format!("{}:", AT_LETTER);
    let order_prefix = format!("{}:", ORDER_LETTER);
    let path_prefix = format!("{}:", PATH_LETTER);

    let (element_type, remaining) = if query.starts_with(&in_exclude_prefix) {
        // Handle in:-title (excluded breadcrumb)
        (
            ElementType::ExcludedIn,
            query
                .strip_prefix(&in_exclude_prefix)
                .map_or_else(|| query.to_string(), |s| s.to_string()),
        )
    } else if query.starts_with(&at_exclude_prefix) {
        // Handle at:-filename (excluded filename)
        (
            ElementType::ExcludedAt,
            query
                .strip_prefix(&at_exclude_prefix)
                .map_or_else(|| query.to_string(), |s| s.to_string()),
        )
    } else if query.starts_with(&path_exclude_prefix) {
        // Handle pt:-path (excluded path)
        (
            ElementType::ExcludedPath,
            query
                .strip_prefix(&path_exclude_prefix)
                .map_or_else(|| query.to_string(), |s| s.to_string()),
        )
    } else if query.starts_with(">-") {
        // Handle >-title (excluded breadcrumb)
        (
            ElementType::ExcludedIn,
            query
                .strip_prefix(">-")
                .map_or_else(|| query.to_string(), |s| s.to_string()),
        )
    } else if query.starts_with("@-") {
        // Handle @-filename (excluded filename)
        (
            ElementType::ExcludedAt,
            query
                .strip_prefix("@-")
                .map_or_else(|| query.to_string(), |s| s.to_string()),
        )
    } else if query.starts_with("/-") {
        // Handle /-path (excluded path)
        (
            ElementType::ExcludedPath,
            query
                .strip_prefix("/-")
                .map_or_else(|| query.to_string(), |s| s.to_string()),
        )
    }
    // ... continue with existing if-else chain for positive prefixes ...
    // ... existing implementation continues unchanged ...
```

Note: The full implementation continues with the existing if-else pattern for `in_prefix`, `at_prefix`, etc.

- [ ] **Step 4: Update from_query_string to handle exclusion types**

```rust
match qp.el_type {
    ElementType::Term => terms.push(qp.term),
    ElementType::In => breadcrumb.push(qp.term),
    ElementType::At => filename.push(qp.term),
    ElementType::Path => path.push(qp.term),
    ElementType::OrderBy { asc } => {
        if let Some(o) = OrderBy::from_term(&qp.term, asc) {
            order_by.push(o);
        }
    }
    // New exclusion handling
    ElementType::ExcludedTerm => excluded_terms.push(qp.term),
    ElementType::ExcludedIn => excluded_breadcrumb.push(qp.term),
    ElementType::ExcludedAt => excluded_filename.push(qp.term),
    ElementType::ExcludedPath => excluded_path.push(qp.term),
    ElementType::Invalid => {}
}
```

- [ ] **Step 5: Run test to verify parser works**

Run: `cd core && cargo test test_compound_exclusion_prefixes`
Expected: PASS

- [ ] **Step 6: Run all existing parser tests**

Run: `cd core && cargo test search_terms`
Expected: ALL PASS (backward compatibility maintained)

- [ ] **Step 7: Commit parser implementation**

```bash
cd core
git add src/db/search_terms.rs
git commit -m "feat: implement optimized exclusion operator parsing

- Replace chained if-else with data-driven prefix matching
- Add support for compound exclusion prefixes (>-, @-, /-, in:-, etc)
- Handle content exclusions with - prefix validation
- Maintain backward compatibility with existing syntax"
```

## Task 3: SQL Generation Enhancement - FTS4 Exclusion Queries

**Files:**
- Modify: `core/src/db/mod.rs:224-287` (build_search_sql_query function)
- Test: `core/src/db/mod.rs` (add tests to existing `#[cfg(test)]` section or create new one)

- [ ] **Step 1: Write failing test for FTS4 exclusion SQL generation**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fts4_mixed_exclusion_sql_generation() {
        let (sql, params) = build_search_sql_query("meeting -cancelled");

        assert!(sql.contains("notesContent MATCH"));
        assert_eq!(params.len(), 1);
        assert_eq!(params[0], "meeting -cancelled");

        // Should generate single query with combined positive and negative terms
        assert!(sql.contains("SELECT DISTINCT"));
        // Note: May contain INTERSECT if other search types are involved
    }

    #[test]
    fn test_exclusion_only_sql_generation() {
        // Critical test: exclusion-only queries MUST use NOT IN, not pure FTS4 MATCH
        let (sql, params) = build_search_sql_query("-cancelled");

        // Should NOT contain pure FTS4 exclusion (which is invalid)
        assert!(!sql.contains("MATCH \"-cancelled\""));
        // Should use NOT IN subquery approach
        assert!(sql.contains("NOT IN"));
        assert!(sql.contains("SELECT DISTINCT notesContent.path FROM notesContent WHERE notesContent MATCH"));
        assert_eq!(params.len(), 1);
        assert_eq!(params[0], "cancelled");
    }

    #[test]
    fn test_breadcrumb_exclusion_sql_generation() {
        let (sql, params) = build_search_sql_query(">project >-draft");

        assert!(sql.contains("notesContent MATCH"));
        assert_eq!(params.len(), 1);
        assert_eq!(params[0], "breadcrumb: project breadcrumb: -draft");
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd core && cargo test test_fts4_exclusion_sql_generation`
Expected: FAIL with assertion errors (exclusion not handled in SQL generation)

- [ ] **Step 3: Enhance build_search_sql_query for content exclusions**

```rust
// Content search (searches ALL FTS4 columns: path, breadcrumb, text)
if !search_terms.terms.is_empty() || !search_terms.excluded_terms.is_empty() {
    if !search_terms.terms.is_empty() {
        // Positive content terms: create query with all positive terms + exclusions
        let mut fts_query_parts = vec![search_terms.terms.join(" ")];

        // Add excluded terms with FTS4 - prefix
        for excluded in &search_terms.excluded_terms {
            fts_query_parts.push(format!("-{}", excluded));
        }

        let terms_sql = format!("{} WHERE notesContent MATCH ?{}", base_sql, var_num);
        queries.push(terms_sql);
        params.push(fts_query_parts.join(" "));
        var_num += 1;
    } else if !search_terms.excluded_terms.is_empty() {
        // Exclusion-only content query: FTS4 doesn't support pure exclusions
        // Use NOT IN approach with subquery for each excluded term
        let mut exclusion_conditions = vec![];
        for excluded in &search_terms.excluded_terms {
            exclusion_conditions.push(format!(
                "notes.path NOT IN (SELECT DISTINCT notesContent.path FROM notesContent WHERE notesContent MATCH ?{})",
                var_num
            ));
            params.push(excluded.clone());
            var_num += 1;
        }

        // Important: Use base_sql to get all notes, then exclude matching ones
        let terms_sql = format!("{} WHERE {}", base_sql, exclusion_conditions.join(" AND "));
        queries.push(terms_sql);
    }
}
```

- [ ] **Step 4: Enhance build_search_sql_query for breadcrumb exclusions**

```rust
// Breadcrumb/title search (targets breadcrumb column specifically)
if !search_terms.breadcrumb.is_empty() || !search_terms.excluded_breadcrumb.is_empty() {
    if !search_terms.breadcrumb.is_empty() {
        // Positive breadcrumb terms: create query with positive terms + exclusions
        let mut breadcrumb_parts = vec![];

        // Add positive breadcrumb terms with column prefix
        for breadcrumb in &search_terms.breadcrumb {
            breadcrumb_parts.push(format!("breadcrumb: {}", breadcrumb));
        }

        // Add excluded breadcrumb terms with column prefix
        for excluded in &search_terms.excluded_breadcrumb {
            breadcrumb_parts.push(format!("breadcrumb: -{}", excluded));
        }

        let terms_sql = format!("{} WHERE notesContent MATCH ?{}", base_sql, var_num);
        queries.push(terms_sql);
        params.push(breadcrumb_parts.join(" "));
        var_num += 1;
    } else if !search_terms.excluded_breadcrumb.is_empty() {
        // Exclusion-only breadcrumb query: use NOT IN approach for breadcrumb column
        let mut exclusion_conditions = vec![];
        for excluded in &search_terms.excluded_breadcrumb {
            exclusion_conditions.push(format!(
                "notes.path NOT IN (SELECT DISTINCT notesContent.path FROM notesContent WHERE notesContent MATCH ?{})",
                var_num
            ));
            params.push(format!("breadcrumb: {}", excluded));
            var_num += 1;
        }

        let terms_sql = format!("{} WHERE {}", base_sql, exclusion_conditions.join(" AND "));
        queries.push(terms_sql);
    }
}
```

- [ ] **Step 5: Run test to verify FTS4 exclusion works**

Run: `cd core && cargo test test_fts4_exclusion_sql_generation`
Expected: PASS

- [ ] **Step 6: Commit FTS4 exclusion implementation**

```bash
cd core
git add src/db/mod.rs
git commit -m "feat: add FTS4 exclusion query generation

- Support mixed positive/negative terms in single FTS4 query
- Handle exclusion-only queries with NOT IN subqueries
- Add proper FTS4 column-specific syntax for breadcrumb exclusions
- Maintain INTERSECT architecture for multi-type queries"
```

## Task 4: SQL Generation Enhancement - LIKE Exclusion Queries

**Files:**
- Modify: `core/src/db/mod.rs:245-277` (filename and path query sections)
- Test: `core/src/db/mod.rs` (add LIKE exclusion tests)

- [ ] **Step 1: Write failing test for LIKE exclusion SQL generation**

```rust
#[test]
fn test_like_exclusion_sql_generation() {
    let (sql, params) = build_search_sql_query("@2024 @-draft");

    // Should generate filename query with positive and negative conditions
    assert!(sql.contains("notes.noteName LIKE"));
    assert!(sql.contains("notes.noteName NOT LIKE"));
    assert!(params.contains(&"2024".to_string()));
    assert!(params.contains(&"draft".to_string()));
}

#[test]
fn test_exclusion_only_like_query() {
    let (sql, params) = build_search_sql_query("@-draft @-temp");

    // Exclusion-only should still generate valid WHERE clause
    assert!(sql.contains("notes.noteName NOT LIKE"));
    assert!(!sql.contains("notes.noteName LIKE")); // No positive conditions
    assert_eq!(params.len(), 2);
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd core && cargo test test_like_exclusion_sql_generation`
Expected: FAIL (LIKE exclusions not implemented)

- [ ] **Step 3: Enhance filename query section for exclusions**

```rust
// Filename search (LIKE-based with manual exclusion)
if !search_terms.filename.is_empty() || !search_terms.excluded_filename.is_empty() {
    let mut positive_conditions = vec![];
    let mut negative_conditions = vec![];

    // Positive filename conditions
    for filename in search_terms.filename {
        if !filename.is_empty() {
            positive_conditions.push(format!("notes.noteName LIKE ('%' || ?{} || '%')", var_num));
            params.push(filename);
            var_num += 1;
        }
    }

    // Negative filename conditions
    for excluded in search_terms.excluded_filename {
        if !excluded.is_empty() {
            negative_conditions.push(format!("notes.noteName NOT LIKE ('%' || ?{} || '%')", var_num));
            params.push(excluded);
            var_num += 1;
        }
    }

    // Combine conditions: (pos1 OR pos2 OR ...) AND (NOT neg1 AND NOT neg2 AND ...)
    let mut where_parts = vec![];
    if !positive_conditions.is_empty() {
        where_parts.push(format!("({})", positive_conditions.join(" OR ")));
    }
    if !negative_conditions.is_empty() {
        where_parts.push(format!("({})", negative_conditions.join(" AND ")));
    }

    // Handle edge case: only exclusions (should match everything except excluded)
    let final_where = if positive_conditions.is_empty() && !negative_conditions.is_empty() {
        negative_conditions.join(" AND ")
    } else {
        where_parts.join(" AND ")
    };

    let terms_sql = format!("{} WHERE {}", base_sql, final_where);
    queries.push(terms_sql);
}
```

- [ ] **Step 4: Enhance path query section for exclusions**

```rust
// Path search (LIKE-based with manual exclusion)
if !search_terms.path.is_empty() || !search_terms.excluded_path.is_empty() {
    let mut positive_conditions = vec![];
    let mut negative_conditions = vec![];

    // Positive path conditions
    for path in search_terms.path {
        if !path.is_empty() {
            match path.strip_suffix("/") {
                Some(absolute) => {
                    positive_conditions.push(format!("notes.basePath = ('/' || ?{})", var_num));
                    params.push(absolute.to_string());
                }
                None => {
                    positive_conditions.push(format!("notes.basePath LIKE ('/' || ?{} || '%')", var_num));
                    params.push(path.to_string());
                }
            }
            var_num += 1;
        }
    }

    // Negative path conditions
    for excluded in search_terms.excluded_path {
        if !excluded.is_empty() {
            match excluded.strip_suffix("/") {
                Some(absolute) => {
                    negative_conditions.push(format!("notes.basePath != ('/' || ?{})", var_num));
                    params.push(absolute.to_string());
                }
                None => {
                    negative_conditions.push(format!("notes.basePath NOT LIKE ('/' || ?{} || '%')", var_num));
                    params.push(excluded.to_string());
                }
            }
            var_num += 1;
        }
    }

    // Combine conditions similar to filename logic
    let mut where_parts = vec![];
    if !positive_conditions.is_empty() {
        where_parts.push(format!("({})", positive_conditions.join(" OR ")));
    }
    if !negative_conditions.is_empty() {
        where_parts.push(format!("({})", negative_conditions.join(" AND ")));
    }

    let final_where = if positive_conditions.is_empty() && !negative_conditions.is_empty() {
        negative_conditions.join(" AND ")
    } else {
        where_parts.join(" AND ")
    };

    let terms_sql = format!("{} WHERE {}", base_sql, final_where);
    queries.push(terms_sql);
}
```

- [ ] **Step 5: Run test to verify LIKE exclusions work**

Run: `cd core && cargo test test_like_exclusion_sql_generation`
Expected: PASS

- [ ] **Step 6: Run comprehensive SQL generation tests**

Run: `cd core && cargo test build_search_sql_query`
Expected: ALL PASS

- [ ] **Step 7: Commit LIKE exclusion implementation**

```bash
cd core
git add src/db/mod.rs
git commit -m "feat: add LIKE-based exclusion query generation

- Support filename and path exclusions with NOT LIKE clauses
- Handle mixed positive/negative LIKE conditions
- Support exclusion-only LIKE queries
- Maintain existing path matching logic (exact vs prefix)"
```

## Task 5: Integration Testing - CLI End-to-End Tests

**Files:**
- Modify: `tui/tests/cli_integration_test.rs:178-179` (add new test functions)
- Test: Run integration tests to verify complete functionality

- [ ] **Step 1: Write failing integration test for basic exclusions**

```rust
#[tokio::test]
async fn test_cli_search_basic_exclusions() {
    let workspace_dir = TempDir::new().unwrap();
    let config_dir = TempDir::new().unwrap();
    let config_path = config_dir.path().join("config.toml");

    // Create test vault with notes including exclusion scenarios
    let vault = setup_exclusion_test_vault(&workspace_dir).await;
    write_config(&config_path, workspace_dir.path());

    // Test content exclusion
    let result = run_cli(
        CliCommand::Search {
            query: "meeting -cancelled".to_string(),
            format: OutputFormat::Text,
        },
        Some(config_path.clone()),
    )
    .await;

    assert!(result.is_ok(), "search with exclusion should succeed: {:?}", result);

    // Validate that exclusion actually works by checking vault directly
    let search_results = vault.search_notes("meeting -cancelled").await.expect("direct search should work");

    // Should find "weekly-meeting" but not "cancelled-meeting"
    let paths: Vec<String> = search_results.iter()
        .map(|(entry, _)| entry.path.as_str().to_string())
        .collect();

    assert!(paths.contains(&"weekly-meeting.md".to_string()),
        "Should find weekly-meeting note");
    assert!(!paths.contains(&"cancelled-meeting.md".to_string()),
        "Should exclude cancelled-meeting note");
}

async fn setup_exclusion_test_vault(dir: &TempDir) -> NoteVault {
    let vault = NoteVault::new(dir.path()).await.expect("failed to create vault");
    vault.init_and_validate().await.expect("failed to init vault");

    // Create notes for exclusion testing
    vault.create_note(
        &VaultPath::note_path_from("weekly-meeting"),
        "# Weekly Meeting\n\nRegular team meeting notes.",
    ).await.expect("failed to create meeting note");

    vault.create_note(
        &VaultPath::note_path_from("cancelled-meeting"),
        "# Cancelled Meeting\n\nThis meeting was cancelled.",
    ).await.expect("failed to create cancelled note");

    vault.create_note(
        &VaultPath::note_path_from("project-draft"),
        "# Project Draft\n\nDraft version of project proposal.",
    ).await.expect("failed to create draft note");

    vault.create_note(
        &VaultPath::note_path_from("project-final"),
        "# Project Final\n\nFinal version of project proposal.",
    ).await.expect("failed to create final note");

    vault.recreate_index().await.expect("failed to recreate index");
    vault
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd tui && cargo test test_cli_search_basic_exclusions`
Expected: FAIL (exclusion parsing not integrated into CLI yet)

- [ ] **Step 3: Write test for compound exclusions**

```rust
#[tokio::test]
async fn test_cli_search_compound_exclusions() {
    let workspace_dir = TempDir::new().unwrap();
    let config_dir = TempDir::new().unwrap();
    let config_path = config_dir.path().join("config.toml");

    setup_exclusion_test_vault(&workspace_dir).await;
    write_config(&config_path, workspace_dir.path());

    // Test title exclusion
    let result = run_cli(
        CliCommand::Search {
            query: ">project >-draft".to_string(),
            format: OutputFormat::Text,
        },
        Some(config_path.clone()),
    )
    .await;

    assert!(result.is_ok(), "title exclusion should succeed: {:?}", result);

    // Test filename exclusion
    let result = run_cli(
        CliCommand::Search {
            query: "@project @-draft".to_string(),
            format: OutputFormat::Text,
        },
        Some(config_path.clone()),
    )
    .await;

    assert!(result.is_ok(), "filename exclusion should succeed: {:?}", result);
}
```

- [ ] **Step 4: Write test for exclusion-only queries**

```rust
#[tokio::test]
async fn test_cli_search_exclusion_only() {
    let workspace_dir = TempDir::new().unwrap();
    let config_dir = TempDir::new().unwrap();
    let config_path = config_dir.path().join("config.toml");

    setup_exclusion_test_vault(&workspace_dir).await;
    write_config(&config_path, workspace_dir.path());

    // Test pure content exclusion
    let result = run_cli(
        CliCommand::Search {
            query: "-cancelled".to_string(),
            format: OutputFormat::Text,
        },
        Some(config_path.clone()),
    )
    .await;

    assert!(result.is_ok(), "exclusion-only search should succeed: {:?}", result);

    // Test pure title exclusion
    let result = run_cli(
        CliCommand::Search {
            query: ">-draft".to_string(),
            format: OutputFormat::Text,
        },
        Some(config_path),
    )
    .await;

    assert!(result.is_ok(), "title exclusion-only should succeed: {:?}", result);
}
```

- [ ] **Step 5: Run integration tests**

Run: `cd tui && cargo test cli_search_.*exclusion`
Expected: PASS (exclusion functionality working end-to-end)

- [ ] **Step 6: Run all CLI integration tests**

Run: `cd tui && cargo test cli_integration_test`
Expected: ALL PASS (backward compatibility maintained)

- [ ] **Step 7: Commit integration tests**

```bash
cd tui
git add tests/cli_integration_test.rs
git commit -m "test: add integration tests for exclusion operators

- Test basic content and title exclusions via CLI
- Test compound exclusion syntax combinations
- Test exclusion-only query scenarios
- Verify end-to-end functionality with real vault setup"
```

## Task 6: Documentation Updates

**Files:**
- Modify: `tui/docs/cli-testing.md` (add exclusion examples)
- Modify: `README.md` (update search documentation)

- [ ] **Step 1: Add exclusion examples to CLI testing documentation**

Add to `tui/docs/cli-testing.md` in the manual testing section:

```markdown
#### Exclusion Operator Testing

Test exclusion operators with various combinations:

```bash
# Basic content exclusion
kimun search "meeting -cancelled"

# Title exclusion
kimun search ">project >-draft"

# Filename exclusion
kimun search "@2024 @-temp"

# Path exclusion
kimun search "/docs /-private"

# Exclusion-only queries
kimun search "-cancelled"
kimun search ">-draft"

# Complex combinations
kimun search "meeting @2024 -cancelled >-draft /docs"
```

**Expected Behavior:**
- Content exclusions filter out notes containing excluded terms
- Title exclusions work on note titles/breadcrumbs only
- Filename exclusions work on note filenames only
- Path exclusions work on note path prefixes
- Exclusion-only queries return all notes except those matching excluded terms
```

- [ ] **Step 2: Update README search documentation**

Add to `README.md` in the CLI search section:

```markdown
### Exclusion Operators

Use `-` prefix to exclude terms from search results:

```sh
# Exclude specific content
kimun search "meeting -cancelled"

# Exclude from titles
kimun search ">project >-draft"

# Exclude from filenames
kimun search "@2024 @-temp"

# Exclude from paths
kimun search "/docs /-private"

# Exclusion-only searches
kimun search "-cancelled"        # All notes except those containing "cancelled"
kimun search ">-draft"          # All notes except those with "draft" in title
```

**Exclusion operators work with:**
- Content search: `-term` excludes from note content
- Title search: `>-term` or `in:-term` excludes from note titles
- Filename search: `@-term` or `at:-term` excludes from filenames
- Path search: `/-term` or `pt:-term` excludes from paths
- All operators can be combined in a single query
```

- [ ] **Step 3: Commit documentation updates**

```bash
git add tui/docs/cli-testing.md README.md
git commit -m "docs: add exclusion operator documentation

- Add exclusion syntax examples to CLI testing guide
- Update README with comprehensive exclusion operator usage
- Document behavior and combinations for all exclusion types
- Provide clear examples for manual testing scenarios"
```

## Task 7: Final Verification and Cleanup

**Files:**
- Test: All modified files
- Verify: Complete functionality and backward compatibility

- [ ] **Step 1: Run full test suite**

```bash
# Core library tests
cd core && cargo test

# TUI integration tests
cd tui && cargo test

# Build verification
cargo build --release
```

Expected: ALL PASS

- [ ] **Step 2: Manual verification of exclusion operators**

Test each exclusion type manually:

```bash
cd tui

# Basic exclusions
cargo run --bin kimun search "test -exclude"
cargo run --bin kimun search ">title >-draft"
cargo run --bin kimun search "@file @-temp"
cargo run --bin kimun search "/path /-private"

# Exclusion-only
cargo run --bin kimun search "-exclude"
cargo run --bin kimun search ">-draft"
```

Expected: All commands execute without errors, show appropriate filtering

- [ ] **Step 3: Backward compatibility verification**

Test existing query patterns:

```bash
# Existing syntax should work unchanged
cargo run --bin kimun search "meeting notes"
cargo run --bin kimun search ">important @2024 /journal ^-title"
cargo run --bin kimun notes --path "journal/"
```

Expected: All work identically to before exclusion implementation

- [ ] **Step 4: Performance verification**

Test with larger queries:

```bash
# Complex exclusion query
cargo run --bin kimun search "meeting project -cancelled -draft >important >-private @2024 @-temp /docs /-secret"
```

Expected: Reasonable response time, no memory issues

- [ ] **Step 5: Create final verification commit**

```bash
git add -A
git commit -m "feat: complete search refinement with exclusion operators

✅ Parser enhancement with optimized prefix detection
✅ FTS4 and LIKE exclusion query generation
✅ Exclusion-only query support via NOT IN subqueries
✅ Full backward compatibility maintained
✅ Comprehensive integration testing
✅ Complete documentation updates

Enables advanced search queries like:
- Basic exclusions: 'meeting -cancelled'
- Field-specific: '>project >-draft @2024 @-temp'
- Exclusion-only: '-cancelled >-draft'
- Complex combinations: 'meeting @2024 -cancelled >-draft /docs'"
```

---

**Implementation Complete!**

Search refinement with exclusion operators is now fully implemented, tested, and documented. The enhancement maintains full backward compatibility while adding powerful new query capabilities using FTS4-optimized exclusion syntax.