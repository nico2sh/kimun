//! Query variables: `{name}` placeholders the TUI resolves to runtime
//! values before a query reaches core (see `CONTEXT.md` "Query variable"
//! and `adr/0003`). Core's query language never sees these.

use kimun_core::nfs::VaultPath;
use kimun_core::quote_query_term;

/// The current-note variable. A bare `<` typed in the query panel is sugar
/// that expands to `<{note}` (handled at the input layer, not here).
pub const VAR_NOTE: &str = "{note}";

/// True if `template` contains any query variable. The query panel uses
/// this to decide whether to re-run on note navigation.
pub fn query_has_variables(template: &str) -> bool {
    template.contains(VAR_NOTE)
}

/// Resolve all query variables in `template` against the open note,
/// producing a plain query string for `vault.search_notes`. `{note}`
/// becomes the note's clean name (matching how `<` targets are matched —
/// see ADR 0001), quoted when it contains whitespace so a multi-word name
/// stays a single query token. When no note is open, `{note}` resolves to
/// the empty string.
pub fn resolve_query(template: &str, current_note: Option<&VaultPath>) -> String {
    let note_name = current_note
        .map(|p| quote_query_term(&p.get_clean_name()))
        .unwrap_or_default();
    template.replace(VAR_NOTE, &note_name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_variables() {
        assert!(query_has_variables("<{note}"));
        assert!(query_has_variables("#todo <{note}"));
        assert!(!query_has_variables("#todo"));
    }

    #[test]
    fn resolves_note_variable() {
        let p = VaultPath::note_path_from("work/spec.md");
        assert_eq!(resolve_query("<{note}", Some(&p)), "<spec");
        assert_eq!(resolve_query("#todo <{note}", Some(&p)), "#todo <spec");
    }

    #[test]
    fn resolves_note_with_spaces_quoted() {
        let p = VaultPath::note_path_from("work/my note.md");
        // Multi-word name must be quoted so the parser sees one link target,
        // not `<my` plus a stray `note` term.
        assert_eq!(resolve_query("<{note}", Some(&p)), "<\"my note\"");
    }

    #[test]
    fn resolves_to_empty_without_note() {
        assert_eq!(resolve_query("<{note}", None), "<");
        assert_eq!(resolve_query("#todo", None), "#todo");
    }
}
