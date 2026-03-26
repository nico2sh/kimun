use std::vec;

use log::debug;

const IN_CHAR: &str = ">";
const IN_LETTER: &str = "in";
const AT_CHAR: &str = "@";
const AT_LETTER: &str = "at";
const ORDER_CHAR: &str = "^";
const ORDER_LETTER: &str = "or";
const PATH_CHAR: &str = "/";
const PATH_LETTER: &str = "pt";

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
}

struct QueryTermExtractor {
    el_type: ElementType,
    term: String,
    remainder: String,
}

impl QueryTermExtractor {
    fn extract_and_consume<S: AsRef<str>>(query: S) -> QueryTermExtractor {
        let query = query.as_ref().trim();
        let in_prefix = format!("{}:", IN_LETTER);
        let at_prefix = format!("{}:", AT_LETTER);
        let order_prefix = format!("{}:", ORDER_LETTER);
        let path_prefix = format!("{}:", PATH_LETTER);
        let excluded_in_prefix = format!("-{}:", IN_LETTER);
        let excluded_at_prefix = format!("-{}:", AT_LETTER);
        let excluded_path_prefix = format!("-{}:", PATH_LETTER);

        let (element_type, remaining) = if query.starts_with(&excluded_in_prefix) {
            (
                ElementType::ExcludedIn,
                query
                    .strip_prefix(&excluded_in_prefix)
                    .map_or_else(|| query.to_string(), |s| s.to_string()),
            )
        } else if query.starts_with("-in:") {
            (
                ElementType::ExcludedIn,
                query
                    .strip_prefix("-in:")
                    .map_or_else(|| query.to_string(), |s| s.to_string()),
            )
        } else if query.starts_with("-") && query.starts_with(&format!("-{}", IN_CHAR)) {
            (
                ElementType::ExcludedIn,
                query
                    .strip_prefix(&format!("-{}", IN_CHAR))
                    .map_or_else(|| query.to_string(), |s| s.to_string()),
            )
        } else if query.starts_with(&excluded_at_prefix) {
            (
                ElementType::ExcludedAt,
                query
                    .strip_prefix(&excluded_at_prefix)
                    .map_or_else(|| query.to_string(), |s| s.to_string()),
            )
        } else if query.starts_with("-at:") {
            (
                ElementType::ExcludedAt,
                query
                    .strip_prefix("-at:")
                    .map_or_else(|| query.to_string(), |s| s.to_string()),
            )
        } else if query.starts_with("-") && query.starts_with(&format!("-{}", AT_CHAR)) {
            (
                ElementType::ExcludedAt,
                query
                    .strip_prefix(&format!("-{}", AT_CHAR))
                    .map_or_else(|| query.to_string(), |s| s.to_string()),
            )
        } else if query.starts_with(&excluded_path_prefix) {
            (
                ElementType::ExcludedPath,
                query
                    .strip_prefix(&excluded_path_prefix)
                    .map_or_else(|| query.to_string(), |s| s.to_string()),
            )
        } else if query.starts_with("-pt:") {
            (
                ElementType::ExcludedPath,
                query
                    .strip_prefix("-pt:")
                    .map_or_else(|| query.to_string(), |s| s.to_string()),
            )
        } else if query.starts_with("-") && query.starts_with(&format!("-{}", PATH_CHAR)) {
            (
                ElementType::ExcludedPath,
                query
                    .strip_prefix(&format!("-{}", PATH_CHAR))
                    .map_or_else(|| query.to_string(), |s| s.to_string()),
            )
        } else if query.starts_with("-") {
            // Handle excluded terms (simple `-term` syntax)
            (
                ElementType::ExcludedTerm,
                query
                    .strip_prefix("-")
                    .map_or_else(|| query.to_string(), |s| s.to_string()),
            )
        } else if query.starts_with(&in_prefix) {
            (
                ElementType::In,
                query
                    .strip_prefix(&in_prefix)
                    .map_or_else(|| query.to_string(), |s| s.to_string()),
            )
        } else if query.starts_with(IN_CHAR) {
            (
                ElementType::In,
                query
                    .strip_prefix(IN_CHAR)
                    .map_or_else(|| query.to_string(), |s| s.to_string()),
            )
        } else if query.starts_with(&at_prefix) {
            (
                ElementType::At,
                query
                    .strip_prefix(&at_prefix)
                    .map_or_else(|| query.to_string(), |s| s.to_string()),
            )
        } else if query.starts_with(AT_CHAR) {
            (
                ElementType::At,
                query
                    .strip_prefix(AT_CHAR)
                    .map_or_else(|| query.to_string(), |s| s.to_string()),
            )
        } else if query.starts_with(&order_prefix) {
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
        } else if query.starts_with(&path_prefix) {
            (
                ElementType::Path,
                query
                    .strip_prefix(&path_prefix)
                    .map_or_else(|| query.to_string(), |s| s.to_string()),
            )
        } else if query.starts_with(PATH_CHAR) {
            (
                ElementType::Path,
                query
                    .strip_prefix(PATH_CHAR)
                    .map_or_else(|| query.to_string(), |s| s.to_string()),
            )
        } else {
            (ElementType::Term, query.to_string())
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
    pub excluded_terms: Vec<String>,
    pub excluded_breadcrumb: Vec<String>,
    pub excluded_filename: Vec<String>,
    pub excluded_path: Vec<String>,
}

impl SearchTerms {
    pub fn from_query_string<S: AsRef<str>>(query: S) -> Self {
        let mut query = query.as_ref().to_string();
        let mut breadcrumb = vec![];
        let mut terms = vec![];
        let mut filename = vec![];
        let mut order_by = vec![];
        let mut path = vec![];
        let mut excluded_terms = vec![];
        let mut excluded_breadcrumb = vec![];
        let mut excluded_filename = vec![];
        let mut excluded_path = vec![];
        while !query.is_empty() {
            let qp = QueryTermExtractor::extract_and_consume(query);
            query = qp.remainder;
            match qp.el_type {
                ElementType::Term => terms.push(qp.term),
                ElementType::In => breadcrumb.push(qp.term),
                ElementType::At => filename.push(qp.term),
                ElementType::OrderBy { asc } => {
                    if let Some(o) = OrderBy::from_term(&qp.term, asc) {
                        order_by.push(o);
                    }
                }
                ElementType::Invalid => {}
                ElementType::Path => path.push(qp.term),
                ElementType::ExcludedTerm => excluded_terms.push(qp.term),
                ElementType::ExcludedIn => excluded_breadcrumb.push(qp.term),
                ElementType::ExcludedAt => excluded_filename.push(qp.term),
                ElementType::ExcludedPath => excluded_path.push(qp.term),
            }
        }

        Self {
            breadcrumb,
            filename,
            order_by,
            terms,
            path,
            excluded_terms,
            excluded_breadcrumb,
            excluded_filename,
            excluded_path,
        }
    }
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
}
