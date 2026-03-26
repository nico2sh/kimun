# Search Refinement with Exclusion Operators Design

**Date:** March 26, 2026
**Feature:** CLI Phase 2 - Enhanced Search with Exclusion Operators
**Status:** Design Phase

## Overview

This design enhances Kimun's existing search capabilities by adding exclusion operators using `-` prefixes. The enhancement extends the current sophisticated query syntax (`>title`, `@filename`, `/path`, ordering) with exclusion support across all search categories, enabling queries like `meeting -cancelled`, `@2024 @-draft`, and `/journal >-private -todo`.

## Goals

- **Extend existing search syntax** with `-` exclusion operators for all search types
- **Maintain backward compatibility** - all current queries continue working identically
- **Leverage FTS4 capabilities** - use native FTS exclusion for efficient content/title filtering
- **Unified CLI/TUI support** - enhancement works seamlessly across both interfaces
- **Robust error handling** - graceful degradation for malformed queries

## Architecture & Components

### Parser Extension (`core/src/db/search_terms.rs`)

**Enhanced QueryTermExtractor:**
- Detect `-` prefixes for all search types: `-term`, `>-title`, `@-filename`, `/-path`
- Support quoted exclusions: `@-"draft notes"`, `>-'cancelled meeting'`
- Maintain existing parsing logic for positive terms

**Extended SearchTerms struct:**
```rust
#[derive(Default, Debug)]
pub struct SearchTerms {
    // Existing positive collections
    pub terms: Vec<String>,
    pub breadcrumb: Vec<String>,
    pub filename: Vec<String>,
    pub path: Vec<String>,
    pub order_by: Vec<OrderBy>,

    // New exclusion collections
    pub excluded_terms: Vec<String>,
    pub excluded_breadcrumb: Vec<String>,
    pub excluded_filename: Vec<String>,
    pub excluded_path: Vec<String>,
}
```

**Parsing Logic Enhancement:**

**Extended ElementType enum:**
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

**Enhanced `extract_and_consume` logic:**
```rust
impl QueryTermExtractor {
    fn extract_and_consume<S: AsRef<str>>(query: S) -> QueryTermExtractor {
        let query = query.as_ref().trim();

        // Check for exclusion prefixes first (compound prefixes)
        if query.starts_with("in:-") {
            // Handle in:-title (excluded breadcrumb)
            let remaining = query.strip_prefix("in:-").unwrap();
            return Self::parse_term(ElementType::ExcludedIn, remaining);
        }
        if query.starts_with(">-") {
            // Handle >-title (excluded breadcrumb)
            let remaining = query.strip_prefix(">-").unwrap();
            return Self::parse_term(ElementType::ExcludedIn, remaining);
        }

        if query.starts_with("at:-") {
            // Handle at:-filename (excluded filename)
            let remaining = query.strip_prefix("at:-").unwrap();
            return Self::parse_term(ElementType::ExcludedAt, remaining);
        }
        if query.starts_with("@-") {
            // Handle @-filename (excluded filename)
            let remaining = query.strip_prefix("@-").unwrap();
            return Self::parse_term(ElementType::ExcludedAt, remaining);
        }

        if query.starts_with("pt:-") {
            // Handle pt:-path (excluded path)
            let remaining = query.strip_prefix("pt:-").unwrap();
            return Self::parse_term(ElementType::ExcludedPath, remaining);
        }
        if query.starts_with("/-") {
            // Handle /-path (excluded path)
            let remaining = query.strip_prefix("/-").unwrap();
            return Self::parse_term(ElementType::ExcludedPath, remaining);
        }

        // Check for simple exclusion prefix
        if query.starts_with("-") && !query.starts_with("--") {
            // Handle -term (excluded content term)
            let remaining = query.strip_prefix("-").unwrap_or(query);
            // Avoid false positives: "-" alone or "- " should be treated as regular terms
            if !remaining.is_empty() && !remaining.starts_with(" ") {
                return Self::parse_term(ElementType::ExcludedTerm, remaining);
            }
        }

        // Existing prefix logic (in:, >, at:, @, pt:, /, ^:, ^)
        // ... (unchanged existing code) ...
    }
}
```

**Integration with Existing Parser Loop:**
The exclusion detection integrates with the existing `SearchTerms::from_query_string` loop:

```rust
impl SearchTerms {
    pub fn from_query_string<S: AsRef<str>>(query: S) -> Self {
        let mut query = query.as_ref().to_string();
        let mut breadcrumb = vec![];
        let mut excluded_breadcrumb = vec![];
        let mut terms = vec![];
        let mut excluded_terms = vec![];
        // ... other collections ...

        // Existing loop with enhanced element type handling
        while !query.is_empty() {
            let qp = QueryTermExtractor::extract_and_consume(query);
            query = qp.remainder;

            match qp.el_type {
                // Existing positive handlers
                ElementType::Term => terms.push(qp.term),
                ElementType::In => breadcrumb.push(qp.term),
                ElementType::At => filename.push(qp.term),
                ElementType::Path => path.push(qp.term),
                ElementType::OrderBy { asc } => {
                    if let Some(o) = OrderBy::from_term(&qp.term, asc) {
                        order_by.push(o);
                    }
                }
                // New exclusion handlers
                ElementType::ExcludedTerm => excluded_terms.push(qp.term),
                ElementType::ExcludedIn => excluded_breadcrumb.push(qp.term),
                ElementType::ExcludedAt => excluded_filename.push(qp.term),
                ElementType::ExcludedPath => excluded_path.push(qp.term),
                ElementType::Invalid => {}
            }
        }

        Self {
            breadcrumb,
            excluded_breadcrumb,
            filename,
            excluded_filename,
            order_by,
            path,
            excluded_path,
            terms,
            excluded_terms,
        }
    }
}
```

### Database Integration (`core/src/db/mod.rs`)

**FTS4 Table Structure:**
```sql
CREATE VIRTUAL TABLE notesContent USING fts4(path, breadcrumb, text)
```

**Current Query Building Pattern:**
- Each search type generates a separate `SELECT` query with `WHERE` clause
- All queries are combined using `INTERSECT`
- Parameters are properly bound using `?` placeholders

**Enhanced Query Building for Exclusions:**
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
        // Exclusion-only content query: match all then exclude
        let mut exclusion_parts = vec![];
        for excluded in &search_terms.excluded_terms {
            exclusion_parts.push(format!("-{}", excluded));
        }

        // Use FTS4 exclusion syntax - this will match documents that don't contain excluded terms
        let terms_sql = format!("{} WHERE notesContent MATCH ?{}", base_sql, var_num);
        queries.push(terms_sql);
        params.push(exclusion_parts.join(" "));
        var_num += 1;
    }
}

// Breadcrumb/title search (targets breadcrumb column specifically)
if !search_terms.breadcrumb.is_empty() || !search_terms.excluded_breadcrumb.is_empty() {
    if !search_terms.breadcrumb.is_empty() {
        // Positive breadcrumb terms: create query with positive terms + exclusions
        let mut fts_query_parts = vec![search_terms.breadcrumb.join(" ")];

        // Add excluded breadcrumb terms
        for excluded in &search_terms.excluded_breadcrumb {
            fts_query_parts.push(format!("-{}", excluded));
        }

        let terms_sql = format!("{} WHERE notesContent.breadcrumb MATCH ?{}", base_sql, var_num);
        queries.push(terms_sql);
        params.push(fts_query_parts.join(" "));
        var_num += 1;
    } else if !search_terms.excluded_breadcrumb.is_empty() {
        // Exclusion-only breadcrumb query: exclude from breadcrumb column
        let mut exclusion_parts = vec![];
        for excluded in &search_terms.excluded_breadcrumb {
            exclusion_parts.push(format!("-{}", excluded));
        }

        let terms_sql = format!("{} WHERE notesContent.breadcrumb MATCH ?{}", base_sql, var_num);
        queries.push(terms_sql);
        params.push(exclusion_parts.join(" "));
        var_num += 1;
    }
}

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

## Data Flow & Implementation

### Query Processing Pipeline

1. **Parse Query:** `"meeting @2024 -cancelled >-draft"`
   ```rust
   SearchTerms {
       terms: ["meeting"],
       filename: ["2024"],
       excluded_terms: ["cancelled"],
       excluded_breadcrumb: ["draft"],
       ...
   }
   ```

2. **Build SQL Components:**
   - **Content FTS (all columns):** `notesContent MATCH "meeting -cancelled"`
   - **Filename LIKE:** `notes.noteName LIKE '%2024%'`
   - **Title FTS (breadcrumb only):** `notesContent.breadcrumb MATCH "-draft"`

3. **Combine with INTERSECT:**
   ```sql
   (SELECT DISTINCT notes.path, title, size, modified, hash, noteName
    FROM notesContent JOIN notes ON notesContent.path = notes.path
    WHERE notesContent MATCH "meeting -cancelled")
   INTERSECT
   (SELECT DISTINCT notes.path, title, size, modified, hash, noteName
    FROM notesContent JOIN notes ON notesContent.path = notes.path
    WHERE notes.noteName LIKE '%2024%')
   INTERSECT
   (SELECT DISTINCT notes.path, title, size, modified, hash, noteName
    FROM notesContent JOIN notes ON notesContent.path = notes.path
    WHERE notesContent.breadcrumb MATCH "-draft")
   ```

### FTS4 Integration Details

**FTS4 Table Structure:**
- Virtual table: `notesContent(path, breadcrumb, text)`
- Content search (`terms`): Uses `notesContent MATCH` which searches ALL columns (path, breadcrumb, text)
- Title search (`breadcrumb`): Uses `notesContent.breadcrumb MATCH` which searches breadcrumb column only

**Native FTS4 Exclusion Behavior:**
- FTS4 supports `-term` syntax natively for efficient exclusion
- Exclusions are processed at index level (faster than post-filtering)
- Works with phrase matching: `-"cancelled meeting"`

**Critical Exclusion Scope:**
- **Content exclusions**: `notesContent MATCH "meeting -cancelled"` will exclude "cancelled" from ALL columns (path, breadcrumb, text), not just text content
- **Title exclusions**: `notesContent.breadcrumb MATCH "project -draft"` will exclude "draft" from breadcrumb column only
- **Exclusion-only queries**: `notesContent MATCH "-cancelled"` returns documents that don't contain "cancelled" in any column
- **Column-specific exclusion-only**: `notesContent.breadcrumb MATCH "-draft"` returns documents where breadcrumb doesn't contain "draft"

**Query Construction Examples:**
- Content exclusion: `"meeting project -cancelled"` (excludes cancelled from any column)
- Title exclusion: `"-draft"` on breadcrumb column (excludes draft from titles only)
- Mixed terms: Content query gets positive terms + exclusions, title query gets separate exclusions

**Exclusion-Only Query Handling:**
- `>-draft` (title exclusion only): `notesContent.breadcrumb MATCH "-draft"` returns all documents where breadcrumb doesn't contain "draft"
- `-cancelled` (content exclusion only): `notesContent MATCH "-cancelled"` returns all documents where no column contains "cancelled"
- Multiple exclusions: `>-draft >-private` becomes `notesContent.breadcrumb MATCH "-draft -private"`
- **INTERSECT behavior**: Exclusion-only queries generate valid queries that get intersected normally with other search types

**FTS4 Syntax Validation:**
- SQLite FTS4 returns empty results for malformed queries (no crashes)
- Common valid patterns: `"term1 term2 -excluded"`, `"-excluded1 -excluded2"`
- Pure exclusions: `"-excluded1 -excluded2"` (valid FTS4 syntax)
- Invalid patterns handled gracefully: malformed quotes, unrecognized operators

### Manual Exclusion for LIKE Queries

**Filename/Path Processing:**
- Generate separate positive (`LIKE`) and negative (`NOT LIKE`) conditions
- Combine using AND logic: `(positive1 OR positive2) AND (NOT negative1 AND NOT negative2)`
- Bind parameters separately to prevent SQL injection

**SQL Structure:**
```sql
SELECT ... WHERE
    (notes.noteName LIKE '%2024%' OR notes.noteName LIKE '%meeting%')
    AND
    (notes.noteName NOT LIKE '%draft%' AND notes.noteName NOT LIKE '%cancelled%')
```

## Error Handling & Edge Cases

### Query Parsing Robustness

**Malformed Exclusions:**
- `meeting -` (missing excluded term) → treat `-` as regular term, search for "meeting -"
- `>-` (empty title exclusion) → ignore the exclusion, continue with other terms
- Maintains current parsing behavior for edge cases

**Quoting Edge Cases:**
- `"meeting -cancelled"` → search for exact phrase "meeting -cancelled" (no exclusion)
- `-"cancelled meeting"` → exclude exact phrase "cancelled meeting"
- `>-'draft notes'` → exclude title matching "draft notes"
- Mismatched quotes handled by existing parser logic

**Invalid FTS4 Syntax:**
- SQLite FTS4 returns empty results for malformed queries rather than errors
- Log malformed FTS expressions for debugging
- Graceful degradation: return empty results instead of crashing

### Search Behavior Specifications

**Logical Edge Cases:**
- **All exclusions:** `-cancelled -todo -draft` → returns empty results (logically correct)
- **Non-existent exclusions:** `meeting -nonexistent` → same as `meeting` (no effect)
- **Duplicate terms:** `meeting -meeting` → returns empty results (logically consistent)

**Interaction with Existing Features:**
- **Ordering works:** `meeting -cancelled ^-title` → excludes cancelled, sorts by title descending
- **Path filtering:** `/journal meeting -cancelled` → search journal path, exclude cancelled
- **Combined syntax:** `>project @2024 -cancelled /-private` → complex multi-field exclusion

### Backward Compatibility

**Guaranteed Identical Behavior:**
- All existing search queries without `-` prefixes produce identical results
- No changes to CLI command syntax (`kimun search "query"`) or TUI search interface
- Performance characteristics remain the same for non-exclusion queries
- Existing error handling behavior preserved

**Edge Case Handling:**
- `meeting -` (trailing dash): Treated as search for literal "meeting -" (current behavior)
- `"meeting-cancelled"` (quoted with dash): Searches for exact phrase "meeting-cancelled" (no exclusion parsing)
- `-` (dash alone): Treated as literal search term (current behavior)
- `--term` (double dash): Treated as literal "--term" search (current behavior)

**Parser Precedence Rules:**
1. Quoted strings are parsed literally (no exclusion detection inside quotes)
2. Compound prefixes checked before simple prefixes (`>-` before `>` and `-`)
3. Exclusion prefixes require non-empty terms (`>-` alone is treated as literal `>-`)
4. Whitespace handling follows existing patterns

**Migration Safety:**
- No database schema changes required
- No breaking API changes in core library
- Existing integration tests continue passing without modification

## Performance Considerations

### FTS4 Exclusion Performance

**Advantages:**
- Index-level filtering (faster than post-processing)
- Native SQLite optimization for exclusion queries
- Scales well with vault size (O(log n) index lookups)

**Performance Expectations:**
- Small vaults (<1000 notes): Minimal impact over current search performance
- Medium vaults (1000-10000 notes): Should remain responsive for typical queries
- Large vaults (10000+ notes): Performance depends on exclusion complexity and result set size

**Optimization Strategies:**
- FTS4 exclusions are inherently efficient
- LIKE exclusions process smaller result sets (post-intersection)
- Existing query optimization (INTERSECT) minimizes data transfer

### Memory Usage

**Query Processing:**
- Minimal memory overhead for parsing exclusion terms
- FTS4 handles exclusion in SQLite memory space
- Result sets are typically smaller due to exclusions

**Large Exclusion Lists:**
- Multiple exclusions like `-a -b -c -d -e` scale with FTS4 query complexity
- FTS4 handles boolean expressions efficiently at the index level
- Performance testing needed to determine practical limits for exclusion count

## Testing Strategy

### Unit Testing (`search_terms.rs`)

**Parser Testing:**
```rust
#[test]
fn test_exclusion_syntax() {
    // Basic exclusions
    assert_exclusion_parsing("meeting -cancelled",
        terms: ["meeting"], excluded_terms: ["cancelled"]);

    // Field-specific exclusions
    assert_exclusion_parsing(">title >-draft",
        breadcrumb: ["title"], excluded_breadcrumb: ["draft"]);

    // Quoted exclusions
    assert_exclusion_parsing("-\"cancelled meeting\"",
        excluded_terms: ["cancelled meeting"]);

    // Mixed syntax
    assert_exclusion_parsing("@2024 @-draft /journal /-private meeting -cancelled",
        /* complex validation */);
}

#[test]
fn test_malformed_exclusions() {
    // Edge cases should not crash
    assert_no_panic("meeting -");
    assert_no_panic(">-");
    assert_no_panic("@-\"unclosed quote");
}

#[test]
fn test_backward_compatibility() {
    // All existing test cases must pass identically
    // Import from existing parser tests
}
```

### Database Integration Testing

**SQL Query Generation:**
```rust
#[test]
fn test_fts_exclusion_queries() {
    let (sql, params) = build_search_sql_query("meeting -cancelled");
    assert!(sql.contains("notesContent MATCH"));
    assert_eq!(params[0], "meeting -cancelled");
}

#[test]
fn test_like_exclusion_queries() {
    let (sql, params) = build_search_sql_query("@meeting @-draft");
    assert!(sql.contains("NOT LIKE"));
    assert!(params.contains(&"draft".to_string()));
}

#[test]
fn test_complex_combinations() {
    // Multi-field exclusions
    // Performance with large exclusion lists
    // Edge case SQL generation
}
```

### Integration Testing (`tui/tests/cli_integration_test.rs`)

**End-to-End CLI Testing:**
```rust
#[tokio::test]
async fn test_cli_search_exclusions() {
    let workspace_dir = setup_test_vault_with_exclusion_scenarios().await;

    // Test basic exclusion
    let result = run_cli(CliCommand::Search {
        query: "meeting -cancelled".to_string(),
        format: OutputFormat::Text
    }, None).await;

    assert!(result.is_ok());
    let output = capture_output(result);
    assert!(output.contains("weekly-meeting.md"));
    assert!(!output.contains("cancelled-meeting.md"));
}

#[tokio::test]
async fn test_exclusion_edge_cases() {
    // All-exclusion queries
    // Non-existent exclusions
    // Malformed syntax handling
}
```

### Performance Testing

**Benchmarking Suite:**
```rust
#[bench]
fn bench_exclusion_vs_normal_search(b: &mut Bencher) {
    // Compare performance impact of exclusion operators
    // Test with various vault sizes
    // Measure memory usage
}

#[bench]
fn bench_large_exclusion_lists(b: &mut Bencher) {
    // Test queries with many exclusions
    // Identify performance breaking points
}
```

### TUI Integration Testing

**Search Interface Testing:**
- Verify exclusion syntax works in TUI search box
- Test search result filtering in browse screen
- Ensure exclusion operators don't break search history
- Validate search highlighting with exclusions

## Implementation Plan

### Phase 1: Parser Enhancement
1. Extend `QueryTermExtractor` to detect exclusion prefixes
2. Add exclusion collections to `SearchTerms` struct
3. Implement exclusion parsing logic with existing patterns
4. Add comprehensive unit tests for parser changes

### Phase 2: Database Integration
1. Modify `build_search_sql_query` for FTS4 exclusion support
2. Implement LIKE exclusion logic for filename/path searches
3. Add database integration tests
4. Performance testing with representative data sets

### Phase 3: Testing & Validation
1. Run comprehensive test suite including regression tests
2. Manual testing of complex exclusion scenarios
3. Performance benchmarking on large vaults
4. CLI integration testing with new syntax

### Phase 4: Documentation & Release
1. Update CLI testing documentation with exclusion examples
2. Add exclusion syntax to README search documentation
3. Update TUI help text to mention exclusion operators
4. Create user-facing examples and tutorials

## Success Criteria

- ✅ All existing queries work identically (100% backward compatibility)
- ✅ Exclusion operators work across all search types (`-term`, `>-title`, `@-filename`, `/-path`)
- ✅ Complex combinations work: `meeting @2024 -cancelled >-draft /journal`
- ✅ Performance remains acceptable: <2s for large vaults with exclusions
- ✅ Error handling is robust: malformed queries don't crash, provide useful feedback
- ✅ Both CLI and TUI interfaces support exclusion operators seamlessly
- ✅ Comprehensive test coverage including edge cases and performance scenarios

## Future Enhancements

**Advanced Boolean Logic:**
- AND/OR operators: `(meeting OR standup) -cancelled`
- Grouping with parentheses: `+(meeting project) -(cancelled draft)`
- Field-specific boolean: `>+(important urgent) >-(draft archived)`

**Query Optimization:**
- Query result caching for repeated searches
- Index optimization for common exclusion patterns
- Smart query rewriting for performance

**User Experience:**
- Search suggestion/completion with exclusion syntax
- Visual highlighting of excluded terms in results
- Search history with exclusion patterns