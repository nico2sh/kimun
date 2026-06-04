//! Query variables: `{name}` placeholders the TUI resolves to runtime
//! values before a query reaches core (see `CONTEXT.md` "Query variable"
//!). Core's query language never sees these.

use kimun_core::nfs::VaultPath;
use kimun_core::{expand_bare_note_prefixes, quote_query_term};

/// The current-note variable. A bare note operator (`<`, `>`, `=`, their long
/// forms and `-` exclusion variants) typed in the query panel is sugar for
/// `<op>{note}`, expanded by core's [`expand_bare_note_prefixes`] before
/// resolution so the DSL's tokenization is never re-implemented here.
pub const VAR_NOTE: &str = "{note}";

/// True if `template` contains any query variable (including the bare-operator
/// sugar forms). The query panel uses this to decide whether to re-run on note
/// navigation.
pub fn query_has_variables(template: &str) -> bool {
    expand_bare_note_prefixes(template, VAR_NOTE).contains(VAR_NOTE)
}

/// Resolve all query variables in `template` against the open note,
/// producing a plain query string for `vault.search_notes`. Bare note
/// operators are first expanded to their `{note}` form, then `{note}`
/// becomes the note's clean name (matching how `<` targets are
/// matched), quoted when it contains whitespace so a multi-word name
/// stays a single query token. When no note is open, `{note}` resolves
/// to the empty string.
pub fn resolve_query(template: &str, current_note: Option<&VaultPath>) -> String {
    let note_name = current_note
        .map(|p| quote_query_term(&p.get_clean_name()))
        .unwrap_or_default();
    expand_bare_note_prefixes(template, VAR_NOTE).replace(VAR_NOTE, &note_name)
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

    #[test]
    fn bare_operators_expand_to_note_variable() {
        let p = VaultPath::note_path_from("work/spec.md");
        assert_eq!(resolve_query("<", Some(&p)), "<spec");
        assert_eq!(resolve_query(">", Some(&p)), ">spec");
        assert_eq!(resolve_query("=", Some(&p)), "=spec");
        assert_eq!(resolve_query("#todo <", Some(&p)), "#todo <spec");
        assert_eq!(resolve_query("< #todo", Some(&p)), "<spec #todo");
    }

    #[test]
    fn bare_long_forms_and_exclusions_expand_too() {
        let p = VaultPath::note_path_from("work/spec.md");
        assert_eq!(resolve_query("lk:", Some(&p)), "lk:spec");
        assert_eq!(resolve_query("fwd:", Some(&p)), "fwd:spec");
        assert_eq!(resolve_query("name:", Some(&p)), "name:spec");
        assert_eq!(resolve_query("-<", Some(&p)), "-<spec");
    }

    #[test]
    fn apostrophe_in_term_does_not_suppress_expansion() {
        // A mid-token apostrophe is a literal character (matching core's
        // parser), not a quote opener, so sugar after a contraction still
        // expands.
        let p = VaultPath::note_path_from("work/spec.md");
        assert_eq!(resolve_query("don't <", Some(&p)), "don't <spec");
        assert_eq!(resolve_query("= don't <", Some(&p)), "=spec don't <spec");
    }

    #[test]
    fn operators_with_targets_stay_untouched() {
        let p = VaultPath::note_path_from("work/spec.md");
        assert_eq!(resolve_query("<projects", Some(&p)), "<projects");
        assert_eq!(resolve_query(">projects", Some(&p)), ">projects");
        assert_eq!(resolve_query("=projects", Some(&p)), "=projects");
    }

    #[test]
    fn bare_operator_inside_quotes_stays_untouched() {
        let p = VaultPath::note_path_from("work/spec.md");
        assert_eq!(resolve_query("\"a < b\"", Some(&p)), "\"a < b\"");
        assert_eq!(resolve_query("'a = b'", Some(&p)), "'a = b'");
    }

    #[test]
    fn bare_operators_count_as_variables() {
        assert!(query_has_variables("<"));
        assert!(query_has_variables(">"));
        assert!(query_has_variables("="));
        assert!(query_has_variables("#todo <"));
        assert!(!query_has_variables("<projects"));
        assert!(!query_has_variables("\"a < b\""));
    }
}
