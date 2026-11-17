//! The query builder's model, toolkit-free: a query as a list of typed rows
//! (a tag, a folder, a property …) and a sort, that turns into a query string
//! and back. It is a view over [`SearchTerms`]: parsing and writing are core's
//! (`from_query_string`, `to_query_string`), so quoting and the operator
//! vocabulary live in one place. What this adds is the row shape a form can
//! edit, and that nothing is lost on the way through: whatever a row can't
//! express is a [`RowKind::Raw`] row, written back as it was.
//!
//! The sort is the query's own order directive, read and written with the
//! helpers of [`crate::logic::sort`] — the same ones the sort dialog uses.

use kimun_core::{PropertyFilter, PropertyOp, PropertyTest, SearchTerms};

use crate::logic::sort::{SortField, SortOrder, directive_of_query, query_with_sort};

/// What a row asks of a note.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowKind {
    /// Full-text: the note contains the words.
    Text,
    /// The note's file name matches (`=`).
    Name,
    /// A section heading matches (`@`).
    Heading,
    /// The note's path matches, a folder or part of one (`/`).
    Folder,
    /// The note has the tag (`#`).
    Tag,
    /// The note links to the named one (`<`).
    LinksTo,
    /// The note is linked from the named one (`>`).
    LinkedFrom,
    /// A property test (`%`).
    Property,
    /// Query text no other row expresses, kept as written.
    Raw,
}

impl RowKind {
    /// Every kind, in the order a type picker lists them.
    pub const ALL: [RowKind; 9] = [
        RowKind::Text,
        RowKind::Name,
        RowKind::Heading,
        RowKind::Folder,
        RowKind::Tag,
        RowKind::LinksTo,
        RowKind::LinkedFrom,
        RowKind::Property,
        RowKind::Raw,
    ];

    /// The kind's name in a picker.
    pub fn label(self) -> &'static str {
        match self {
            RowKind::Text => "Text",
            RowKind::Name => "Note name",
            RowKind::Heading => "Heading",
            RowKind::Folder => "Folder",
            RowKind::Tag => "Tag",
            RowKind::LinksTo => "Links to",
            RowKind::LinkedFrom => "Linked from",
            RowKind::Property => "Property",
            RowKind::Raw => "Raw",
        }
    }

    /// Whether the row can be turned into "must not" (a raw row is as written).
    pub fn can_exclude(self) -> bool {
        self != RowKind::Raw
    }
}

/// One condition of the query.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuilderRow {
    pub kind: RowKind,
    /// "Must not": the note must *not* match (`-`).
    pub exclude: bool,
    /// What is matched: the text, name, folder, tag or note. A property's
    /// value. The whole text of a raw row.
    pub value: String,
    /// A property's key. Unused by the other kinds.
    pub key: String,
    /// A property's comparison; `None` is "has the property", with no value.
    pub op: Option<PropertyOp>,
}

impl BuilderRow {
    /// An empty row of `kind`.
    pub fn new(kind: RowKind) -> Self {
        Self {
            kind,
            exclude: false,
            value: String::new(),
            key: String::new(),
            op: (kind == RowKind::Property).then_some(PropertyOp::Eq),
        }
    }

    /// A row of `kind` holding `value`.
    pub fn with_value(kind: RowKind, value: impl Into<String>) -> Self {
        Self {
            value: value.into(),
            ..Self::new(kind)
        }
    }

    /// Whether the row says enough to be part of the query: a value, and for a
    /// property a key (and a value, unless it only asks for the property).
    /// Incomplete rows stay on screen and are left out of the query.
    pub fn is_complete(&self) -> bool {
        let has_value = !self.value.trim().is_empty();
        match self.kind {
            RowKind::Property => !self.key.trim().is_empty() && (self.op.is_none() || has_value),
            _ => has_value,
        }
    }

    /// The property filter a property row stands for.
    fn property_filter(&self) -> Option<PropertyFilter> {
        match self.op {
            None => PropertyFilter::exists(&self.key),
            Some(op) => PropertyFilter::compare(&self.key, op, &self.value),
        }
    }
}

/// The comparisons a property row offers, `None` (has the property) first.
pub const PROPERTY_OPS: [Option<PropertyOp>; 7] = [
    None,
    Some(PropertyOp::Eq),
    Some(PropertyOp::Ne),
    Some(PropertyOp::Lt),
    Some(PropertyOp::Le),
    Some(PropertyOp::Gt),
    Some(PropertyOp::Ge),
];

/// A comparison's name in a picker; `None` is "has the property".
pub fn property_op_label(op: Option<PropertyOp>) -> &'static str {
    match op {
        None => "exists",
        Some(PropertyOp::Eq) => "=",
        Some(PropertyOp::Ne) => "≠",
        Some(PropertyOp::Lt) => "<",
        Some(PropertyOp::Le) => "≤",
        Some(PropertyOp::Gt) => ">",
        Some(PropertyOp::Ge) => "≥",
    }
}

/// A query as rows and a sort.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct BuilderModel {
    pub rows: Vec<BuilderRow>,
    /// How the results are ordered; `None` leaves the default order.
    pub sort: Option<(SortField, SortOrder)>,
}

impl BuilderModel {
    /// The rows of `query`. Every part of it is kept: what has no row of its
    /// own (a token the parser can't use, a second sort, a property value with
    /// a literal `*`) becomes a raw row.
    pub fn from_query(query: &str) -> Self {
        let terms = SearchTerms::from_query_string(query);
        let mut rows = Vec::new();
        let mut valued = |kind: RowKind, include: &[String], exclude: &[String]| {
            for (values, exclude) in [(include, false), (exclude, true)] {
                rows.extend(values.iter().map(|v| BuilderRow {
                    exclude,
                    ..BuilderRow::with_value(kind, v.clone())
                }));
            }
        };
        valued(RowKind::Text, &terms.terms, &terms.excluded_terms);
        valued(RowKind::Name, &terms.filename, &terms.excluded_filename);
        valued(
            RowKind::Heading,
            &terms.breadcrumb,
            &terms.excluded_breadcrumb,
        );
        valued(RowKind::Folder, &terms.path, &terms.excluded_path);
        valued(RowKind::Tag, &terms.labels, &terms.excluded_labels);
        valued(RowKind::LinksTo, &terms.links, &terms.excluded_links);
        valued(
            RowKind::LinkedFrom,
            &terms.forward_links,
            &terms.excluded_forward_links,
        );
        for (filters, exclude) in [
            (&terms.properties, false),
            (&terms.excluded_properties, true),
        ] {
            for filter in filters {
                rows.push(property_row(filter, exclude));
            }
        }
        // The first sort is the sort control's; any further one has no place
        // in a form, so it is kept as written.
        for extra in terms.order_by.iter().skip(1) {
            let only = SearchTerms {
                order_by: vec![extra.clone()],
                ..SearchTerms::default()
            };
            rows.push(BuilderRow::with_value(RowKind::Raw, only.to_query_string()));
        }
        rows.extend(
            terms
                .unparsed
                .iter()
                .map(|raw| BuilderRow::with_value(RowKind::Raw, raw.clone())),
        );
        Self {
            rows,
            sort: directive_of_query(query),
        }
    }

    /// The terms the complete rows stand for, but for the raw rows.
    fn terms(&self) -> SearchTerms {
        let mut t = SearchTerms::default();
        for row in self.rows.iter().filter(|r| r.is_complete()) {
            let value = row.value.trim().to_string();
            let (include, exclude) = match row.kind {
                RowKind::Text => (&mut t.terms, &mut t.excluded_terms),
                RowKind::Name => (&mut t.filename, &mut t.excluded_filename),
                RowKind::Heading => (&mut t.breadcrumb, &mut t.excluded_breadcrumb),
                RowKind::Folder => (&mut t.path, &mut t.excluded_path),
                // Tags are matched in lowercase, as the parser reads them.
                RowKind::Tag => {
                    let value = value.to_lowercase();
                    let list = if row.exclude {
                        &mut t.excluded_labels
                    } else {
                        &mut t.labels
                    };
                    list.push(value);
                    continue;
                }
                RowKind::LinksTo => (&mut t.links, &mut t.excluded_links),
                RowKind::LinkedFrom => (&mut t.forward_links, &mut t.excluded_forward_links),
                RowKind::Property => {
                    if let Some(filter) = row.property_filter() {
                        let list = if row.exclude {
                            &mut t.excluded_properties
                        } else {
                            &mut t.properties
                        };
                        list.push(filter);
                    }
                    continue;
                }
                // Written after the sort, as they were (see `to_query`).
                RowKind::Raw => continue,
            };
            if row.exclude { exclude } else { include }.push(value);
        }
        t
    }

    /// The query the complete rows and the sort say. The raw rows come last,
    /// as written: setting the sort strips every order directive from the
    /// text it is given, so a raw second sort must be added after it, and an
    /// unterminated quote runs to the end of the query so it belongs there.
    pub fn to_query(&self) -> String {
        let text = self.terms().to_query_string();
        let text = match &self.sort {
            Some((field, order)) => query_with_sort(&text, field, *order).unwrap_or(text),
            None => text,
        };
        let raw = self
            .rows
            .iter()
            .filter(|r| r.kind == RowKind::Raw && r.is_complete())
            .map(|r| r.value.trim());
        std::iter::once(text.as_str())
            .chain(raw)
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// How many rows are left out of the query for being incomplete.
    pub fn incomplete(&self) -> usize {
        self.rows.iter().filter(|r| !r.is_complete()).count()
    }
}

/// The row for a property filter. A literal `*` in a value (a quoted one) is
/// not a form value — a form's `*` is a wildcard — so that filter stays raw.
fn property_row(filter: &PropertyFilter, exclude: bool) -> BuilderRow {
    match &filter.test {
        PropertyTest::Exists => BuilderRow {
            exclude,
            key: filter.key.clone(),
            op: None,
            ..BuilderRow::new(RowKind::Property)
        },
        PropertyTest::Compare {
            op,
            value,
            wildcard,
        } if *wildcard || !value.contains('*') => BuilderRow {
            exclude,
            key: filter.key.clone(),
            op: Some(*op),
            value: value.clone(),
            ..BuilderRow::new(RowKind::Property)
        },
        PropertyTest::Compare { .. } => {
            let only = if exclude {
                SearchTerms {
                    excluded_properties: vec![filter.clone()],
                    ..SearchTerms::default()
                }
            } else {
                SearchTerms {
                    properties: vec![filter.clone()],
                    ..SearchTerms::default()
                }
            };
            BuilderRow::with_value(RowKind::Raw, only.to_query_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(m: &BuilderModel) -> Vec<(RowKind, bool, &str)> {
        m.rows
            .iter()
            .map(|r| (r.kind, r.exclude, r.value.as_str()))
            .collect()
    }

    /// Rows → query → rows is stable, and the query means what it did.
    fn assert_keeps_meaning(query: &str) {
        let model = BuilderModel::from_query(query);
        let written = model.to_query();
        assert_eq!(
            SearchTerms::from_query_string(&written),
            SearchTerms::from_query_string(query),
            "{query:?} written as {written:?}"
        );
        assert_eq!(BuilderModel::from_query(&written), model, "{query:?}");
    }

    #[test]
    fn each_kind_of_term_gets_its_row() {
        let m = BuilderModel::from_query(
            "meeting -draft =tasks @plan /journal #urgent -#old <spec >index",
        );
        assert_eq!(
            kinds(&m),
            vec![
                (RowKind::Text, false, "meeting"),
                (RowKind::Text, true, "draft"),
                (RowKind::Name, false, "tasks"),
                (RowKind::Heading, false, "plan"),
                (RowKind::Folder, false, "journal"),
                (RowKind::Tag, false, "urgent"),
                (RowKind::Tag, true, "old"),
                (RowKind::LinksTo, false, "spec"),
                (RowKind::LinkedFrom, false, "index"),
            ]
        );
        assert_eq!(m.sort, None);
    }

    #[test]
    fn property_rows_carry_key_comparison_and_value() {
        let m = BuilderModel::from_query("%due %priority>=2 -%status=done");
        let p = |i: usize| {
            (
                m.rows[i].key.as_str(),
                m.rows[i].op,
                m.rows[i].value.as_str(),
            )
        };
        assert_eq!(p(0), ("due", None, ""));
        assert_eq!(p(1), ("priority", Some(PropertyOp::Ge), "2"));
        assert_eq!(p(2), ("status", Some(PropertyOp::Eq), "done"));
        assert!(m.rows[2].exclude);
        assert!(m.rows.iter().all(|r| r.kind == RowKind::Property));
    }

    #[test]
    fn the_sort_is_read_and_written_with_the_sort_helpers() {
        let m = BuilderModel::from_query("#a -^%\"due date\"");
        assert_eq!(
            m.sort,
            Some((
                SortField::Property("due date".into()),
                SortOrder::Descending
            ))
        );
        assert_eq!(m.rows.len(), 1);
        let q = m.to_query();
        assert_eq!(directive_of_query(&q), m.sort);
        assert!(q.starts_with("#a "), "{q}");
        // No sort, no directive.
        let m = BuilderModel::from_query("#a");
        assert_eq!(m.to_query(), "#a");
    }

    #[test]
    fn what_no_row_expresses_is_kept_as_raw() {
        // An unterminated quote, a bare prefix, a second sort, a literal `*`.
        let q = "a # \"open";
        let m = BuilderModel::from_query(q);
        let raw: Vec<&str> = m
            .rows
            .iter()
            .filter(|r| r.kind == RowKind::Raw)
            .map(|r| r.value.as_str())
            .collect();
        assert_eq!(raw, vec!["#", "\"open"]);
        assert_keeps_meaning(q);

        let m = BuilderModel::from_query("a ^title -^file");
        assert_eq!(m.sort, Some((SortField::Title, SortOrder::Ascending)));
        assert_eq!(m.rows.last().map(|r| r.kind), Some(RowKind::Raw));
        assert_keeps_meaning("a ^title -^file");

        // The wildcard one is a property row; the literal one stays raw, and
        // is written after the other filters (they are ANDed, so the order
        // between them means nothing).
        let q = "%name=a*b %name=\"a*b\"";
        let m = BuilderModel::from_query(q);
        assert_eq!(m.rows[0].kind, RowKind::Property, "the wildcard one");
        assert_eq!(m.rows[1].kind, RowKind::Raw, "the literal one");
        assert_keeps_meaning(q);
    }

    #[test]
    fn queries_keep_their_meaning_through_the_rows() {
        for q in [
            "",
            "meeting notes",
            "\"two words\" -\"not this\"",
            "=\"my note\" @\"a heading\" /journal/2024",
            "#finance -#draft",
            "<{note} >{note}",
            "%\"due date\"<2025-01-01 %tags=proj* -%done",
            "x ^title",
            "x -^%rank",
            "a # lk: ^bogus",
            "caf\u{e9} #T\u{e9}l\u{e9}",
        ] {
            assert_keeps_meaning(q);
        }
    }

    #[test]
    fn incomplete_rows_stay_but_are_not_written() {
        let mut m = BuilderModel::from_query("#a");
        m.rows.push(BuilderRow::new(RowKind::Tag));
        m.rows.push(BuilderRow::with_value(RowKind::Text, "   "));
        let mut p = BuilderRow::new(RowKind::Property);
        p.key = "due".into();
        p.op = Some(PropertyOp::Gt); // no value yet
        m.rows.push(p);
        assert_eq!(m.rows.len(), 4);
        assert_eq!(m.incomplete(), 3);
        assert_eq!(m.to_query(), "#a");
        // Once it says enough, it is part of the query.
        m.rows[3].value = "2025-01-01".into();
        assert_eq!(m.to_query(), "#a %due>2025-01-01");
        // A property with no comparison only needs its key.
        m.rows[3].op = None;
        m.rows[3].value.clear();
        assert_eq!(m.to_query(), "#a %due");
    }

    #[test]
    fn rows_built_by_hand_write_what_typing_would() {
        let mut m = BuilderModel::default();
        m.rows
            .push(BuilderRow::with_value(RowKind::Tag, " Urgent "));
        m.rows.push(BuilderRow {
            exclude: true,
            ..BuilderRow::with_value(RowKind::Name, "my note")
        });
        m.rows
            .push(BuilderRow::with_value(RowKind::LinksTo, "{note}"));
        m.rows.push(BuilderRow {
            key: "Due Date".into(),
            value: "2025-01-01".into(),
            op: Some(PropertyOp::Lt),
            ..BuilderRow::new(RowKind::Property)
        });
        m.sort = Some((SortField::Title, SortOrder::Ascending));
        let q = m.to_query();
        assert_eq!(
            SearchTerms::from_query_string(&q),
            SearchTerms::from_query_string(
                "#urgent -=\"my note\" <{note} %\"due date\"<2025-01-01 or:title"
            )
        );
        assert!(!RowKind::Raw.can_exclude() && RowKind::Tag.can_exclude());
    }

    #[test]
    fn a_blank_property_sort_is_left_out() {
        let mut m = BuilderModel::from_query("#a");
        m.sort = Some((SortField::Property(String::new()), SortOrder::Ascending));
        assert_eq!(m.to_query(), "#a");
    }
}
