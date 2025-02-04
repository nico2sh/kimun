use log::debug;

const IN_CHAR: &str = ">";
const IN_LETTER: &str = "in";
const AT_CHAR: &str = "@";
const AT_LETTER: &str = "at";

enum ElementType {
    Invalid,
    Term,
    In,
    At,
}

enum ElementCloseChar {
    Quote,
    SingleQuote,
    Space,
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

        let (element_type, remaining) = if query.starts_with(&in_prefix) {
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

#[derive(Default, Debug)]
pub struct SearchTerms {
    pub terms: Vec<String>,
    pub breadcrumb: Vec<String>,
    pub path: Vec<String>,
}

impl SearchTerms {
    pub fn from_query_string<S: AsRef<str>>(query: S) -> Self {
        let mut query = query.as_ref().to_string();
        let mut breadcrumb = vec![];
        let mut terms = vec![];
        let mut path = vec![];
        while !query.is_empty() {
            let qp = QueryTermExtractor::extract_and_consume(query);
            query = qp.remainder;
            match qp.el_type {
                ElementType::Term => terms.push(qp.term),
                ElementType::In => breadcrumb.push(qp.term),
                ElementType::At => path.push(qp.term),
                ElementType::Invalid => {}
            }
        }

        Self {
            breadcrumb,
            path,
            terms,
        }
    }

    pub fn under<S: AsRef<str>>(mut self, term: S) -> Self {
        self.breadcrumb.push(term.as_ref().to_string());
        self
    }

    pub fn inside<S: AsRef<str>>(mut self, term: S) -> Self {
        self.path.push(term.as_ref().to_string());
        self
    }

    pub fn with_text<S: AsRef<str>>(mut self, term: S) -> Self {
        self.terms.push(term.as_ref().to_string());
        self
    }

    pub fn get_query_cond(&self) -> (String, Vec<String>) {
        let mut cond = vec![];
        let mut var_num = 1;
        let mut values = vec![];
        if !self.terms.is_empty() {
            cond.push(format!("notesContent.text MATCH ?{}", var_num));
            values.push(self.terms.join(" "));
            var_num += 1;
        }
        if !self.path.is_empty() {
            cond.push(format!("notesContent.path MATCH ?{}", var_num));
            values.push(self.path.join(" "));
            var_num += 1;
        }
        if !self.breadcrumb.is_empty() {
            cond.push(format!("notesContent.breadcrumb MATCH ?{}", var_num));
            values.push(self.breadcrumb.join(" "));
        }

        (cond.join(" AND "), values)
    }
}

#[cfg(test)]
mod tests {
    use super::SearchTerms;

    #[test]
    fn get_conditions() {
        let search_terms = SearchTerms {
            terms: vec!["some".to_string(), "text".to_string()],
            path: vec!["file".to_string()],
            breadcrumb: vec!["title".to_string(), "more_title".to_string()],
        };

        let (cond, terms) = search_terms.get_query_cond();
        println!("condition: {}", cond);
        println!("terms: {:?}", terms);
    }

    #[test]
    fn search_terms() {
        let query = "some text more terms";
        let search_terms = SearchTerms::from_query_string(query);
        println!("{:?}", &search_terms);

        let breadcrumb = search_terms.breadcrumb;
        let path = search_terms.path;
        let terms = search_terms.terms;

        assert!(breadcrumb.is_empty());
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
        let path = search_terms.path;
        let terms = search_terms.terms;

        assert!(!breadcrumb.is_empty());
        assert!(path.is_empty());
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
        let path = search_terms.path;
        let terms = search_terms.terms;

        assert!(breadcrumb.is_empty());
        assert!(!path.is_empty());
        assert!(terms.is_empty());
        assert_eq!(2, path.len());
        assert!(path.contains(&"file".to_string()));
        assert!(path.contains(&"directory".to_string()));
    }

    #[test]
    fn search_at_quoted() {
        let query = "@'file name' at:\"directory path\"";
        let search_terms = SearchTerms::from_query_string(query);
        println!("{:?}", &search_terms);

        let breadcrumb = search_terms.breadcrumb;
        let path = search_terms.path;
        let terms = search_terms.terms;

        assert!(breadcrumb.is_empty());
        assert!(!path.is_empty());
        assert!(terms.is_empty());
        assert_eq!(2, path.len());
        assert!(path.contains(&"file name".to_string()));
        assert!(path.contains(&"directory path".to_string()));
    }

    #[test]
    fn search_at_quoted_not_closed() {
        let query = "@'file name' at:\"directory path";
        let search_terms = SearchTerms::from_query_string(query);
        println!("{:?}", &search_terms);

        let breadcrumb = search_terms.breadcrumb;
        let path = search_terms.path;
        let terms = search_terms.terms;

        assert!(breadcrumb.is_empty());
        assert!(!path.is_empty());
        assert!(terms.is_empty());
        assert_eq!(1, path.len());
        assert!(path.contains(&"file name".to_string()));
    }

    #[test]
    fn search_combined() {
        let query = "searchterm    @file otherterm at:directory in:title >text      \"some text\"";
        let search_terms = SearchTerms::from_query_string(query);
        println!("{:?}", &search_terms);

        let breadcrumb = search_terms.breadcrumb;
        let path = search_terms.path;
        let terms = search_terms.terms;

        assert!(!breadcrumb.is_empty());
        assert!(!path.is_empty());
        assert!(!terms.is_empty());
        assert_eq!(3, terms.len());
        assert!(terms.contains(&"searchterm".to_string()));
        assert!(terms.contains(&"otherterm".to_string()));
        assert!(terms.contains(&"some text".to_string()));
        assert_eq!(2, breadcrumb.len());
        assert!(breadcrumb.contains(&"title".to_string()));
        assert!(breadcrumb.contains(&"text".to_string()));
        assert_eq!(2, path.len());
        assert!(path.contains(&"file".to_string()));
        assert!(path.contains(&"directory".to_string()));
    }
}
