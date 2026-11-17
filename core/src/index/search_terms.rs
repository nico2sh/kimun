use std::vec;

use log::debug;

use crate::note::properties::{search_form, search_key};

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
    Property,
    ExcludedProperty,
}

struct QueryTermExtractor {
    el_type: ElementType,
    term: String,
    remainder: String,
}

// Table of (long_prefix, short_prefix, element_type_tag) for non-special prefix types.
// Excluded variants must come before their positive counterparts so longer prefixes match first.
type PrefixEntry = (&'static str, &'static str, fn() -> ElementType);

fn prefix_table() -> [PrefixEntry; 14] {
    [
        ("-name:", "-=", || ElementType::ExcludedAt),
        ("-lk:", "-<", || ElementType::ExcludedLinks),
        ("-fwd:", "->", || ElementType::ExcludedForwardLinks),
        ("-in:", "-@", || ElementType::ExcludedIn),
        ("-pt:", "-/", || ElementType::ExcludedPath),
        ("-lb:", "-#", || ElementType::ExcludedLabel),
        ("-prop:", "-%", || ElementType::ExcludedProperty),
        ("name:", "=", || ElementType::At),
        ("lk:", "<", || ElementType::Links),
        ("fwd:", ">", || ElementType::ForwardLinks),
        ("in:", "@", || ElementType::In),
        ("pt:", "/", || ElementType::Path),
        ("lb:", "#", || ElementType::Label),
        ("prop:", "%", || ElementType::Property),
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
            if let Some((order, prefix_len)) = order_prefix(query) {
                (order, query[prefix_len..].to_string())
            } else if let Some(rest) = query.strip_prefix('-') {
                (ElementType::ExcludedTerm, rest.to_string())
            } else {
                (ElementType::Term, query.to_string())
            }
        };

        // A term quoting carries past a space (`%"due date"<x`,
        // `%k="in progress"`, `^%"due date"`) ends at its closing quote.
        match quoted_term_len(&element_type, &remaining) {
            Some(Some(end)) => {
                return QueryTermExtractor {
                    el_type: element_type,
                    term: remaining[..end].to_string(),
                    remainder: remaining[end..].trim().to_string(),
                };
            }
            Some(None) => {
                return QueryTermExtractor {
                    el_type: ElementType::Invalid,
                    term: String::new(),
                    remainder: String::new(),
                };
            }
            None => {}
        }

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

/// The quoted span a token of kind `el` carries past spaces, if any: a
/// property term ([`property_term_len`]) or a quoted sort key
/// ([`quoted_sort_key_len`]).
fn quoted_term_len(el: &ElementType, term: &str) -> Option<Option<usize>> {
    match el {
        ElementType::Property | ElementType::ExcludedProperty => property_term_len(term),
        ElementType::OrderBy { .. } => quoted_sort_key_len(term),
        _ => None,
    }
}

/// The characters a property operator is made of.
const OPERATOR_CHARS: [char; 4] = ['=', '!', '<', '>'];

/// When `s` opens with a quote: `Some(Some(len))`, the byte length through
/// the matching closing quote, or `Some(None)` when it is never closed.
/// `None` when `s` doesn't start with a quote.
fn quoted_len(s: &str) -> Option<Option<usize>> {
    let quote = s.chars().next().filter(|c| *c == '"' || *c == '\'')?;
    Some(s[1..].find(quote).map(|close| close + 2))
}

/// `s` without one pair of matching surrounding quotes.
fn unquote(s: &str) -> &str {
    ['"', '\'']
        .into_iter()
        .find_map(|q| s.strip_prefix(q).and_then(|r| r.strip_suffix(q)))
        .unwrap_or(s)
}

/// The byte length of a property term that quoting carries past a space: a
/// quoted key directly followed by an operator (`"due date"<2025-01-01`,
/// `"due date"="in progress"`), or a plain key whose value is quoted right
/// after the operator (`status="in progress"`). `Some(None)` when such a
/// quote is never closed; `None` when the term has neither shape, so the
/// ordinary token rules apply (that includes whole-term quoting,
/// `"status=in progress"`). Shared by the parser, the lexer and the bare-
/// prefix expander so they can't disagree.
fn property_term_len(term: &str) -> Option<Option<usize>> {
    let op_len = |s: &str| s.chars().take_while(|c| OPERATOR_CHARS.contains(c)).count();
    if let Some(Some(key_len)) = quoted_len(term) {
        let rest = &term[key_len..];
        let ops = op_len(rest);
        if ops == 0 {
            return None;
        }
        let value = &rest[ops..];
        let value_len = match quoted_len(value) {
            Some(Some(n)) => n,
            Some(None) => return Some(None),
            None => value.find(' ').unwrap_or(value.len()),
        };
        return Some(Some(key_len + ops + value_len));
    }
    let token_end = term.find(' ').unwrap_or(term.len());
    let op_at = term[..token_end].find(OPERATOR_CHARS)?;
    let after_op = op_at + op_len(&term[op_at..]);
    quoted_len(&term[after_op..]).map(|len| len.map(|n| after_op + n))
}

/// The byte length of a sort term whose property key is quoted
/// (`%"due date"`, `prop:'due date'`); `Some(None)` when the quote is never
/// closed, `None` for any other sort term.
fn quoted_sort_key_len(term: &str) -> Option<Option<usize>> {
    let key = term
        .strip_prefix("prop:")
        .or_else(|| term.strip_prefix('%'))?;
    let prefix = term.len() - key.len();
    quoted_len(key).map(|len| len.map(|n| prefix + n))
}

/// A parsed `or:`/`^` order directive: the column to sort by together with
/// its direction. Produced by the query parser when it encounters an order
/// token; [`OrderField`] is the direction-free counterpart used by callers
/// that carry the asc/desc choice separately.
#[derive(Debug, Clone, PartialEq, Eq)]
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
    /// Sort by a frontmatter property (`or:prop:key` / `or:%key`). Notes
    /// without it sort last in either direction.
    Property {
        /// Lowercased property key.
        key: String,
        /// `true` to sort ascending, `false` to sort descending.
        asc: bool,
    },
}

impl OrderBy {
    /// The column and direction (`true` for ascending).
    pub fn field(&self) -> (OrderField, bool) {
        match self {
            OrderBy::Title { asc } => (OrderField::Title, *asc),
            OrderBy::FileName { asc } => (OrderField::FileName, *asc),
            OrderBy::Property { key, asc } => (OrderField::Property(key.clone()), *asc),
        }
    }

    fn from_term(term: &str, asc: bool) -> Option<Self> {
        if let Some(key) = term
            .strip_prefix("prop:")
            .or_else(|| term.strip_prefix('%'))
        {
            return search_key(unquote(key)).map(|key| OrderBy::Property { key, asc });
        }
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
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OrderField {
    /// Order results by note title.
    Title,
    /// Order results by filename.
    FileName,
    /// Order results by a frontmatter property, the key as the user wrote
    /// it; [`with_order_directive`] quotes it when needed.
    Property(String),
}

/// Comparison operator of a `prop:key<op>value` filter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PropertyOp {
    /// `=` — equal; on a list, "contains this item".
    Eq,
    /// `!=` — has the property but no value equal to this one.
    Ne,
    /// `<`
    Lt,
    /// `<=`
    Le,
    /// `>`
    Gt,
    /// `>=`
    Ge,
}

impl PropertyOp {
    /// The SQL comparison operator.
    pub fn sql(self) -> &'static str {
        match self {
            PropertyOp::Eq => "=",
            PropertyOp::Ne => "!=",
            PropertyOp::Lt => "<",
            PropertyOp::Le => "<=",
            PropertyOp::Gt => ">",
            PropertyOp::Ge => ">=",
        }
    }
}

/// What a `prop:` filter asks of a property.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum PropertyTest {
    /// A bare key (`%due`): the note has the property, whatever its value —
    /// including none (YAML `due:`) or an empty list.
    Exists,
    /// `key<op>value`: a value of the property compares true against `value`
    /// (in `search_form`, without surrounding quotes).
    Compare {
        /// The comparison.
        op: PropertyOp,
        /// The value compared against.
        value: String,
        /// Whether `*` in `value` is a wildcard (for `=` / `!=`): true for an
        /// unquoted value; a quoted one is matched literally (`"***"`).
        wildcard: bool,
    },
}

/// One `prop:` / `%` filter: `key<op>value`, or a bare `key` (has the
/// property). Keys and values are in `search_form`: property matching
/// ignores case and accents.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PropertyFilter {
    /// The property key.
    pub key: String,
    /// What the filter asks of the property.
    pub test: PropertyTest,
}

impl PropertyFilter {
    /// The has-property filter for `key`, as typed in a form: `None` when the
    /// key can't be a property (blank, multi-line).
    pub fn exists(key: &str) -> Option<Self> {
        Some(Self {
            key: search_key(key)?,
            test: PropertyTest::Exists,
        })
    }

    /// The filter `key<op>value`, as typed in a form (not as written in a
    /// query, so nothing is quoted): the key and value are put in search form,
    /// and a `*` in the value is a wildcard, as in an unquoted query value.
    /// `None` when the key can't be a property or the value is blank.
    pub fn compare(key: &str, op: PropertyOp, value: &str) -> Option<Self> {
        let value = search_form(value.trim());
        (!value.is_empty()).then_some(())?;
        Some(Self {
            key: search_key(key)?,
            test: PropertyTest::Compare {
                op,
                wildcard: value.contains('*'),
                value,
            },
        })
    }

    /// Parses `key<op>value`, or a bare `key` as a has-property filter. The
    /// key runs up to the first `= ! < >`; `None` when the key is empty, or an
    /// operator is present but malformed or has no value.
    pub fn parse(term: &str) -> Option<Self> {
        // A quoted key (`"due date"<x`) may hold any character; otherwise the
        // key runs up to the first operator character.
        let (key, rest) = match quoted_len(term) {
            Some(Some(n)) if term[n..].starts_with(OPERATOR_CHARS) => {
                (unquote(&term[..n]), &term[n..])
            }
            _ => match term.find(OPERATOR_CHARS) {
                Some(at) => (&term[..at], &term[at..]),
                None => {
                    return Some(Self {
                        key: search_key(unquote(term))?,
                        test: PropertyTest::Exists,
                    });
                }
            },
        };
        let key = search_key(key)?;
        let (op, len) = [
            ("!=", PropertyOp::Ne),
            ("<=", PropertyOp::Le),
            (">=", PropertyOp::Ge),
            ("=", PropertyOp::Eq),
            ("<", PropertyOp::Lt),
            (">", PropertyOp::Gt),
        ]
        .into_iter()
        .find_map(|(s, op)| rest.starts_with(s).then_some((op, s.len())))?;
        let raw = rest[len..].trim();
        // `k<>x`, `k==x`: an unquoted value can't start with another
        // operator character — that's a malformed operator, not a value.
        if raw.starts_with(OPERATOR_CHARS) {
            return None;
        }
        let unquoted = unquote(raw);
        let wildcard = unquoted.len() == raw.len() && unquoted.contains('*');
        let value = search_form(unquoted);
        (!value.is_empty()).then_some(Self {
            key,
            test: PropertyTest::Compare {
                op,
                value,
                wildcard,
            },
        })
    }
}

/// The order directive `token` starts with, in any of its four forms —
/// `-or:`, `or:`, `-^`, `^` — and the directive's length.
fn order_prefix(token: &str) -> Option<(ElementType, usize)> {
    let (desc, rest) = match token.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, token),
    };
    let directive_len = if rest.starts_with(ORDER_CHAR) {
        ORDER_CHAR.len()
    } else if rest
        .strip_prefix(ORDER_LETTER)
        .is_some_and(|after| after.starts_with(':'))
    {
        ORDER_LETTER.len() + 1
    } else {
        return None;
    };
    Some((
        ElementType::OrderBy { asc: !desc },
        usize::from(desc) + directive_len,
    ))
}

/// The query token at the start of `rest` (which has no leading
/// whitespace), exactly as the parser reads it: its byte length, its kind
/// (a field prefix or an order directive, `None` for a bare term) and the
/// length of that prefix. A quoted value — or a term quoting carries past a
/// space (`%"due date"<x`, `^%"due date"`) — ends at its closing quote, or
/// takes the rest of the query when unterminated; anything else ends at the
/// next ASCII space. The one tokenizer the bare-prefix expander and the
/// order-directive helpers share with the parser's grammar.
fn next_token(rest: &str) -> (usize, Option<ElementType>, usize) {
    let (kind, prefix_len) = match detect_prefix(rest) {
        Some((el, remaining)) => (Some(el), rest.len() - remaining.len()),
        None => match order_prefix(rest) {
            Some((el, prefix_len)) => (Some(el), prefix_len),
            None => (None, 0),
        },
    };
    let value = &rest[prefix_len..];
    let quoted = kind
        .as_ref()
        .and_then(|el| quoted_term_len(el, value))
        .or_else(|| quoted_len(value));
    let len = match quoted {
        Some(Some(n)) => prefix_len + n,
        Some(None) => rest.len(),
        None => rest.find(' ').unwrap_or(rest.len()),
    };
    (len, kind, prefix_len)
}

/// The tokens of `query` (see [`next_token`]) with their kinds and whether
/// the token is an unclosed quote running to the end, whitespace between them
/// dropped.
fn query_tokens(query: &str) -> impl Iterator<Item = (&str, Option<ElementType>, bool)> {
    let mut rest = query;
    std::iter::from_fn(move || {
        rest = rest.trim_start();
        if rest.is_empty() {
            return None;
        }
        let (len, kind, _) = next_token(rest);
        let token = &rest[..len];
        rest = &rest[len..];
        let unterminated = rest.is_empty() && query_has_unterminated_quote(token);
        Some((token, kind, unterminated))
    })
}

/// Wrap `term` in the search DSL's quote characters when it needs them, so a
/// multi-word value (e.g. a note name with spaces) is parsed as a single token
/// instead of being split across terms: when it contains whitespace or starts
/// with a quote of its own. Other values are returned unchanged. The quote
/// character is `"`, or `'` for a value containing `"`. Note names can never
/// contain `"` (invalid on Windows/macOS/Linux filesystems alike).
pub fn quote_query_term(term: &str) -> String {
    if value_needs_quotes(term) {
        wrap_in_quotes(term)
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

        // A prefix is only meaningful at the token start; the token ends
        // where the parser ends it (`next_token`).
        let (token_len, kind, prefix_len) = next_token(rest);
        out.push_str(&rest[..token_len]);
        // Bare prefix: the whole token is the prefix itself.
        if prefix_len == token_len && kind.as_ref().is_some_and(is_note_element) {
            out.push_str(target);
        }
        rest = &rest[token_len..];
    }
    out
}

/// Return `query` with any order directive (`or:`/`-or:`/`^`/`-^`, in any
/// position) removed. Other tokens keep their order and their text (a quoted
/// value keeps its inner spaces); the whitespace between tokens becomes a
/// single space. Tokens are read as the parser reads them, so a quoted sort
/// key (`^%"due date"`) is removed whole. The DSL knowledge lives here in core
/// so the TUI never hardcodes the directive syntax.
pub fn strip_order_directive(query: &str) -> String {
    let mut kept = Vec::new();
    for (token, kind, unterminated) in query_tokens(query) {
        if unterminated {
            // The parser drops everything from an unclosed quote on; strip
            // directives word by word there, so a sort set after it replaces
            // rather than piles up.
            kept.extend(
                token
                    .split_whitespace()
                    .filter(|w| order_prefix(w).is_none()),
            );
        } else if !matches!(kind, Some(ElementType::OrderBy { .. })) {
            kept.push(token);
        }
    }
    kept.join(" ")
}

/// Return `query` with its order directive replaced by `field`/`asc`.
///
/// Any existing order directive is stripped (see [`strip_order_directive`]),
/// then the canonical `or:<field>` (ascending) / `-or:<field>` (descending)
/// directive is appended.
pub fn with_order_directive(query: &str, field: OrderField, asc: bool) -> String {
    let base = strip_order_directive(query);
    let directive = order_directive(&field, asc, false);
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
/// - `prop:` / `%` — property filter: `key<op>value` (`prop:status=done`,
///   `%priority>2`) or a bare key for "has the property" (`%due`)
/// - `or:` / `^` — order directive (`or:title`, `^file`, …)
///
/// Any prefix may be negated by a leading `-` (`-#draft`, `-lk:spec`) to
/// route the token into the matching `excluded_*` field. Values may be quoted
/// with `"` or `'` to include whitespace (e.g. `="my note"`); an unterminated
/// quote discards the token. Bare prefixes with no value are dropped.
#[derive(Default, Debug, Clone, PartialEq, Eq)]
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
    /// `prop:` / `%` filters (`key<op>value`, or a bare `key`).
    pub properties: Vec<PropertyFilter>,
    /// Negated `prop:` / `%` filters (`-prop:`, `-%`).
    pub excluded_properties: Vec<PropertyFilter>,
    /// Tokens the parser could not turn into a filter and so ignores when
    /// searching: an unterminated quote (which runs to the end of the query),
    /// a prefix with no value (`#`, `lk:`), a malformed property filter or
    /// an unknown sort key. Kept exactly as written so a query editor can
    /// show them and [`SearchTerms::to_query_string`] can write them back.
    /// The index never reads this field.
    pub unparsed: Vec<String>,
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
        let mut properties = vec![];
        let mut excluded_properties = vec![];
        let mut unparsed = vec![];
        while !query.is_empty() {
            let before = query;
            let qp = QueryTermExtractor::extract_and_consume(&before);
            query = qp.remainder;
            // Each arm reports whether the token was usable; the others are
            // kept verbatim in `unparsed`.
            let usable = match qp.el_type {
                ElementType::Term => push_nonempty(&mut terms, qp.term),
                ElementType::In => push_nonempty(&mut breadcrumb, qp.term),
                ElementType::At => push_nonempty(&mut filename, qp.term),
                ElementType::OrderBy { asc } => match OrderBy::from_term(&qp.term, asc) {
                    Some(o) => {
                        order_by.push(o);
                        true
                    }
                    None => false,
                },
                ElementType::Invalid => false,
                ElementType::Path => push_nonempty(&mut path, qp.term),
                ElementType::Label => push_nonempty(&mut labels, qp.term.to_lowercase()),
                ElementType::ExcludedTerm => push_nonempty(&mut excluded_terms, qp.term),
                ElementType::ExcludedIn => push_nonempty(&mut excluded_breadcrumb, qp.term),
                ElementType::ExcludedAt => push_nonempty(&mut excluded_filename, qp.term),
                ElementType::ExcludedPath => push_nonempty(&mut excluded_path, qp.term),
                ElementType::ExcludedLabel => {
                    push_nonempty(&mut excluded_labels, qp.term.to_lowercase())
                }
                ElementType::Links => push_nonempty(&mut links, qp.term),
                ElementType::ExcludedLinks => push_nonempty(&mut excluded_links, qp.term),
                ElementType::ForwardLinks => push_nonempty(&mut forward_links, qp.term),
                ElementType::ExcludedForwardLinks => {
                    push_nonempty(&mut excluded_forward_links, qp.term)
                }
                ElementType::Property => {
                    push_some(&mut properties, PropertyFilter::parse(&qp.term))
                }
                ElementType::ExcludedProperty => {
                    push_some(&mut excluded_properties, PropertyFilter::parse(&qp.term))
                }
            };
            if !usable {
                // What the extractor consumed, as written. Not a second
                // tokenizer pass: it can end a token earlier than the lexer
                // does (an empty quote pair after a prefix), and the rest is
                // then a token of its own.
                let trimmed = before.trim();
                let consumed = trimmed[..trimmed.len() - query.len()].trim_end();
                if !consumed.is_empty() {
                    unparsed.push(consumed.to_string());
                }
            }
        }

        dedup_preserving_order(&mut labels);
        dedup_preserving_order(&mut excluded_labels);
        dedup_preserving_order(&mut links);
        dedup_preserving_order(&mut excluded_links);
        dedup_preserving_order(&mut forward_links);
        dedup_preserving_order(&mut excluded_forward_links);
        dedup_preserving_order(&mut properties);
        dedup_preserving_order(&mut excluded_properties);

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
            properties,
            excluded_properties,
            unparsed,
        }
    }

    /// Write the terms back as a query string: the inverse of
    /// [`SearchTerms::from_query_string`]. Parsing the result gives back an
    /// equal `SearchTerms`, though not necessarily the same text: operators
    /// come out in their short form, grouped by kind, and values are quoted
    /// only when they need it. The one exception is a property key or value
    /// that itself begins with a quote character, which the syntax can't
    /// always express. Order does not matter between filters (they
    /// are ANDed); sort directives keep their order. `unparsed` tokens come
    /// last and verbatim, since an unterminated quote runs to the end.
    ///
    /// ```
    /// use kimun_core::SearchTerms;
    ///
    /// let st = SearchTerms::from_query_string("lb:urgent -name:\"my note\" meeting");
    /// assert_eq!(st.to_query_string(), "meeting #urgent -=\"my note\"");
    /// ```
    pub fn to_query_string(&self) -> String {
        let mut out: Vec<String> = Vec::new();
        out.extend(self.terms.iter().map(|v| bare_term_token("", v)));
        let valued: [(&str, &Vec<String>); 6] = [
            ("@", &self.breadcrumb),
            ("=", &self.filename),
            ("/", &self.path),
            ("#", &self.labels),
            ("<", &self.links),
            (">", &self.forward_links),
        ];
        for (prefix, values) in valued {
            out.extend(
                values
                    .iter()
                    .map(|v| format!("{prefix}{}", quote_query_term(v))),
            );
        }
        out.extend(self.excluded_terms.iter().map(|v| bare_term_token("-", v)));
        let excluded: [(&str, &Vec<String>); 6] = [
            ("-@", &self.excluded_breadcrumb),
            ("-=", &self.excluded_filename),
            ("-/", &self.excluded_path),
            ("-#", &self.excluded_labels),
            ("-<", &self.excluded_links),
            ("->", &self.excluded_forward_links),
        ];
        for (prefix, values) in excluded {
            out.extend(
                values
                    .iter()
                    .map(|v| format!("{prefix}{}", quote_query_term(v))),
            );
        }
        out.extend(
            self.properties
                .iter()
                .map(|f| format!("%{}", property_token(f))),
        );
        out.extend(
            self.excluded_properties
                .iter()
                .map(|f| format!("-%{}", property_token(f))),
        );
        out.extend(self.order_by.iter().map(|o| {
            let (field, asc) = o.field();
            order_directive(&field, asc, true)
        }));
        out.extend(self.unparsed.iter().cloned());
        out.join(" ")
    }
}

/// Pushes `value` unless it is empty; whether it was pushed.
fn push_nonempty(into: &mut Vec<String>, value: String) -> bool {
    let keep = !value.is_empty();
    if keep {
        into.push(value);
    }
    keep
}

/// Pushes `value` when there is one; whether it was.
fn push_some<T>(into: &mut Vec<T>, value: Option<T>) -> bool {
    match value {
        Some(v) => {
            into.push(v);
            true
        }
        None => false,
    }
}

/// `value` in quotes, with whichever quote character it does not contain
/// (`"` unless it holds one). A value holding both can't be written; the
/// parser would end it at the first inner `'`.
fn wrap_in_quotes(value: &str) -> String {
    let q = if value.contains('"') { '\'' } else { '"' };
    format!("{q}{value}{q}")
}

/// Whether a value written right after a prefix must be quoted to read back
/// as written: it holds whitespace, or starts with a quote of its own.
fn value_needs_quotes(value: &str) -> bool {
    value.chars().any(char::is_whitespace) || value.starts_with(['"', '\''])
}

/// A full-text term, written after `lead` (empty, or `-` for an excluded
/// one). Besides the quoting every value gets, a term that would read as a
/// prefix, an order directive or an exclusion once `lead` is in front of it
/// is quoted too (`"#draft"`, `-"#draft"`).
fn bare_term_token(lead: &str, term: &str) -> String {
    let written = format!("{lead}{term}");
    let reads_as_syntax = (lead.is_empty() && term.starts_with('-'))
        || detect_prefix(&written).is_some()
        || order_prefix(&written).is_some();
    let token = if reads_as_syntax || value_needs_quotes(term) {
        wrap_in_quotes(term)
    } else {
        term.to_string()
    };
    format!("{lead}{token}")
}

/// A property filter after its `%` / `-%`: `key`, or `key<op>value`. The key
/// is quoted when it holds whitespace, quotes or an operator character. A
/// value is quoted when whitespace, a leading quote or operator character, or
/// (for a literal value) a `*` would otherwise change its meaning. A wildcard
/// value holding whitespace can only be written by quoting the whole filter.
fn property_token(filter: &PropertyFilter) -> String {
    let key = if filter
        .key
        .chars()
        .any(|c| c.is_whitespace() || OPERATOR_CHARS.contains(&c))
        || filter.key.starts_with(['"', '\''])
    {
        wrap_in_quotes(&filter.key)
    } else {
        filter.key.clone()
    };
    let PropertyTest::Compare {
        op,
        value,
        wildcard,
    } = &filter.test
    else {
        return key;
    };
    if *wildcard {
        let body = format!("{key}{}{value}", op.sql());
        return if value.chars().any(char::is_whitespace) {
            wrap_in_quotes(&body)
        } else {
            body
        };
    }
    let literal_needs_quotes =
        value_needs_quotes(value) || value.starts_with(OPERATOR_CHARS) || value.contains('*');
    let value = if literal_needs_quotes {
        wrap_in_quotes(value)
    } else {
        value.clone()
    };
    format!("{key}{}{value}", op.sql())
}

/// An order directive, the one writer for both [`with_order_directive`]
/// (`or:title`, `-or:prop:key`) and [`SearchTerms::to_query_string`]
/// (`short`: `^title`, `-^%key`).
fn order_directive(field: &OrderField, asc: bool, short: bool) -> String {
    let (directive, property) = if short {
        (ORDER_CHAR.to_string(), "%")
    } else {
        (format!("{ORDER_LETTER}:"), "prop:")
    };
    let field = match field {
        OrderField::Title => "title".to_string(),
        OrderField::FileName => "file".to_string(),
        OrderField::Property(key) => format!("{property}{}", quote_query_term(key)),
    };
    format!("{}{directive}{field}", if asc { "" } else { "-" })
}

fn dedup_preserving_order<T: Eq + std::hash::Hash + Clone>(v: &mut Vec<T>) {
    let mut seen = std::collections::HashSet::new();
    v.retain(|x| seen.insert(x.clone()));
}

// ---------------------------------------------------------------------------
// Query lexer — token spans for presentation (syntax highlighting)
// ---------------------------------------------------------------------------

/// Token class of a span in a query string, for syntax highlighting. Mirrors
/// the grammar `QueryTermExtractor::extract_and_consume` consumes — the two
/// must stay in step (see the `lexer_agrees_with_parser_*` tests).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueryTokenClass {
    /// A leading `-` (exclusion).
    Negation,
    /// A field prefix: a sigil (`<` `>` `=` `@` `/` `#` `%` `^`) or its long form
    /// (`lk:` `fwd:` `name:` `in:` `pt:` `lb:` `prop:` `or:`).
    FieldKey,
    /// A note-targeting value (after `<` / `>` / `=` and long forms).
    LinkValue,
    /// A label value (after `#` / `lb:`).
    TagValue,
    /// A quoted value (any field), quotes included.
    Quoted,
    /// A bare `YYYY-MM-DD` date term.
    Date,
    /// A bare numeric term.
    Number,
    /// A plain search term or unclassified value.
    Term,
    /// An opening quote with no closing quote: the parser drops everything
    /// from here on. The one real "parse error" the lenient grammar has.
    Unterminated,
}

/// One classified span of a query string. `range` indexes the original
/// string (byte offsets), so spans can be styled in place.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueryTokenSpan {
    /// Byte range of the span in the query string.
    pub range: std::ops::Range<usize>,
    /// What the span is.
    pub class: QueryTokenClass,
}

/// True if `term` looks like a `YYYY-MM-DD` date.
fn is_date_like(term: &str) -> bool {
    let b = term.as_bytes();
    b.len() == 10
        && b[4] == b'-'
        && b[7] == b'-'
        && b.iter()
            .enumerate()
            .all(|(i, c)| matches!(i, 4 | 7) || c.is_ascii_digit())
}

/// Value class for the span after a field prefix (or a bare term).
fn value_class(el: &ElementType, term: &str) -> QueryTokenClass {
    match el {
        ElementType::Label | ElementType::ExcludedLabel => QueryTokenClass::TagValue,
        ElementType::Links
        | ElementType::ExcludedLinks
        | ElementType::ForwardLinks
        | ElementType::ExcludedForwardLinks
        | ElementType::At
        | ElementType::ExcludedAt => QueryTokenClass::LinkValue,
        ElementType::Term | ElementType::ExcludedTerm => {
            if is_date_like(term) {
                QueryTokenClass::Date
            } else if !term.is_empty() && term.chars().all(|c| c.is_ascii_digit()) {
                QueryTokenClass::Number
            } else {
                QueryTokenClass::Term
            }
        }
        _ => QueryTokenClass::Term,
    }
}

/// Lex `query` into classified spans for syntax highlighting. Whitespace is
/// not covered by any span. The lexer follows the parser's grammar exactly:
/// tokens split on ASCII space, a prefix is recognized at a token start, a
/// quote is honored only at a value start, and an unterminated quote swallows
/// the rest of the string (classified [`QueryTokenClass::Unterminated`]).
pub fn query_token_spans(query: &str) -> Vec<QueryTokenSpan> {
    let mut spans = Vec::new();
    let mut pos = 0usize;
    let len = query.len();

    while pos < len {
        // Skip whitespace at a token boundary. The parser splits on ASCII
        // space and then `trim()`s each round, so any Unicode whitespace at
        // a token START is discarded — while whitespace *inside* an unquoted
        // token (e.g. a tab between letters) stays part of it, which this
        // loop preserves because it only runs at boundaries.
        let c = query[pos..].chars().next().expect("pos < len");
        if c.is_whitespace() {
            pos += c.len_utf8();
            continue;
        }
        let token_start = pos;
        let rest = &query[pos..];

        // Leading `-`: exclusion for prefixes, order, or bare terms. The
        // prefix table folds `-` into its entries; the lexer emits it as its
        // own span so it can be styled red.
        let (neg, after_neg) = match rest.strip_prefix('-') {
            Some(r) => (true, r),
            None => (false, rest),
        };

        // Field prefix (long form first, then sigil), then order forms.
        let prefixed = prefix_table()
            .into_iter()
            .find_map(|(long, short, make_type)| {
                let long = long.strip_prefix('-').unwrap_or(long);
                let short = short.strip_prefix('-').unwrap_or(short);
                // Only the matching polarity exists in the table twice; the
                // stripped forms are identical, so dedupe by polarity here.
                after_neg
                    .strip_prefix(long)
                    .map(|r| (make_type(), long.len(), r))
                    .or_else(|| {
                        after_neg
                            .strip_prefix(short)
                            .map(|r| (make_type(), short.len(), r))
                    })
            })
            .or_else(|| {
                let order_letter = format!("{ORDER_LETTER}:");
                after_neg
                    .strip_prefix(&order_letter)
                    .map(|r| (ElementType::OrderBy { asc: !neg }, order_letter.len(), r))
                    .or_else(|| {
                        after_neg
                            .strip_prefix(ORDER_CHAR)
                            .map(|r| (ElementType::OrderBy { asc: !neg }, ORDER_CHAR.len(), r))
                    })
            });

        let (el, prefix_len, value) = match prefixed {
            Some((el, plen, value)) => (el, plen, value),
            None => (
                if neg {
                    ElementType::ExcludedTerm
                } else {
                    ElementType::Term
                },
                0,
                after_neg,
            ),
        };

        let mut cursor = token_start;
        if neg {
            spans.push(QueryTokenSpan {
                range: cursor..cursor + 1,
                class: QueryTokenClass::Negation,
            });
            cursor += 1;
        }
        if prefix_len > 0 {
            spans.push(QueryTokenSpan {
                range: cursor..cursor + prefix_len,
                class: QueryTokenClass::FieldKey,
            });
            cursor += prefix_len;
        }

        // Value: quoted (at a value start, or right after a property
        // operator) or up to the next space.
        let property_quoted = quoted_term_len(&el, value);
        if let Some(quoted) = property_quoted {
            let end = quoted.map_or(len, |n| cursor + n);
            spans.push(QueryTokenSpan {
                range: cursor..end,
                class: if quoted.is_some() {
                    QueryTokenClass::Quoted
                } else {
                    QueryTokenClass::Unterminated
                },
            });
            pos = end;
        } else if let Some(q) = value.chars().next().filter(|c| *c == '"' || *c == '\'') {
            match value[1..].find(q) {
                Some(close_rel) => {
                    let end = cursor + 1 + close_rel + 1;
                    spans.push(QueryTokenSpan {
                        range: cursor..end,
                        class: QueryTokenClass::Quoted,
                    });
                    pos = end;
                }
                None => {
                    spans.push(QueryTokenSpan {
                        range: cursor..len,
                        class: QueryTokenClass::Unterminated,
                    });
                    pos = len;
                }
            }
        } else {
            let value_end = value.find(' ').map_or(len, |i| cursor + i);
            if value_end > cursor {
                spans.push(QueryTokenSpan {
                    range: cursor..value_end,
                    class: value_class(&el, &query[cursor..value_end]),
                });
            }
            pos = value_end.max(cursor + usize::from(value_end == cursor));
        }
    }
    spans
}

/// True if `query` ends in an unterminated quoted value — the only real
/// parse error the lenient grammar produces (the parser silently drops the
/// rest of the string).
pub fn query_has_unterminated_quote(query: &str) -> bool {
    query_token_spans(query)
        .last()
        .is_some_and(|s| s.class == QueryTokenClass::Unterminated)
}

#[cfg(test)]
mod lexer_tests {
    use super::*;

    fn classes(q: &str) -> Vec<(QueryTokenClass, String)> {
        query_token_spans(q)
            .into_iter()
            .map(|s| (s.class, q[s.range].to_string()))
            .collect()
    }

    #[test]
    fn lexes_property_prefixes() {
        use QueryTokenClass as C;
        assert_eq!(
            classes("prop:status=done -%n>2"),
            vec![
                (C::FieldKey, "prop:".into()),
                (C::Term, "status=done".into()),
                (C::Negation, "-".into()),
                (C::FieldKey, "%".into()),
                (C::Term, "n>2".into()),
            ]
        );
    }

    #[test]
    fn lexes_quoted_keys_as_one_span() {
        use QueryTokenClass as C;
        assert_eq!(
            classes("%\"due date\"<x y ^%\"due date\""),
            vec![
                (C::FieldKey, "%".into()),
                (C::Quoted, "\"due date\"<x".into()),
                (C::Term, "y".into()),
                (C::FieldKey, "^".into()),
                (C::Quoted, "%\"due date\"".into()),
            ]
        );
    }

    #[test]
    fn lexes_quoted_property_value_as_one_span() {
        use QueryTokenClass as C;
        assert_eq!(
            classes("%status=\"in progress\" x"),
            vec![
                (C::FieldKey, "%".into()),
                (C::Quoted, "status=\"in progress\"".into()),
                (C::Term, "x".into()),
            ]
        );
        assert!(query_has_unterminated_quote("x %status=\"in progress"));
    }

    #[test]
    fn lexes_field_prefixes_and_values() {
        use QueryTokenClass as C;
        assert_eq!(
            classes("#meeting <maria fwd:plan"),
            vec![
                (C::FieldKey, "#".into()),
                (C::TagValue, "meeting".into()),
                (C::FieldKey, "<".into()),
                (C::LinkValue, "maria".into()),
                (C::FieldKey, "fwd:".into()),
                (C::LinkValue, "plan".into()),
            ]
        );
    }

    #[test]
    fn lexes_negation_quotes_dates_numbers() {
        use QueryTokenClass as C;
        assert_eq!(
            classes(r#"-#wip "refresh token" 2026-04-01 42"#),
            vec![
                (C::Negation, "-".into()),
                (C::FieldKey, "#".into()),
                (C::TagValue, "wip".into()),
                (C::Quoted, "\"refresh token\"".into()),
                (C::Date, "2026-04-01".into()),
                (C::Number, "42".into()),
            ]
        );
    }

    #[test]
    fn lexes_order_directive_and_bare_negated_term() {
        use QueryTokenClass as C;
        assert_eq!(
            classes("or:title -^file -draft"),
            vec![
                (C::FieldKey, "or:".into()),
                (C::Term, "title".into()),
                (C::Negation, "-".into()),
                (C::FieldKey, "^".into()),
                (C::Term, "file".into()),
                (C::Negation, "-".into()),
                (C::Term, "draft".into()),
            ]
        );
    }

    /// The parser trims each extraction round, so whitespace other than a
    /// space before a token must not hide its field prefix (regression:
    /// `a \t#x` is a label filter, not a plain term).
    #[test]
    fn whitespace_before_a_token_does_not_hide_its_prefix() {
        use QueryTokenClass as C;
        assert_eq!(
            classes("a \t#x"),
            vec![
                (C::Term, "a".into()),
                (C::FieldKey, "#".into()),
                (C::TagValue, "x".into()),
            ]
        );
        // A tab INSIDE an unquoted token stays part of it (parser doc).
        assert_eq!(classes("a\tb"), vec![(C::Term, "a\tb".into())]);
    }

    #[test]
    fn unterminated_quote_marks_the_tail() {
        use QueryTokenClass as C;
        let got = classes(r#"plan #"half open"#);
        assert_eq!(got.last().unwrap().0, C::Unterminated);
        assert!(query_has_unterminated_quote(r#"plan #"half open"#));
        assert!(!query_has_unterminated_quote(r#"plan #"closed""#));
    }

    /// The lexer and the parser must agree on tokenization: every value the
    /// parser extracts appears verbatim as a value span (not a key/negation).
    #[test]
    fn lexer_agrees_with_parser_on_values() {
        let q = r#"alpha -#wip <maria "two words" pt:proj/sub or:title"#;
        let terms = SearchTerms::from_query_string(q);
        let spans = query_token_spans(q);
        let values: Vec<&str> = spans
            .iter()
            .filter(|s| {
                !matches!(
                    s.class,
                    QueryTokenClass::FieldKey | QueryTokenClass::Negation
                )
            })
            .map(|s| q[s.range.clone()].trim_matches('"'))
            .collect();
        for term in terms
            .terms
            .iter()
            .chain(terms.labels.iter())
            .chain(terms.excluded_labels.iter())
            .chain(terms.links.iter())
            .chain(terms.path.iter())
        {
            assert!(
                values.iter().any(|v| v.eq_ignore_ascii_case(term)),
                "parser term {term:?} missing from lexer values {values:?}"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::expand_bare_note_prefixes;
    use super::SearchTerms;
    use super::{OrderBy, PropertyFilter, PropertyOp, PropertyTest};

    fn pf(key: &str, op: PropertyOp, value: &str) -> PropertyFilter {
        PropertyFilter {
            key: key.into(),
            test: PropertyTest::Compare {
                op,
                value: value.into(),
                wildcard: value.contains('*'),
            },
        }
    }

    fn has(key: &str) -> PropertyFilter {
        PropertyFilter {
            key: key.into(),
            test: PropertyTest::Exists,
        }
    }

    #[test]
    fn property_filter_operators() {
        for (term, op, value) in [
            ("k=v", PropertyOp::Eq, "v"),
            ("k!=v", PropertyOp::Ne, "v"),
            ("k<5", PropertyOp::Lt, "5"),
            ("k<=5", PropertyOp::Le, "5"),
            ("k>2024-01-01", PropertyOp::Gt, "2024-01-01"),
            ("k>=5", PropertyOp::Ge, "5"),
        ] {
            assert_eq!(
                PropertyFilter::parse(term),
                Some(pf("k", op, value)),
                "{term}"
            );
        }
    }

    #[test]
    fn property_filter_normalizes_and_rejects() {
        assert_eq!(
            PropertyFilter::parse("Status=Done"),
            Some(pf("status", PropertyOp::Eq, "done"))
        );
        assert_eq!(
            PropertyFilter::parse("due date=x y"),
            Some(pf("due date", PropertyOp::Eq, "x y"))
        );
        assert_eq!(
            PropertyFilter::parse("k=\"v\""),
            Some(pf("k", PropertyOp::Eq, "v"))
        );
        assert_eq!(PropertyFilter::parse(" K "), Some(has("k")));
        assert_eq!(PropertyFilter::parse("  "), None);
        assert_eq!(PropertyFilter::parse("=v"), None);
        assert_eq!(PropertyFilter::parse("k="), None);
        assert_eq!(PropertyFilter::parse("k!v"), None);
    }

    #[test]
    fn prop_prefixes_long_short_and_excluded() {
        let st =
            SearchTerms::from_query_string("prop:status=done %priority>2 -prop:a=b -%c!=d meeting");
        assert_eq!(
            st.properties,
            vec![
                pf("status", PropertyOp::Eq, "done"),
                pf("priority", PropertyOp::Gt, "2"),
            ]
        );
        assert_eq!(
            st.excluded_properties,
            vec![pf("a", PropertyOp::Eq, "b"), pf("c", PropertyOp::Ne, "d"),]
        );
        assert_eq!(st.terms, vec!["meeting"]);
    }

    // Whole-term quoting carries spaces in the value.
    #[test]
    fn quoted_property_term_keeps_spaces() {
        let st =
            SearchTerms::from_query_string("prop:\"status=in progress\" %'due date<2025-01-01'");
        assert_eq!(
            st.properties,
            vec![
                pf("status", PropertyOp::Eq, "in progress"),
                pf("due date", PropertyOp::Lt, "2025-01-01"),
            ]
        );
        assert!(st.terms.is_empty());
    }

    // A quote right after the operator carries spaces in the value too.
    #[test]
    fn quoted_property_value_keeps_spaces() {
        let st = SearchTerms::from_query_string(
            "%status=\"in progress\" meeting -prop:title<='b c' %n>=2",
        );
        assert_eq!(
            st.properties,
            vec![
                pf("status", PropertyOp::Eq, "in progress"),
                pf("n", PropertyOp::Ge, "2"),
            ]
        );
        assert_eq!(
            st.excluded_properties,
            vec![pf("title", PropertyOp::Le, "b c")]
        );
        assert_eq!(st.terms, vec!["meeting"]);
    }

    #[test]
    fn unterminated_quoted_property_value_drops_the_rest() {
        let st = SearchTerms::from_query_string("meeting %status=\"in progress");
        assert!(st.properties.is_empty());
        assert_eq!(st.terms, vec!["meeting"]);
    }

    #[test]
    fn malformed_property_terms_are_dropped() {
        let st = SearchTerms::from_query_string("%=x %k!v prop:k=");
        assert!(st.properties.is_empty());
        assert!(st.terms.is_empty());
    }

    #[test]
    fn a_quoted_key_takes_an_operator_and_value() {
        let st = SearchTerms::from_query_string(
            "%\"due date\"<2025-01-01 x -prop:'due date'=\"in progress\" %\"Due Date\">=5",
        );
        assert_eq!(
            st.properties,
            vec![
                pf("due date", PropertyOp::Lt, "2025-01-01"),
                pf("due date", PropertyOp::Ge, "5"),
            ]
        );
        assert_eq!(
            st.excluded_properties,
            vec![pf("due date", PropertyOp::Eq, "in progress")]
        );
        assert_eq!(st.terms, vec!["x"]);
        assert!(st.links.is_empty() && st.forward_links.is_empty() && st.filename.is_empty());
        let has_only = SearchTerms::from_query_string("%\"due date\"");
        assert_eq!(has_only.properties, vec![has("due date")]);
    }

    #[test]
    fn a_quoted_sort_key_sorts_by_that_property() {
        let st = SearchTerms::from_query_string("^%\"due date\" -or:prop:'Due Date' x");
        assert!(
            matches!(&st.order_by[0], OrderBy::Property { key, asc: true } if key == "due date")
        );
        assert!(
            matches!(&st.order_by[1], OrderBy::Property { key, asc: false } if key == "due date")
        );
        assert_eq!(st.terms, vec!["x"], "no stray term");
    }

    #[test]
    fn an_operator_run_is_not_a_value() {
        for term in ["k<>x", "k==x", "k<>\"x\"", "k=<5", "k!=!x"] {
            assert_eq!(PropertyFilter::parse(term), None, "{term}");
        }
        assert_eq!(
            PropertyFilter::parse("k=\"<3\""),
            Some(pf("k", PropertyOp::Eq, "<3")),
            "a quoted value may start with an operator character"
        );
    }

    #[test]
    fn repeated_property_filters_are_kept_once() {
        let st = SearchTerms::from_query_string("%a=1 %a=1 %A=1 %b -%c=2 -%c=2");
        assert_eq!(st.properties, vec![pf("a", PropertyOp::Eq, "1"), has("b")]);
        assert_eq!(st.excluded_properties, vec![pf("c", PropertyOp::Eq, "2")]);
    }

    #[test]
    fn bare_property_key_is_a_has_property_filter() {
        let st = SearchTerms::from_query_string("%Status prop:'due date' -%archived meeting");
        assert_eq!(st.properties, vec![has("status"), has("due date"),]);
        assert_eq!(st.excluded_properties, vec![has("archived")]);
        assert_eq!(st.terms, vec!["meeting"]);
    }

    #[test]
    fn order_by_property() {
        let st = SearchTerms::from_query_string("or:prop:Due -or:%priority ^title");
        assert!(matches!(&st.order_by[0], OrderBy::Property { key, asc: true } if key == "due"));
        assert!(
            matches!(&st.order_by[1], OrderBy::Property { key, asc: false } if key == "priority")
        );
        assert!(matches!(st.order_by[2], OrderBy::Title { asc: true }));
    }

    #[test]
    fn expand_bare_short_note_prefixes() {
        assert_eq!(expand_bare_note_prefixes("<", "{note}"), "<{note}");
        assert_eq!(expand_bare_note_prefixes(">", "{note}"), ">{note}");
        assert_eq!(expand_bare_note_prefixes("=", "{note}"), "={note}");
        assert_eq!(
            expand_bare_note_prefixes("#todo <", "{note}"),
            "#todo <{note}"
        );
        assert_eq!(
            expand_bare_note_prefixes("< #todo", "{note}"),
            "<{note} #todo"
        );
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
        assert_eq!(
            expand_bare_note_prefixes("<projects", "{note}"),
            "<projects"
        );
        assert_eq!(
            expand_bare_note_prefixes(">projects", "{note}"),
            ">projects"
        );
        assert_eq!(
            expand_bare_note_prefixes("=projects", "{note}"),
            "=projects"
        );
        assert_eq!(
            expand_bare_note_prefixes("lk:projects", "{note}"),
            "lk:projects"
        );
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
    fn changing_the_sort_after_an_unclosed_quote_replaces_it() {
        use super::{with_order_directive, OrderField};
        let once = with_order_directive("x\"y '", OrderField::Title, true);
        let twice = with_order_directive(&once, OrderField::Title, false);
        assert_eq!(twice, "x\"y ' -or:title");
    }

    #[test]
    fn sort_directives_with_quoted_keys_are_whole_tokens() {
        use super::{strip_order_directive, with_order_directive, OrderField};
        assert_eq!(strip_order_directive("x ^%\"due date\" y"), "x y");
        assert_eq!(strip_order_directive("-or:prop:'due date' x"), "x");
        assert_eq!(
            with_order_directive("x ^%\"due date\"", OrderField::Title, true),
            "x or:title"
        );
        assert_eq!(
            strip_order_directive("%k=\"a  b\" or:title"),
            "%k=\"a  b\"",
            "a quoted value keeps its inner spaces"
        );
        assert_eq!(
            expand_bare_note_prefixes("^%\"a < b\" <", "{note}"),
            "^%\"a < b\" <{note}",
            "operators inside a quoted sort key are not expanded"
        );
    }

    #[test]
    fn expand_ignores_operators_inside_quoted_property_values() {
        for query in [
            "%flow=\"draft -> review\"",
            "%k='x < y' #a",
            "%\"due date\"=\"a = b\"",
            "-prop:k=\"> spec\"",
        ] {
            assert_eq!(expand_bare_note_prefixes(query, "{note}"), query, "{query}");
        }
        assert_eq!(
            expand_bare_note_prefixes("%\"due date\"<2025-01-01 <", "{note}"),
            "%\"due date\"<2025-01-01 <{note}",
            "a real bare prefix after a quoted property term is still expanded"
        );
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
        println!("{:?}", search_terms);

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
        println!("{:?}", search_terms);

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
        println!("{:?}", search_terms);

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
        println!("{:?}", search_terms);

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
        println!("{:?}", search_terms);

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
        println!("{:?}", search_terms);

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

    #[test]
    fn property_order_round_trips_quoted_key() {
        use super::{with_order_directive, OrderField};
        let q = with_order_directive("#work", OrderField::Property("due date".into()), false);
        assert_eq!(q, "#work -or:prop:\"due date\"");
        let st = SearchTerms::from_query_string(&q);
        match st.order_by.as_slice() {
            [OrderBy::Property { key, asc }] => {
                assert_eq!(key, "due date");
                assert!(!asc);
            }
            other => panic!("expected one property order, got {other:?}"),
        }
    }

    #[test]
    fn property_order_is_replaced_not_piled() {
        use super::{with_order_directive, OrderField};
        let q = with_order_directive("x", OrderField::Property("due".into()), true);
        assert_eq!(q, "x or:prop:due");
        let q = with_order_directive(&q, OrderField::FileName, true);
        assert_eq!(q, "x or:file");
        let q = with_order_directive("x ^%due", OrderField::Property("prio".into()), true);
        assert_eq!(q, "x or:prop:prio");
    }
    // ---- to_query_string / unparsed ----

    /// `parse(to_query_string(parse(q))) == parse(q)`, and a second pass is
    /// stable text.
    fn assert_round_trips(q: &str) {
        let parsed = SearchTerms::from_query_string(q);
        let written = parsed.to_query_string();
        let reparsed = SearchTerms::from_query_string(&written);
        assert_eq!(reparsed, parsed, "query {q:?} written as {written:?}");
        assert_eq!(reparsed.to_query_string(), written, "unstable for {q:?}");
    }

    #[test]
    fn round_trip_corpus() {
        for q in [
            "",
            "meeting",
            "meeting notes",
            "\"two words\" other",
            "'it has \"quotes\"' x",
            "-cancelled",
            "-\"two words\"",
            "\"-dash first\"",
            "\"#notatag\" -\"#nottag\"",
            "\"name:literal\"",
            "\"or:literal\"",
            "kimu*",
            "@meeting -@temp",
            "in:meeting",
            "=tasks -=drafts",
            "name:\"my note\"",
            "/journal/2024 -/archive",
            "pt:\"my folder/sub\"",
            "#finance #Finance -#draft",
            "lb:\"my tag\"",
            "<projects >projects <work/projects",
            "lk:spec -lk:old fwd:index -fwd:gone",
            "<\"my note\" ->\"my note\"",
            "<{note} >{note} ={note}",
            "%due",
            "-%due",
            "%priority>=2 %priority<9 %status!=done %rank>1 %rank<=7",
            "%status=\"in progress\"",
            "%\"due date\"<2025-01-01",
            "%\"due date\"=\"end of year\"",
            "%\"due date\"",
            "%tags=proj*",
            "%tags=\"proj*\"",
            "%title=\"=x\"",
            "prop:status=done -prop:status=wip",
            "x ^title",
            "x -^title",
            "x ^file",
            "x ^%due",
            "x -^%due",
            "x ^%\"due date\"",
            "x or:title -or:prop:rank",
            "meeting #urgent -#draft @work =tasks /journal <a >b %p>1 ^file",
            "caf\u{e9} #T\u{e9}l\u{e9}",
        ] {
            assert_round_trips(q);
        }
    }

    #[test]
    fn to_query_string_uses_short_forms_and_groups() {
        let st =
            SearchTerms::from_query_string("lb:urgent -name:\"my note\" meeting prop:due ^title");
        assert_eq!(
            st.to_query_string(),
            "meeting #urgent -=\"my note\" %due ^title"
        );
        assert_eq!(SearchTerms::default().to_query_string(), "");
    }

    #[test]
    fn terms_that_look_like_syntax_are_quoted() {
        let st = SearchTerms {
            terms: vec!["#x".into(), "-y".into(), "name:z".into(), "or:t".into()],
            excluded_terms: vec!["#x".into(), "plain".into()],
            ..Default::default()
        };
        let written = st.to_query_string();
        assert_eq!(SearchTerms::from_query_string(&written), st, "{written}");
    }

    #[test]
    fn values_holding_both_quote_kinds_do_not_panic() {
        let st = SearchTerms {
            terms: vec!["a \"b\" 'c'".into()],
            ..Default::default()
        };
        let _ = st.to_query_string();
    }

    #[test]
    fn unparsed_keeps_discarded_tokens_verbatim() {
        let st = SearchTerms::from_query_string("a # lk: %status=");
        assert_eq!(st.terms, vec!["a"]);
        assert_eq!(st.unparsed, vec!["#", "lk:", "%status="]);
        assert!(st.labels.is_empty() && st.links.is_empty() && st.properties.is_empty());

        // An unknown sort key and a malformed operator.
        let st = SearchTerms::from_query_string("^bogus %k<>x");
        assert_eq!(st.unparsed, vec!["^bogus", "%k<>x"]);
        assert!(st.order_by.is_empty() && st.properties.is_empty());

        // Empty quotes.
        let st = SearchTerms::from_query_string("\"\" b");
        assert_eq!(st.terms, vec!["b"]);
        assert_eq!(st.unparsed, vec!["\"\""]);
    }

    #[test]
    fn unterminated_quote_runs_to_the_end_and_is_kept() {
        let st = SearchTerms::from_query_string("a #t \"open quote #x");
        assert_eq!(st.terms, vec!["a"]);
        assert_eq!(st.labels, vec!["t"]);
        assert_eq!(st.unparsed, vec!["\"open quote #x"]);
        assert_eq!(st.to_query_string(), "a #t \"open quote #x");
        assert_round_trips("a #t \"open quote #x");
    }

    #[test]
    fn clean_queries_have_no_unparsed_tokens() {
        let st = SearchTerms::from_query_string("a #b <c %d=e ^title -x");
        assert!(st.unparsed.is_empty());
    }

    #[test]
    fn unparsed_does_not_change_what_the_query_matches() {
        // Everything else is identical with or without the dead tokens.
        let clean = SearchTerms::from_query_string("a #b");
        let mut with_junk = SearchTerms::from_query_string("a # #b lk:");
        with_junk.unparsed.clear();
        assert_eq!(clean, with_junk);
    }

    #[test]
    fn round_trip_keeps_unparsed_tokens() {
        for q in ["# lk:", "a ^bogus b", "%k<>x #t", "x \"\" y"] {
            assert_round_trips(q);
        }
    }

    #[test]
    fn oversized_queries_are_still_truncated() {
        let long = "a ".repeat(super::MAX_QUERY_LEN);
        let st = SearchTerms::from_query_string(&long);
        assert!(st.to_query_string().len() <= super::MAX_QUERY_LEN);
    }

    #[test]
    fn round_trip_when_a_token_is_only_partly_consumed() {
        // Found by fuzzing: an empty quote pair after a prefix ends the token
        // early, and the rest is a token of its own.
        for q in ["-\"\"q w\"", "-''<", "-''=<=", "#\"\"x", "%\"\"a b"] {
            assert_round_trips(q);
        }
    }

    /// A seeded fuzz over glued-together fragments of the syntax: whatever the
    /// parser makes of them, writing the result back and parsing again gives
    /// the same terms. Property filters (`%`) are left out: a key or value
    /// that itself starts with a quote can't always be written back.
    #[test]
    fn round_trip_holds_for_random_fragments() {
        const PIECES: [&str; 20] = [
            "a", "b c", "#", "-", "<", ">", "=", "lk:", "name:", "\"", "'", "^", "title", "x*",
            "@", "/", " ", "or:", "-^", "\"\"",
        ];
        let mut seed: u64 = 0x2545_f491_4f6c_dd1d;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        for _ in 0..30_000 {
            let len = 1 + (next() % 7) as usize;
            let q: String = (0..len)
                .map(|_| PIECES[(next() % PIECES.len() as u64) as usize])
                .collect();
            assert_round_trips(&q);
        }
    }

    #[test]
    fn property_filters_built_from_fields_match_what_parsing_gives() {
        let built = PropertyFilter::compare("Due Date", PropertyOp::Ge, " 2025-01-01 ").unwrap();
        let parsed = PropertyFilter::parse("\"Due Date\">=2025-01-01").unwrap();
        assert_eq!(built, parsed);
        let built = PropertyFilter::compare("Status", PropertyOp::Eq, "Do*").unwrap();
        assert_eq!(built, PropertyFilter::parse("status=do*").unwrap());
        assert_eq!(
            PropertyFilter::exists("R\u{e9}sum\u{e9}").unwrap(),
            PropertyFilter::parse("resume").unwrap()
        );
        assert_eq!(PropertyFilter::compare("k", PropertyOp::Eq, "  "), None);
        assert_eq!(PropertyFilter::compare("", PropertyOp::Eq, "v"), None);
        assert_eq!(PropertyFilter::exists("  "), None);
    }
}
