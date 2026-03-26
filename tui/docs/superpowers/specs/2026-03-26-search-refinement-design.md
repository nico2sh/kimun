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
- Extend `ElementType` enum with exclusion variants
- Modify `extract_and_consume` to handle `-` prefixed terms
- Categorize terms into positive/negative collections during parsing

### Database Integration (`core/src/db/mod.rs`)

**FTS4-Aware Query Building:**
- **Content/Title Search:** Combine positive and negative terms into FTS4 expressions
- **Filename/Path Search:** Generate separate `NOT LIKE` clauses for exclusions
- **Query Combination:** Use existing `INTERSECT` approach to combine results

**Enhanced `build_search_sql_query` function:**
```rust
// For FTS4 queries (terms, breadcrumb)
if !search_terms.terms.is_empty() || !search_terms.excluded_terms.is_empty() {
    let mut fts_terms = search_terms.terms.clone();
    for excluded in &search_terms.excluded_terms {
        fts_terms.push(format!("-{}", excluded));
    }
    // Use: notesContent MATCH ?
    // With: "meeting project -cancelled"
}

// For LIKE queries (filename, path)
if !search_terms.filename.is_empty() || !search_terms.excluded_filename.is_empty() {
    let mut conditions = vec![];

    // Positive conditions
    for filename in search_terms.filename {
        conditions.push("notes.noteName LIKE ('%' || ? || '%')");
    }

    // Negative conditions
    for excluded in search_terms.excluded_filename {
        conditions.push("notes.noteName NOT LIKE ('%' || ? || '%')");
    }

    // Combine: WHERE (positive OR positive) AND (NOT negative AND NOT negative)
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
   - **Content FTS:** `notesContent MATCH "meeting -cancelled"`
   - **Filename LIKE:** `notes.noteName LIKE '%2024%'`
   - **Title FTS:** `notesContent.breadcrumb MATCH "-draft"`

3. **Combine with INTERSECT:**
   ```sql
   (SELECT ... WHERE notesContent MATCH "meeting -cancelled")
   INTERSECT
   (SELECT ... WHERE notes.noteName LIKE '%2024%')
   INTERSECT
   (SELECT ... WHERE notesContent.breadcrumb MATCH "-draft")
   ```

### FTS4 Integration Details

**Native FTS4 Exclusion:**
- FTS4 supports `-term` syntax natively for efficient exclusion
- Exclusions are processed at index level (faster than post-filtering)
- Works with phrase matching: `-"cancelled meeting"`

**Query Construction:**
- Combine positive and negative terms: `["meeting", "project", "-cancelled"]` → `"meeting project -cancelled"`
- FTS4 handles operator precedence and optimization automatically
- Maintains compatibility with existing FTS4 features

**Fallback for Complex Cases:**
- If FTS4 query becomes malformed, gracefully fall back to positive-only search
- Log warnings for debugging but maintain user experience

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

**Guaranteed Compatibility:**
- All existing queries produce identical results
- No changes to CLI command syntax or TUI search interface
- Parser handles existing edge cases exactly as before
- Performance characteristics remain the same for non-exclusion queries

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

**Benchmarking Targets:**
- Small vaults (<1000 notes): Sub-100ms response
- Medium vaults (1000-10000 notes): Sub-500ms response
- Large vaults (10000+ notes): Sub-2s response

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
- Multiple exclusions like `-a -b -c -d -e` scale linearly
- FTS4 handles complex boolean expressions efficiently
- Practical limit: ~50 exclusion terms before performance degradation

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