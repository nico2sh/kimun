// tui/tests/note_path_resolution_test.rs
//
// Unit tests for CLI note path resolution helpers.

use kimun_notes::cli::helpers::resolve_note_path;

// resolve_note_path: relative path joined with quick_note_path
#[test]
fn test_relative_path_joined_with_quick_note_path() {
    let path = resolve_note_path("my-note", "/inbox").unwrap();
    assert_eq!(path.to_string(), "/inbox/my-note.md");
}

// resolve_note_path: relative path without extension gets .md
#[test]
fn test_relative_path_no_extension_gets_md() {
    let path = resolve_note_path("ideas/thing", "/notes").unwrap();
    assert_eq!(path.to_string(), "/notes/ideas/thing.md");
}

// resolve_note_path: explicit .md extension is not doubled
#[test]
fn test_explicit_md_extension_not_doubled() {
    let path = resolve_note_path("my-note.md", "/inbox").unwrap();
    assert_eq!(path.to_string(), "/inbox/my-note.md");
}

// resolve_note_path: absolute path (leading PATH_SEPARATOR) ignores quick_note_path
#[test]
fn test_absolute_path_ignores_quick_note_path() {
    let path = resolve_note_path("/projects/idea", "/inbox").unwrap();
    assert_eq!(path.to_string(), "/projects/idea.md");
}

// resolve_note_path: absolute path with .md is not doubled
#[test]
fn test_absolute_path_with_md_not_doubled() {
    let path = resolve_note_path("/projects/idea.md", "/inbox").unwrap();
    assert_eq!(path.to_string(), "/projects/idea.md");
}

// resolve_note_path: empty string returns error
#[test]
fn test_empty_path_returns_error() {
    let result = resolve_note_path("", "/inbox");
    assert!(result.is_err(), "empty path should return an error");
}

// resolve_note_path: whitespace-only returns error
#[test]
fn test_whitespace_only_path_returns_error() {
    let result = resolve_note_path("   ", "/inbox");
    assert!(result.is_err(), "whitespace-only path should return an error");
}

// resolve_note_path: quick_note_path at root
#[test]
fn test_quick_note_path_root_default() {
    let path = resolve_note_path("my-note", "/").unwrap();
    assert_eq!(path.to_string(), "/my-note.md");
}

// resolve_note_path: empty quick_note_path uses root
#[test]
fn test_empty_quick_note_path_uses_root() {
    let path = resolve_note_path("my-note", "").unwrap();
    assert_eq!(path.to_string(), "/my-note.md");
}

// resolve_note_path: bare separator returns error
#[test]
fn test_root_separator_only_returns_error() {
    let result = resolve_note_path("/", "/inbox");
    assert!(result.is_err(), "bare separator should return an error");
}
