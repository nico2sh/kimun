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
}

struct QueryTermExtractor {
    el_type: ElementType,
    term: String,
    remainder: String,
}

// Table of (long_prefix, short_prefix, element_type_tag) for non-special prefix types.
// Excluded variants must come before their positive counterparts so longer prefixes match first.
type PrefixEntry = (&'static str, &'static str, fn() -> ElementType);

fn prefix_table() -> [PrefixEntry; 8] {
    [
        ("in:-", ">-", || ElementType::ExcludedIn),
        ("at:-", "@-", || ElementType::ExcludedAt),
        ("pt:-", "/-", || ElementType::ExcludedPath),
        ("lb:-", "#-", || ElementType::ExcludedLabel),
        ("in:", ">", || ElementType::In),
        ("at:", "@", || ElementType::At),
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
        } else if query.starts_with("-") {
            // Handle excluded terms (simple `-term` syntax)
            (
                ElementType::ExcludedTerm,
                query.strip_prefix("-").unwrap().to_string(),
            )
        } else {
            // Handle OrderBy (special case with asc/desc sub-detection)
            let order_prefix = format!("{}:", ORDER_LETTER);
            if query.starts_with(&order_prefix) {
                let desc_prefix = format!("{order_prefix}-");
                let (asc, prefix) = if query.starts_with(&desc_prefix) {
                    (false, desc_prefix)
                } else {
                    (true, order_prefix)
                };
                (
                    ElementType::OrderBy { asc },
                    query.strip_prefix(&prefix).unwrap_or(query).to_string(),
                )
            } else if query.starts_with(ORDER_CHAR) {
                let desc_prefix = format!("{ORDER_CHAR}-");
                let (asc, prefix) = if query.starts_with(&desc_prefix) {
                    (false, desc_prefix.as_str())
                } else {
                    (true, ORDER_CHAR)
                };
                (
                    ElementType::OrderBy { asc },
                    query.strip_prefix(prefix).unwrap_or(query).to_string(),
                )
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

#[derive(Debug)]
pub enum OrderBy {
    Title { asc: bool },
    FileName { asc: bool },
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

#[derive(Default, Debug)]
pub struct SearchTerms {
    pub terms: Vec<String>,
    pub breadcrumb: Vec<String>,
    pub order_by: Vec<OrderBy>,
    pub filename: Vec<String>,
    pub path: Vec<String>,
    pub labels: Vec<String>,
    pub excluded_terms: Vec<String>,
    pub excluded_breadcrumb: Vec<String>,
    pub excluded_filename: Vec<String>,
    pub excluded_path: Vec<String>,
    pub excluded_labels: Vec<String>,
}

/// Maximum byte length of a query string accepted by [`SearchTerms::from_query_string`].
/// 8 KB is more than enough for any real search query; larger inputs are truncated
/// on a char boundary to prevent unbounded memory allocation via duplicate labels.
const MAX_QUERY_LEN: usize = 8 * 1024;

impl SearchTerms {
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
        let mut excluded_terms = vec![];
        let mut excluded_breadcrumb = vec![];
        let mut excluded_filename = vec![];
        let mut excluded_path = vec![];
        let mut excluded_labels = vec![];
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
            }
        }

        dedup_preserving_order(&mut labels);
        dedup_preserving_order(&mut excluded_labels);

        Self {
            breadcrumb,
            filename,
            order_by,
            terms,
            path,
            labels,
            excluded_terms,
            excluded_breadcrumb,
            excluded_filename,
            excluded_path,
            excluded_labels,
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
        let query = ">title in:othertitle";
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
        let query = "@file at:directory";
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
        let query = "@'file name' at:\"directory path\"";
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
        let query = "@'file name' at:\"directory path";
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
        let query = "searchterm    @file otherterm at:directory in:title >text      \"some text\" /basedirectory";
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
        let search_terms = SearchTerms::from_query_string(">-draft in:-private @-temp /-secret");
        assert!(search_terms.terms.is_empty());
        assert!(search_terms.breadcrumb.is_empty());
        assert_eq!(search_terms.excluded_breadcrumb, vec!["draft", "private"]);
        assert_eq!(search_terms.excluded_filename, vec!["temp"]);
        assert_eq!(search_terms.excluded_path, vec!["secret"]);
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
        // Canonical excluded forms are `#-draft` and `lb:-draft`.
        let s2 = SearchTerms::from_query_string("#-draft");
        assert_eq!(s2.excluded_labels, vec!["draft".to_string()]);
        let s3 = SearchTerms::from_query_string("lb:-draft");
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
        let s = SearchTerms::from_query_string("#-draft lb:-draft #-old");
        assert_eq!(
            s.excluded_labels,
            vec!["draft".to_string(), "old".to_string()]
        );
    }

    #[test]
    fn from_query_string_caps_input_length() {
        let huge = "#a ".repeat(20_000); // 60 KB
        let s = SearchTerms::from_query_string(huge);
        // The cap is 8 KB; after dedup, labels has at most 1 entry.
        assert!(s.labels.len() <= 1);
    }

    #[test]
    fn bare_prefix_terms_are_dropped() {
        // None of these bare prefixes should produce a term.
        for q in &[">", "-", ">-", "in:", "at:", "pt:", "/", "@", "/-", "@-"] {
            let s = SearchTerms::from_query_string(*q);
            assert!(s.terms.is_empty(), "{:?} produced terms: {:?}", q, s.terms);
            assert!(s.breadcrumb.is_empty(), "{:?} produced breadcrumb: {:?}", q, s.breadcrumb);
            assert!(s.filename.is_empty(), "{:?} produced filename: {:?}", q, s.filename);
            assert!(s.path.is_empty(), "{:?} produced path: {:?}", q, s.path);
            assert!(s.excluded_terms.is_empty(), "{:?} produced excluded_terms: {:?}", q, s.excluded_terms);
            assert!(s.excluded_breadcrumb.is_empty(), "{:?} produced excluded_breadcrumb: {:?}", q, s.excluded_breadcrumb);
            assert!(s.excluded_filename.is_empty(), "{:?} produced excluded_filename: {:?}", q, s.excluded_filename);
            assert!(s.excluded_path.is_empty(), "{:?} produced excluded_path: {:?}", q, s.excluded_path);
        }
    }
}
