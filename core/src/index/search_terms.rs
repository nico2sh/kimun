use std::vec;

use log::debug;

const ORDER_CHAR: &str = "^";
const ORDER_LETTER: &str = "or";

enum ElementType {
    Invalid,
    Term,
    In,
    At,
    Path,
    OrderBy { asc: bool },
    ExcludedTerm,
    ExcludedIn,
    ExcludedAt,
    ExcludedPath,
    Label,
    ExcludedLabel,
    Links,
    ExcludedLinks,
    ForwardLinks,
    ExcludedForwardLinks,
}

struct QueryTermExtractor {
    el_type: ElementType,
    term: String,
    remainder: String,
}

// Table of (long_prefix, short_prefix, element_type_tag) for non-special prefix types.
// Excluded variants must come before their positive counterparts so longer prefixes match first.
type PrefixEntry = (&'static str, &'static str, fn() -> ElementType);

fn prefix_table() -> [PrefixEntry; 12] {
    [
        ("-name:", "-=", || ElementType::ExcludedAt),
        ("-lk:", "-<", || ElementType::ExcludedLinks),
        ("-fwd:", "->", || ElementType::ExcludedForwardLinks),
        ("-in:", "-@", || ElementType::ExcludedIn),
        ("-pt:", "-/", || ElementType::ExcludedPath),
        ("-lb:", "-#", || ElementType::ExcludedLabel),
        ("name:", "=", || ElementType::At),
        ("lk:", "<", || ElementType::Links),
        ("fwd:", ">", || ElementType::ForwardLinks),
        ("in:", "@", || ElementType::In),
        ("pt:", "/", || ElementType::Path),
        ("lb:", "#", || ElementType::Label),
    ]
}

fn detect_prefix(query: &str) -> Option<(ElementType, &str)> {
    for (long, short, make_type) in prefix_table() {
        if let Some(remaining) = query
            .strip_prefix(long)
            .or_else(|| query.strip_prefix(short))
        {
            return Some((make_type(), remaining));
        }
    }
    None
}

impl QueryTermExtractor {
    fn extract_and_consume<S: AsRef<str>>(query: S) -> QueryTermExtractor {
        let query = query.as_ref().trim();

        let (element_type, remaining) = if let Some((el_type, remaining)) = detect_prefix(query) {
            (el_type, remaining.to_string())
        } else {
            // OrderBy must be checked before bare `-` so `-or:foo` and `-^foo`
            // are recognized as descending sorts, not excluded terms.
            let order_prefix = format!("{}:", ORDER_LETTER);
            let desc_order_prefix = format!("-{}:", ORDER_LETTER);
            let desc_order_char = format!("-{}", ORDER_CHAR);
            if let Some(rest) = query.strip_prefix(&desc_order_prefix) {
                (ElementType::OrderBy { asc: false }, rest.to_string())
            } else if let Some(rest) = query.strip_prefix(&order_prefix) {
                (ElementType::OrderBy { asc: true }, rest.to_string())
            } else if let Some(rest) = query.strip_prefix(&desc_order_char) {
                (ElementType::OrderBy { asc: false }, rest.to_string())
            } else if let Some(rest) = query.strip_prefix(ORDER_CHAR) {
                (ElementType::OrderBy { asc: true }, rest.to_string())
            } else if let Some(rest) = query.strip_prefix('-') {
                (ElementType::ExcludedTerm, rest.to_string())
            } else {
                (ElementType::Term, query.to_string())
            }
        };

        let (sep_char, mut term) = if remaining.starts_with('"') {
            ('"', remaining.chars().skip(1).collect())
        } else if remaining.starts_with("'") {
            ('\'', remaining.chars().skip(1).collect())
        } else {
            (' ', remaining)
        };

        match term.find(sep_char) {
            Some(pos) => {
                let mut remaining = term.split_off(pos);
                remaining = remaining
                    .strip_prefix(sep_char)
                    .map_or_else(|| remaining.trim().to_owned(), |s| s.trim().to_string());
                debug!("TERM: {}", term);
                debug!("REMAINING: {}", remaining);
                QueryTermExtractor {
                    el_type: element_type,
                    term,
                    remainder: remaining,
                }
            }
            None => {
                if sep_char == ' ' {
                    let term = term
                        .strip_suffix(sep_char)
                        .map_or_else(|| term.clone(), |s| s.to_string());
                    QueryTermExtractor {
                        el_type: element_type,
                        term,
                        remainder: String::new(),
                    }
                } else {
                    QueryTermExtractor {
                        el_type: ElementType::Invalid,
                        term: String::new(),
                        remainder: String::new(),
                    }
                }
            }
        }
    }
}

/// A parsed `or:`/`^` order directive: the column to sort by together with
/// its direction. Produced by the query parser when it encounters an order
/// token; [`OrderField`] is the direction-free counterpart used by callers
/// that carry the asc/desc choice separately.
#[derive(Debug)]
pub enum OrderBy {
    /// Sort by note title. `asc` is `true` for ascending, `false` for
    /// descending.
    Title {
        /// `true` to sort ascending, `false` to sort descending.
        asc: bool,
    },
    /// Sort by filename. `asc` is `true` for ascending, `false` for
    /// descending.
    FileName {
        /// `true` to sort ascending, `false` to sort descending.
        asc: bool,
    },
}

impl OrderBy {
    fn from_term(term: &str, asc: bool) -> Option<Self> {
        match term {
            "f" => Some(OrderBy::FileName { asc }),
            "file" => Some(OrderBy::FileName { asc }),
            "filename" => Some(OrderBy::FileName { asc }),
            "t" => Some(OrderBy::Title { asc }),
            "title" => Some(OrderBy::Title { asc }),
            _ => None,
        }
    }
}

/// The field a query can be ordered by. The asc/desc choice is carried
/// separately by callers; this names only the column.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrderField {
    /// Order results by note title.
    Title,
    /// Order results by filename.
    FileName,
}

/// True if `token` is an order directive in any of its four forms:
/// `or:<x>`, `-or:<x>`, `^<x>`, `-^<x>`. Allocation-free: strip an optional
/// leading `-`, then the rest must start with `^` or `or:`.
fn is_order_token(token: &str) -> bool {
    let rest = token.strip_prefix('-').unwrap_or(token);
    rest.starts_with(ORDER_CHAR)
        || rest
            .strip_prefix(ORDER_LETTER)
            .is_some_and(|after| after.starts_with(':'))
}

/// Wrap `term` in the search DSL's quote characters when it contains
/// whitespace, so a multi-word value (e.g. a note name with spaces) is
/// parsed as a single token instead of being split across terms. Values
/// without whitespace are returned unchanged. Note names can never contain
/// `"` (invalid on Windows/macOS/Linux filesystems alike), so no escaping
/// is needed.
pub fn quote_query_term(term: &str) -> String {
    if term.chars().any(char::is_whitespace) {
        format!("\"{term}\"")
    } else {
        term.to_string()
    }
}

/// True if `el` is a note-targeting element: backlinks, forward links, or
/// name match (including their excluded variants). These are the prefixes
/// whose bare form (no target) callers may expand to a current-note target.
fn is_note_element(el: &ElementType) -> bool {
    matches!(
        el,
        ElementType::Links
            | ElementType::ExcludedLinks
            | ElementType::ForwardLinks
            | ElementType::ExcludedForwardLinks
            | ElementType::At
            | ElementType::ExcludedAt
    )
}

/// Return `query` with every bare note-targeting prefix — `<` / `>` / `=`,
/// their long forms `lk:` / `fwd:` / `name:`, and the `-` exclusion variants —
/// expanded to `<prefix><target>`. A prefix is bare when the whole token is
/// exactly the prefix. Tokenization follows the parser's grammar: an unquoted
/// token ends at an ASCII space (only — a tab or NBSP is part of the token,
/// exactly as the parser reads it), a quote is honored only at a value start
/// (the start of a token or right after a prefix), and a quoted value may
/// span spaces. Everything else, including whitespace, is preserved verbatim,
/// so the result is the same query with only the bare prefixes rewritten.
///
/// This lives in core so the TUI's input-layer sugar (a bare `<` standing for
/// "backlinks of the current note") never re-implements the DSL's
/// tokenization.
pub fn expand_bare_note_prefixes(query: &str, target: &str) -> String {
    let mut out = String::with_capacity(query.len());
    let mut rest = query;
    while !rest.is_empty() {
        // Copy inter-token whitespace verbatim.
        let token_start = match rest.find(|c: char| !c.is_whitespace()) {
            Some(pos) => pos,
            None => {
                out.push_str(rest);
                break;
            }
        };
        out.push_str(&rest[..token_start]);
        rest = &rest[token_start..];

        // A prefix is only meaningful at the token start; the value may then
        // be quoted (and span spaces) or run to the next ASCII space — the
        // parser's separator (a tab or NBSP is part of the token).
        let detected = detect_prefix(rest);
        let prefix_len = detected
            .as_ref()
            .map_or(0, |(_, remaining)| rest.len() - remaining.len());
        let value = &rest[prefix_len..];
        let token_len = match value.chars().next() {
            Some(quote @ ('"' | '\'')) => {
                // Quoted value: token ends at the closing quote, or swallows
                // the rest of the string when unterminated (as the parser does).
                match value[quote.len_utf8()..].find(quote) {
                    Some(pos) => prefix_len + quote.len_utf8() * 2 + pos,
                    None => rest.len(),
                }
            }
            _ => rest.find(' ').unwrap_or(rest.len()),
        };
        let token = &rest[..token_len];
        out.push_str(token);
        // Bare prefix: the whole token is the prefix itself.
        if prefix_len == token_len {
            if let Some((el, _)) = detected {
                if is_note_element(&el) {
                    out.push_str(target);
                }
            }
        }
        rest = &rest[token_len..];
    }
    out
}

/// Return `query` with any order directive (`or:`/`-or:`/`^`/`-^`, in any
/// position) removed. Other tokens keep their order; whitespace is normalised
/// to single spaces. The DSL knowledge lives here in core so the TUI never
/// hardcodes the directive syntax.
pub fn strip_order_directive(query: &str) -> String {
    query
        .split_whitespace()
        .filter(|t| !is_order_token(t))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Return `query` with its order directive replaced by `field`/`asc`.
///
/// Any existing order directive is stripped (see [`strip_order_directive`]),
/// then the canonical `or:<field>` (ascending) / `-or:<field>` (descending)
/// directive is appended.
pub fn with_order_directive(query: &str, field: OrderField, asc: bool) -> String {
    let base = strip_order_directive(query);
    let field_term = match field {
        OrderField::Title => "title",
        OrderField::FileName => "file",
    };
    let directive = if asc {
        format!("{}:{}", ORDER_LETTER, field_term)
    } else {
        format!("-{}:{}", ORDER_LETTER, field_term)
    };
    if base.is_empty() {
        directive
    } else {
        format!("{} {}", base, directive)
    }
}

/// A search query string decomposed into the typed buckets the index turns
/// into an FTS query. Each prefix in the DSL routes a token into one of these
/// fields; bare tokens are full-text terms. This is the boundary the TUI
/// builds (often via [`with_order_directive`] / [`quote_query_term`]) and the
/// index consumes, so the DSL syntax lives entirely in core.
///
/// Prefix vocabulary (long form, then short form):
/// - `in:` / `@` — breadcrumb (any path segment / parent directory)
/// - `name:` / `=` — filename
/// - `pt:` / `/` — full path
/// - `lb:` / `#` — label (lowercased and deduplicated)
/// - `lk:` / `<` — backlinks (notes linking *to* the target)
/// - `fwd:` / `>` — forward links (notes the target links *to*)
/// - `or:` / `^` — order directive (`or:title`, `^file`, …)
///
/// Any prefix may be negated by a leading `-` (`-#draft`, `-lk:spec`) to
/// route the token into the matching `excluded_*` field. Values may be quoted
/// with `"` or `'` to include whitespace (e.g. `="my note"`); an unterminated
/// quote discards the token. Bare prefixes with no value are dropped.
#[derive(Default, Debug)]
pub struct SearchTerms {
    /// Bare full-text terms (no prefix). Matched against note content.
    pub terms: Vec<String>,
    /// `in:` / `@` values: matched against any path segment (breadcrumb).
    pub breadcrumb: Vec<String>,
    /// `or:` / `^` order directives, in the order they appeared.
    pub order_by: Vec<OrderBy>,
    /// `name:` / `=` values: matched against the filename.
    pub filename: Vec<String>,
    /// `pt:` / `/` values: matched against the full vault path.
    pub path: Vec<String>,
    /// `lb:` / `#` values: matched against labels. Lowercased and deduped.
    pub labels: Vec<String>,
    /// `lk:` / `<` values: notes that link *to* the named target (backlinks).
    /// Deduped, order preserved.
    pub links: Vec<String>,
    /// `fwd:` / `>` values: notes the named target links *to* (forward
    /// links). Deduped, order preserved.
    pub forward_links: Vec<String>,
    /// Negated bare terms (`-term`): content that must *not* match.
    pub excluded_terms: Vec<String>,
    /// Negated `in:` / `@` values (`-in:`, `-@`).
    pub excluded_breadcrumb: Vec<String>,
    /// Negated `name:` / `=` values (`-name:`, `-=`).
    pub excluded_filename: Vec<String>,
    /// Negated `pt:` / `/` values (`-pt:`, `-/`).
    pub excluded_path: Vec<String>,
    /// Negated `lb:` / `#` values (`-lb:`, `-#`). Lowercased and deduped.
    pub excluded_labels: Vec<String>,
    /// Negated `lk:` / `<` values (`-lk:`, `-<`). Deduped, order preserved.
    pub excluded_links: Vec<String>,
    /// Negated `fwd:` / `>` values (`-fwd:`, `->`). Deduped, order preserved.
    pub excluded_forward_links: Vec<String>,
}

/// Maximum byte length of a query string accepted by [`SearchTerms::from_query_string`].
/// 8 KB is more than enough for any real search query; larger inputs are truncated
/// on a char boundary to prevent unbounded memory allocation via duplicate labels.
const MAX_QUERY_LEN: usize = 8 * 1024;

impl SearchTerms {
    /// Parse a raw query string into typed [`SearchTerms`] buckets.
    ///
    /// Tokens are consumed left to right: a recognised prefix (see the
    /// [`SearchTerms`] docs) routes the value into its field, an optional
    /// leading `-` negates it, and anything else becomes a bare full-text
    /// term. Quoted values may span whitespace; empty values are dropped, and
    /// labels and link targets are deduplicated with order preserved. Inputs
    /// over `MAX_QUERY_LEN` are truncated on a char boundary.
    ///
    /// ```
    /// use kimun_core::SearchTerms;
    ///
    /// let st = SearchTerms::from_query_string("meeting #urgent -#draft @work");
    /// assert_eq!(st.terms, vec!["meeting"]);
    /// assert_eq!(st.labels, vec!["urgent"]);
    /// assert_eq!(st.excluded_labels, vec!["draft"]);
    /// assert_eq!(st.breadcrumb, vec!["work"]);
    /// ```
    pub fn from_query_string<S: AsRef<str>>(query: S) -> Self {
        let query_ref = query.as_ref();
        let query_ref = if query_ref.len() > MAX_QUERY_LEN {
            let mut idx = MAX_QUERY_LEN;
            while !query_ref.is_char_boundary(idx) {
                idx -= 1;
            }
            &query_ref[..idx]
        } else {
            query_ref
        };
        let mut query = query_ref.to_string();
        let mut breadcrumb = vec![];
        let mut terms = vec![];
        let mut filename = vec![];
        let mut order_by = vec![];
        let mut path = vec![];
        let mut labels = vec![];
        let mut links = vec![];
        let mut forward_links = vec![];
        let mut excluded_terms = vec![];
        let mut excluded_breadcrumb = vec![];
        let mut excluded_filename = vec![];
        let mut excluded_path = vec![];
        let mut excluded_labels = vec![];
        let mut excluded_links = vec![];
        let mut excluded_forward_links = vec![];
        while !query.is_empty() {
            let qp = QueryTermExtractor::extract_and_consume(query);
            query = qp.remainder;
            match qp.el_type {
                ElementType::Term => {
                    if !qp.term.is_empty() {
                        terms.push(qp.term);
                    }
                }
                ElementType::In => {
                    if !qp.term.is_empty() {
                        breadcrumb.push(qp.term);
                    }
                }
                ElementType::At => {
                    if !qp.term.is_empty() {
                        filename.push(qp.term);
                    }
                }
                ElementType::OrderBy { asc } => {
                    if let Some(o) = OrderBy::from_term(&qp.term, asc) {
                        order_by.push(o);
                    }
                }
                ElementType::Invalid => {}
                ElementType::Path => {
                    if !qp.term.is_empty() {
                        path.push(qp.term);
                    }
                }
                ElementType::Label => {
                    let n = qp.term.to_lowercase();
                    if !n.is_empty() {
                        labels.push(n);
                    }
                }
                ElementType::ExcludedTerm => {
                    if !qp.term.is_empty() {
                        excluded_terms.push(qp.term);
                    }
                }
                ElementType::ExcludedIn => {
                    if !qp.term.is_empty() {
                        excluded_breadcrumb.push(qp.term);
                    }
                }
                ElementType::ExcludedAt => {
                    if !qp.term.is_empty() {
                        excluded_filename.push(qp.term);
                    }
                }
                ElementType::ExcludedPath => {
                    if !qp.term.is_empty() {
                        excluded_path.push(qp.term);
                    }
                }
                ElementType::ExcludedLabel => {
                    let n = qp.term.to_lowercase();
                    if !n.is_empty() {
                        excluded_labels.push(n);
                    }
                }
                ElementType::Links => {
                    if !qp.term.is_empty() {
                        links.push(qp.term);
                    }
                }
                ElementType::ExcludedLinks => {
                    if !qp.term.is_empty() {
                        excluded_links.push(qp.term);
                    }
                }
                ElementType::ForwardLinks => {
                    if !qp.term.is_empty() {
                        forward_links.push(qp.term);
                    }
                }
                ElementType::ExcludedForwardLinks => {
                    if !qp.term.is_empty() {
                        excluded_forward_links.push(qp.term);
                    }
                }
            }
        }

        dedup_preserving_order(&mut labels);
        dedup_preserving_order(&mut excluded_labels);
        dedup_preserving_order(&mut links);
        dedup_preserving_order(&mut excluded_links);
        dedup_preserving_order(&mut forward_links);
        dedup_preserving_order(&mut excluded_forward_links);

        Self {
            breadcrumb,
            filename,
            order_by,
            terms,
            path,
            labels,
            links,
            forward_links,
            excluded_terms,
            excluded_breadcrumb,
            excluded_filename,
            excluded_path,
            excluded_labels,
            excluded_links,
            excluded_forward_links,
        }
    }
}

fn dedup_preserving_order(v: &mut Vec<String>) {
    let mut seen = std::collections::HashSet::new();
    v.retain(|x| seen.insert(x.clone()));
}

#[cfg(test)]
mod tests {
    use super::SearchTerms;
    use super::expand_bare_note_prefixes;

    #[test]
    fn expand_bare_short_note_prefixes() {
        assert_eq!(expand_bare_note_prefixes("<", "{note}"), "<{note}");
        assert_eq!(expand_bare_note_prefixes(">", "{note}"), ">{note}");
        assert_eq!(expand_bare_note_prefixes("=", "{note}"), "={note}");
        assert_eq!(expand_bare_note_prefixes("#todo <", "{note}"), "#todo <{note}");
        assert_eq!(expand_bare_note_prefixes("< #todo", "{note}"), "<{note} #todo");
    }

    #[test]
    fn expand_bare_long_note_prefixes() {
        assert_eq!(expand_bare_note_prefixes("lk:", "{note}"), "lk:{note}");
        assert_eq!(expand_bare_note_prefixes("fwd:", "{note}"), "fwd:{note}");
        assert_eq!(expand_bare_note_prefixes("name:", "{note}"), "name:{note}");
    }

    #[test]
    fn expand_bare_excluded_note_prefixes() {
        assert_eq!(expand_bare_note_prefixes("-<", "{note}"), "-<{note}");
        assert_eq!(expand_bare_note_prefixes("->", "{note}"), "->{note}");
        assert_eq!(expand_bare_note_prefixes("-=", "{note}"), "-={note}");
        assert_eq!(expand_bare_note_prefixes("-lk:", "{note}"), "-lk:{note}");
    }

    #[test]
    fn expand_leaves_prefixes_with_targets_untouched() {
        assert_eq!(expand_bare_note_prefixes("<projects", "{note}"), "<projects");
        assert_eq!(expand_bare_note_prefixes(">projects", "{note}"), ">projects");
        assert_eq!(expand_bare_note_prefixes("=projects", "{note}"), "=projects");
        assert_eq!(expand_bare_note_prefixes("lk:projects", "{note}"), "lk:projects");
        assert_eq!(
            expand_bare_note_prefixes("<\"my note\"", "{note}"),
            "<\"my note\""
        );
    }

    #[test]
    fn expand_leaves_non_note_prefixes_untouched() {
        assert_eq!(expand_bare_note_prefixes("@", "{note}"), "@");
        assert_eq!(expand_bare_note_prefixes("#", "{note}"), "#");
        assert_eq!(expand_bare_note_prefixes("/", "{note}"), "/");
        assert_eq!(expand_bare_note_prefixes("in:", "{note}"), "in:");
        assert_eq!(expand_bare_note_prefixes("term", "{note}"), "term");
    }

    #[test]
    fn expand_ignores_operators_inside_quoted_terms() {
        assert_eq!(
            expand_bare_note_prefixes("\"a < b\"", "{note}"),
            "\"a < b\""
        );
        assert_eq!(expand_bare_note_prefixes("'a = b'", "{note}"), "'a = b'");
    }

    #[test]
    fn expand_treats_mid_token_quotes_as_literal() {
        // An apostrophe inside a plain term is not a quote opener (matching the
        // parser, which only honors quotes at a value start), so a bare
        // operator after a contraction still expands.
        assert_eq!(
            expand_bare_note_prefixes("= don't <", "{note}"),
            "={note} don't <{note}"
        );
    }

    #[test]
    fn expand_preserves_whitespace_verbatim() {
        assert_eq!(
            expand_bare_note_prefixes("  #todo   <  ", "{note}"),
            "  #todo   <{note}  "
        );
    }

    #[test]
    fn expand_matches_parser_ascii_space_tokenization() {
        // The parser splits unquoted values on the ASCII space only — a NBSP
        // or tab is part of the token — so the expander must not treat one as
        // a token boundary and clobber a user-supplied target.
        assert_eq!(
            expand_bare_note_prefixes("<\u{a0}foo", "{note}"),
            "<\u{a0}foo"
        );
        assert_eq!(expand_bare_note_prefixes("a\t<", "{note}"), "a\t<");
    }

    #[test]
    fn expand_with_unterminated_quote() {
        // The first bare `<` expands; the unterminated quoted token swallows
        // the rest of the string (matching the parser) and stays untouched.
        assert_eq!(
            expand_bare_note_prefixes("< \"my no", "{note}"),
            "<{note} \"my no"
        );
    }

    #[test]
    fn search_terms() {
        let query = "some text more terms";
        let search_terms = SearchTerms::from_query_string(query);
        println!("{:?}", &search_terms);

        let breadcrumb = search_terms.breadcrumb;
        let filename = search_terms.filename;
        let path = search_terms.path;
        let terms = search_terms.terms;

        assert!(breadcrumb.is_empty());
        assert!(filename.is_empty());
        assert!(path.is_empty());
        assert!(!terms.is_empty());
        assert_eq!(4, terms.len());
        assert!(terms.contains(&"some".to_string()));
        assert!(terms.contains(&"text".to_string()));
        assert!(terms.contains(&"more".to_string()));
        assert!(terms.contains(&"terms".to_string()));
    }

    #[test]
    fn search_in() {
        let query = "@title in:othertitle";
        let search_terms = SearchTerms::from_query_string(query);
        println!("{:?}", &search_terms);

        let breadcrumb = search_terms.breadcrumb;
        let filename = search_terms.filename;
        let terms = search_terms.terms;

        assert!(!breadcrumb.is_empty());
        assert!(filename.is_empty());
        assert!(terms.is_empty());
        assert_eq!(2, breadcrumb.len());
        assert!(breadcrumb.contains(&"title".to_string()));
        assert!(breadcrumb.contains(&"othertitle".to_string()));
    }

    #[test]
    fn search_at() {
        let query = "=file name:directory";
        let search_terms = SearchTerms::from_query_string(query);
        println!("{:?}", &search_terms);

        let breadcrumb = search_terms.breadcrumb;
        let filename = search_terms.filename;
        let terms = search_terms.terms;

        assert!(breadcrumb.is_empty());
        assert!(!filename.is_empty());
        assert!(terms.is_empty());
        assert_eq!(2, filename.len());
        assert!(filename.contains(&"file".to_string()));
        assert!(filename.contains(&"directory".to_string()));
    }

    #[test]
    fn search_at_quoted() {
        let query = "='file name' name:\"directory path\"";
        let search_terms = SearchTerms::from_query_string(query);
        println!("{:?}", &search_terms);

        let breadcrumb = search_terms.breadcrumb;
        let filename = search_terms.filename;
        let terms = search_terms.terms;

        assert!(breadcrumb.is_empty());
        assert!(!filename.is_empty());
        assert!(terms.is_empty());
        assert_eq!(2, filename.len());
        assert!(filename.contains(&"file name".to_string()));
        assert!(filename.contains(&"directory path".to_string()));
    }

    #[test]
    fn search_at_quoted_not_closed() {
        let query = "='file name' name:\"directory path";
        let search_terms = SearchTerms::from_query_string(query);
        println!("{:?}", &search_terms);

        let breadcrumb = search_terms.breadcrumb;
        let filename = search_terms.filename;
        let path = search_terms.path;
        let terms = search_terms.terms;

        assert!(breadcrumb.is_empty());
        assert!(!filename.is_empty());
        assert!(terms.is_empty());
        assert!(path.is_empty());
        assert_eq!(1, filename.len());
        assert!(filename.contains(&"file name".to_string()));
    }

    #[test]
    fn search_combined() {
        let query = "searchterm    =file otherterm name:directory in:title @text      \"some text\" /basedirectory";
        let search_terms = SearchTerms::from_query_string(query);
        println!("{:?}", &search_terms);

        let breadcrumb = search_terms.breadcrumb;
        let filename = search_terms.filename;
        let terms = search_terms.terms;
        let path = search_terms.path;

        assert!(!breadcrumb.is_empty());
        assert!(!filename.is_empty());
        assert!(!terms.is_empty());
        assert!(!path.is_empty());
        assert_eq!(3, terms.len());
        assert!(terms.contains(&"searchterm".to_string()));
        assert!(terms.contains(&"otherterm".to_string()));
        assert!(terms.contains(&"some text".to_string()));
        assert_eq!(2, breadcrumb.len());
        assert!(breadcrumb.contains(&"title".to_string()));
        assert!(breadcrumb.contains(&"text".to_string()));
        assert_eq!(2, filename.len());
        assert!(filename.contains(&"file".to_string()));
        assert!(filename.contains(&"directory".to_string()));
        assert_eq!(1, path.len());
        assert!(path.contains(&"basedirectory".to_string()));
    }

    #[test]
    fn test_basic_exclusion_parsing() {
        // Test parsing basic exclusion syntax
        let search_terms = SearchTerms::from_query_string("meeting -cancelled");
        assert_eq!(search_terms.terms, vec!["meeting"]);
        // Note: excluded_terms field doesn't exist yet - test will fail compilation
        assert_eq!(search_terms.excluded_terms, vec!["cancelled"]);
        assert!(search_terms.breadcrumb.is_empty());
    }

    #[test]
    fn test_compound_exclusion_prefixes() {
        let search_terms = SearchTerms::from_query_string("-@draft -in:private -=temp -/secret");
        assert!(search_terms.terms.is_empty());
        assert!(search_terms.breadcrumb.is_empty());
        assert_eq!(search_terms.excluded_breadcrumb, vec!["draft", "private"]);
        assert_eq!(search_terms.excluded_filename, vec!["temp"]);
        assert_eq!(search_terms.excluded_path, vec!["secret"]);
    }

    #[test]
    fn search_links_short() {
        // `<` / `lk:` → backlinks (notes linking *to* target).
        let s = SearchTerms::from_query_string("<projects");
        assert_eq!(s.links, vec!["projects".to_string()]);
        assert!(s.terms.is_empty());
    }

    #[test]
    fn search_links_long() {
        let s = SearchTerms::from_query_string("lk:projects");
        assert_eq!(s.links, vec!["projects".to_string()]);
    }

    #[test]
    fn search_links_with_extension_and_path() {
        let s = SearchTerms::from_query_string("<work/projects.md");
        assert_eq!(s.links, vec!["work/projects.md".to_string()]);
    }

    #[test]
    fn search_links_excluded_short() {
        let s = SearchTerms::from_query_string("-<draft");
        assert_eq!(s.excluded_links, vec!["draft".to_string()]);
    }

    #[test]
    fn search_links_excluded_long() {
        let s = SearchTerms::from_query_string("-lk:draft");
        assert_eq!(s.excluded_links, vec!["draft".to_string()]);
    }

    #[test]
    fn search_links_mixed_with_term() {
        let s = SearchTerms::from_query_string("report <spec");
        assert_eq!(s.terms, vec!["report".to_string()]);
        assert_eq!(s.links, vec!["spec".to_string()]);
    }

    #[test]
    fn search_links_quoted() {
        let s = SearchTerms::from_query_string("<\"my note\"");
        assert_eq!(s.links, vec!["my note".to_string()]);
    }

    #[test]
    fn search_forward_links_short() {
        // `>` / `fwd:` → forward links (notes target links *to*).
        let s = SearchTerms::from_query_string(">spec");
        assert_eq!(s.forward_links, vec!["spec".to_string()]);
        assert!(s.terms.is_empty());
        assert!(s.links.is_empty());
    }

    #[test]
    fn search_forward_links_long() {
        let s = SearchTerms::from_query_string("fwd:spec");
        assert_eq!(s.forward_links, vec!["spec".to_string()]);
    }

    #[test]
    fn search_forward_links_excluded_short() {
        let s = SearchTerms::from_query_string("->draft");
        assert_eq!(s.excluded_forward_links, vec!["draft".to_string()]);
        assert!(s.excluded_links.is_empty());
    }

    #[test]
    fn search_forward_links_excluded_long() {
        let s = SearchTerms::from_query_string("-fwd:draft");
        assert_eq!(s.excluded_forward_links, vec!["draft".to_string()]);
    }

    #[test]
    fn search_backlinks_filename_section_chars() {
        // Confirm the remapped chars land in the right fields.
        assert_eq!(
            SearchTerms::from_query_string("<spec").links,
            vec!["spec".to_string()]
        );
        assert_eq!(
            SearchTerms::from_query_string("=file").filename,
            vec!["file".to_string()]
        );
        assert_eq!(
            SearchTerms::from_query_string("@title").breadcrumb,
            vec!["title".to_string()]
        );
    }

    #[test]
    fn search_label_short() {
        let s = SearchTerms::from_query_string("#important");
        assert_eq!(s.labels, vec!["important".to_string()]);
        assert!(s.terms.is_empty());
    }

    #[test]
    fn search_label_long() {
        let s = SearchTerms::from_query_string("lb:important");
        assert_eq!(s.labels, vec!["important".to_string()]);
    }

    #[test]
    fn search_label_case_normalized() {
        let s = SearchTerms::from_query_string("#Important");
        assert_eq!(s.labels, vec!["important".to_string()]);
    }

    #[test]
    fn search_label_excluded_short() {
        // Canonical excluded forms are `-#draft` and `-lb:draft`.
        let s2 = SearchTerms::from_query_string("-#draft");
        assert_eq!(s2.excluded_labels, vec!["draft".to_string()]);
        let s3 = SearchTerms::from_query_string("-lb:draft");
        assert_eq!(s3.excluded_labels, vec!["draft".to_string()]);
    }

    #[test]
    fn search_multiple_labels() {
        let s = SearchTerms::from_query_string("#a #b lb:c");
        let mut labels = s.labels.clone();
        labels.sort();
        assert_eq!(labels, vec!["a", "b", "c"]);
    }

    #[test]
    fn search_label_mixed_with_term() {
        let s = SearchTerms::from_query_string("meeting #important");
        assert_eq!(s.labels, vec!["important".to_string()]);
        assert_eq!(s.terms, vec!["meeting".to_string()]);
    }

    #[test]
    fn search_bare_hash_is_dropped() {
        let s = SearchTerms::from_query_string("#");
        assert!(s.labels.is_empty());
        assert!(s.terms.is_empty());
    }

    #[test]
    fn search_labels_are_deduped() {
        let s = SearchTerms::from_query_string("#foo #foo lb:foo #bar");
        assert_eq!(s.labels, vec!["foo".to_string(), "bar".to_string()]);
    }

    #[test]
    fn excluded_labels_are_deduped() {
        let s = SearchTerms::from_query_string("-#draft -lb:draft -#old");
        assert_eq!(
            s.excluded_labels,
            vec!["draft".to_string(), "old".to_string()]
        );
    }

    #[test]
    fn exclusion_short_forms_parse_to_excluded_fields() {
        // Locks the `prefix_table` ordering invariant (excluded-before-positive,
        // longer-before-prefix): each excluded short form must land in its own
        // field. A mis-ordered insert would mis-parse one of these.
        assert_eq!(
            SearchTerms::from_query_string("-=foo").excluded_filename,
            vec!["foo"]
        );
        assert_eq!(
            SearchTerms::from_query_string("-<foo").excluded_links,
            vec!["foo"]
        );
        assert_eq!(
            SearchTerms::from_query_string("->foo").excluded_forward_links,
            vec!["foo"]
        );
        assert_eq!(
            SearchTerms::from_query_string("-@foo").excluded_breadcrumb,
            vec!["foo"]
        );
        assert_eq!(
            SearchTerms::from_query_string("-/foo").excluded_path,
            vec!["foo"]
        );
        assert_eq!(
            SearchTerms::from_query_string("-#foo").excluded_labels,
            vec!["foo"]
        );
    }

    #[test]
    fn positive_short_forms_parse_to_fields() {
        // The positive counterparts of the exclusion short forms.
        assert_eq!(SearchTerms::from_query_string("=foo").filename, vec!["foo"]);
        assert_eq!(SearchTerms::from_query_string("<foo").links, vec!["foo"]);
        assert_eq!(
            SearchTerms::from_query_string(">foo").forward_links,
            vec!["foo"]
        );
        assert_eq!(
            SearchTerms::from_query_string("@foo").breadcrumb,
            vec!["foo"]
        );
        assert_eq!(SearchTerms::from_query_string("/foo").path, vec!["foo"]);
        assert_eq!(SearchTerms::from_query_string("#foo").labels, vec!["foo"]);
    }

    #[test]
    fn from_query_string_caps_input_length() {
        let huge = "#a ".repeat(20_000); // 60 KB
        let s = SearchTerms::from_query_string(huge);
        // The cap is 8 KB; after dedup, labels has at most 1 entry.
        assert!(s.labels.len() <= 1);
    }

    #[test]
    fn with_order_inserts_into_plain_query() {
        use super::{with_order_directive, OrderField};
        assert_eq!(
            with_order_directive("hello world", OrderField::Title, true),
            "hello world or:title"
        );
        assert_eq!(
            with_order_directive("hello", OrderField::FileName, false),
            "hello -or:file"
        );
    }

    #[test]
    fn with_order_replaces_existing_directive() {
        use super::{with_order_directive, OrderField};
        assert_eq!(
            with_order_directive("foo or:title bar", OrderField::FileName, true),
            "foo bar or:file"
        );
        assert_eq!(
            with_order_directive("-or:file foo", OrderField::Title, true),
            "foo or:title"
        );
        assert_eq!(
            with_order_directive("foo ^title", OrderField::Title, false),
            "foo -or:title"
        );
        assert_eq!(
            with_order_directive("-^file foo", OrderField::FileName, true),
            "foo or:file"
        );
    }

    #[test]
    fn quote_query_term_wraps_only_when_whitespace() {
        use super::quote_query_term;
        assert_eq!(quote_query_term("spec"), "spec");
        assert_eq!(quote_query_term("my note"), "\"my note\"");
        // Round-trips through the parser as a single link target.
        let s = SearchTerms::from_query_string(format!("<{}", quote_query_term("my note")));
        assert_eq!(s.links, vec!["my note".to_string()]);
    }

    #[test]
    fn strip_order_removes_directive_keeps_rest() {
        use super::strip_order_directive;
        assert_eq!(strip_order_directive("foo or:title bar"), "foo bar");
        assert_eq!(strip_order_directive("-^file <{note}"), "<{note}");
        assert_eq!(strip_order_directive("<{note}"), "<{note}");
        assert_eq!(strip_order_directive("or:title"), "");
    }

    #[test]
    fn with_order_empty_query_yields_bare_directive() {
        use super::{with_order_directive, OrderField};
        assert_eq!(
            with_order_directive("", OrderField::Title, true),
            "or:title"
        );
    }

    #[test]
    fn with_order_roundtrips_through_parser() {
        use super::{with_order_directive, OrderBy, OrderField, SearchTerms};
        let q = with_order_directive("note text", OrderField::Title, false);
        let st = SearchTerms::from_query_string(&q);
        assert!(matches!(
            st.order_by.first(),
            Some(OrderBy::Title { asc: false })
        ));
        assert!(st.terms.iter().any(|t| t == "note"));
    }

    #[test]
    fn with_order_strips_all_existing_directives() {
        use super::{with_order_directive, OrderField};
        assert_eq!(
            with_order_directive("or:title foo -or:file", OrderField::FileName, true),
            "foo or:file"
        );
    }

    #[test]
    fn bare_prefix_terms_are_dropped() {
        // None of these bare prefixes should produce a term.
        for q in &[
            "=", "<", ">", "@", "/", "#", "-", "-=", "-<", "->", "-@", "-/", "-#", "name:", "lk:",
            "fwd:", "in:", "pt:", "lb:", "-name:", "-lk:", "-fwd:", "-in:", "-pt:", "-lb:",
        ] {
            let s = SearchTerms::from_query_string(*q);
            assert!(s.terms.is_empty(), "{:?} produced terms: {:?}", q, s.terms);
            assert!(
                s.breadcrumb.is_empty(),
                "{:?} produced breadcrumb: {:?}",
                q,
                s.breadcrumb
            );
            assert!(
                s.filename.is_empty(),
                "{:?} produced filename: {:?}",
                q,
                s.filename
            );
            assert!(s.path.is_empty(), "{:?} produced path: {:?}", q, s.path);
            assert!(
                s.labels.is_empty(),
                "{:?} produced labels: {:?}",
                q,
                s.labels
            );
            assert!(
                s.excluded_terms.is_empty(),
                "{:?} produced excluded_terms: {:?}",
                q,
                s.excluded_terms
            );
            assert!(
                s.excluded_breadcrumb.is_empty(),
                "{:?} produced excluded_breadcrumb: {:?}",
                q,
                s.excluded_breadcrumb
            );
            assert!(
                s.excluded_filename.is_empty(),
                "{:?} produced excluded_filename: {:?}",
                q,
                s.excluded_filename
            );
            assert!(
                s.excluded_path.is_empty(),
                "{:?} produced excluded_path: {:?}",
                q,
                s.excluded_path
            );
            assert!(
                s.excluded_labels.is_empty(),
                "{:?} produced excluded_labels: {:?}",
                q,
                s.excluded_labels
            );
            assert!(s.links.is_empty(), "{:?} produced links: {:?}", q, s.links);
            assert!(
                s.excluded_links.is_empty(),
                "{:?} produced excluded_links: {:?}",
                q,
                s.excluded_links
            );
            assert!(
                s.forward_links.is_empty(),
                "{:?} produced forward_links: {:?}",
                q,
                s.forward_links
            );
            assert!(
                s.excluded_forward_links.is_empty(),
                "{:?} produced excluded_forward_links: {:?}",
                q,
                s.excluded_forward_links
            );
        }
    }
}
