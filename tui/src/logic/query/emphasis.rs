//! What a query implies for presentation, without styling it: the needles a
//! result's content matched on, and the query's parse problem.

/// The lowercase emphasis needles a query implies: its plain text terms,
/// labels in their in-body `#tag` form, and link targets — what a result's
/// content actually matched on. Shared by the telescope preview and the
/// editor's arrive-from-query emphasis.
pub fn emphasis_needles(query: &str) -> Vec<String> {
    let terms = kimun_core::SearchTerms::from_query_string(query);
    let mut needles: Vec<String> = terms
        .terms
        .iter()
        .map(|t| t.to_lowercase())
        .chain(
            terms
                .labels
                .iter()
                .map(|l| format!("#{}", l.to_lowercase())),
        )
        .chain(terms.links.iter().map(|l| l.to_lowercase()))
        .chain(terms.forward_links.iter().map(|l| l.to_lowercase()))
        .filter(|t| !t.is_empty() && !t.contains('{'))
        .collect();
    needles.sort_unstable();
    needles.dedup();
    needles
}

/// The one-line reason for the query's parse problem, if any — surfaced in
/// the FIND header. The lenient grammar's only real error is an unterminated
/// quote.
pub fn error_reason(query: &str) -> Option<&'static str> {
    kimun_core::query_has_unterminated_quote(query).then_some("unterminated quote")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_reason_only_for_unterminated() {
        assert_eq!(error_reason(r#"a "open"#), Some("unterminated quote"));
        assert_eq!(error_reason(r#"a "closed""#), None);
    }
}
