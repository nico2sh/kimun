//! The search query, toolkit-free: query variables and the context they
//! resolve against ([`QueryContext`], [`resolve_query`]), the saved-search
//! breadcrumb ([`SavedSearchBreadcrumb`]), the emphasis needles a query
//! implies ([`emphasis_needles`]) and its parse problem ([`error_reason`]),
//! the Query panel's rows and their loader ([`BacklinkEntry`],
//! [`BacklinkSource`], [`load_query`]), and the query-syntax reference card
//! ([`SYNTAX`]). The order directive helpers are in
//! [`crate::logic::sort`]. See CONTEXT.md (Query panel, Query variable,
//! Saved search) and adr/0049.

mod breadcrumb;
mod emphasis;
mod find;
mod syntax;
mod vars;

pub use self::breadcrumb::SavedSearchBreadcrumb;
pub use self::emphasis::{emphasis_needles, error_reason};
pub use self::find::{
    BacklinkEntry, BacklinkSource, DEFAULT_QUERY, DEFAULT_QUERY_LONG, QueryHit, QueryPage,
    QueryPageSource, cut_context, extract_context_multi, is_default_query, load_query,
    load_query_page, split_paragraphs,
};
pub use self::syntax::{SYNTAX, SyntaxSection};
pub use self::vars::{
    QueryContext, VAR_NOTE, query_has_variables, query_is_unresolvable, resolve_query,
};
