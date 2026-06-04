//! Query variables: `{name}` placeholders the TUI resolves to runtime
//! values before a query reaches core (see `CONTEXT.md` "Query variable"
//!). Core's query language never sees these.

use kimun_core::nfs::VaultPath;
use kimun_core::quote_query_term;

/// The current-note variable. A bare note operator (`<`, `>`, `=`) typed in
/// the query panel is sugar that expands to `<{note}` / `>{note}` / `={note}`
/// via [`expand_note_sugar`] before resolution.
pub const VAR_NOTE: &str = "{note}";

/// Note-related operators whose bare form (no target) is sugar for
/// `<op>{note}`: backlinks, forward links, and name match.
const NOTE_SUGAR_OPS: [&str; 3] = ["<", ">", "="];

/// Expand bare note operators into their `{note}` form: a whitespace-delimited
/// token that is exactly `<`, `>` or `=` (outside quotes) becomes `<{note}`,
/// `>{note}` or `={note}`. Operators with an explicit target (`<projects`)
/// and operators inside quoted terms are left untouched.
fn expand_note_sugar(template: &str) -> String {
    fn flush(token: &mut String, out: &mut String) {
        out.push_str(token);
        if NOTE_SUGAR_OPS.contains(&token.as_str()) {
            out.push_str(VAR_NOTE);
        }
        token.clear();
    }

    let mut out = String::with_capacity(template.len() + VAR_NOTE.len());
    let mut token = String::new();
    let mut quote: Option<char> = None;
    for c in template.chars() {
        match quote {
            Some(q) => {
                token.push(c);
                if c == q {
                    quote = None;
                }
            }
            None if c == '"' || c == '\'' => {
                quote = Some(c);
                token.push(c);
            }
            None if c.is_whitespace() => {
                flush(&mut token, &mut out);
                out.push(c);
            }
            None => token.push(c),
        }
    }
    flush(&mut token, &mut out);
    out
}

/// True if `template` contains any query variable (including the bare-operator
/// sugar forms). The query panel uses this to decide whether to re-run on note
/// navigation.
pub fn query_has_variables(template: &str) -> bool {
    expand_note_sugar(template).contains(VAR_NOTE)
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
    expand_note_sugar(template).replace(VAR_NOTE, &note_name)
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
