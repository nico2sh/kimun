//! In-memory adapters for testing `SearchList` without a vault.
#![cfg(test)]

use async_trait::async_trait;

use super::seams::{Emit, RowSource, SearchRow};

#[derive(Clone, Debug, PartialEq)]
pub struct TestRow {
    pub name: String,
}

impl TestRow {
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
        }
    }
}

impl SearchRow for TestRow {
    fn to_list_item(
        &self,
        _t: &crate::settings::themes::Theme,
        _i: &crate::settings::icons::Icons,
        _sel: bool,
    ) -> ratatui::widgets::ListItem<'static> {
        ratatui::widgets::ListItem::new(self.name.clone())
    }
    fn match_text(&self) -> Option<&str> {
        Some(&self.name)
    }
}

/// One-shot source: returns rows whose name contains the query (server-side
/// filter analogue), or all rows for an empty query.
pub struct VecSource {
    pub rows: Vec<TestRow>,
    pub reload: bool,
}

#[async_trait]
impl RowSource<TestRow> for VecSource {
    async fn load(&self, query: &str, emit: Emit<TestRow>) {
        let out: Vec<TestRow> = if self.reload && !query.is_empty() {
            self.rows
                .iter()
                .filter(|r| r.name.contains(query))
                .cloned()
                .collect()
        } else {
            self.rows.clone()
        };
        emit.replace(out);
    }
    fn reload_on_query(&self) -> bool {
        self.reload
    }
}

/// One-shot source that also exposes a query-fresh leading row for a non-empty
/// query. Regression guard for the saved-searches virtual entry.
pub struct VecSourceWithLead {
    pub rows: Vec<TestRow>,
}

#[async_trait]
impl RowSource<TestRow> for VecSourceWithLead {
    async fn load(&self, _query: &str, emit: Emit<TestRow>) {
        emit.replace(self.rows.clone());
    }
    fn leading_row(&self, query: &str) -> Option<TestRow> {
        if query.is_empty() {
            None
        } else {
            Some(TestRow::new(&format!("create:{query}")))
        }
    }
    fn reload_on_query(&self) -> bool {
        false
    }
}

/// Streamed source: pushes each row of each batch one at a time, then `done`.
/// Loads once (`reload_on_query` is `false`) so a local `Filter` narrows the
/// set — exercises the streamed Push path the sidebar relies on.
pub struct ScriptedStreamSource {
    pub batches: Vec<Vec<TestRow>>,
}

#[async_trait]
impl RowSource<TestRow> for ScriptedStreamSource {
    async fn load(&self, _query: &str, emit: Emit<TestRow>) {
        for batch in &self.batches {
            for row in batch {
                emit.push(row.clone());
            }
        }
        emit.done();
    }
    fn reload_on_query(&self) -> bool {
        false
    }
}

/// A reload-on-query source that also exposes a query-fresh leading row.
/// Used to prove that the `reload_on_query == true` branch of `requery()`
/// also rebuilds the leading row synchronously (Fix A regression guard).
pub struct ReloadWithLeadSource {
    pub rows: Vec<TestRow>,
}

#[async_trait]
impl RowSource<TestRow> for ReloadWithLeadSource {
    async fn load(&self, query: &str, emit: Emit<TestRow>) {
        let out: Vec<TestRow> = if query.is_empty() {
            self.rows.clone()
        } else {
            self.rows
                .iter()
                .filter(|r| r.name.contains(query))
                .cloned()
                .collect()
        };
        emit.replace(out);
    }
    fn leading_row(&self, query: &str) -> Option<TestRow> {
        if query.is_empty() {
            None
        } else {
            Some(TestRow::new(&format!("create:{query}")))
        }
    }
    fn reload_on_query(&self) -> bool {
        true
    }
}

/// A row type whose leading variant is filter-exempt (`match_text() == None`).
/// Mirrors the sidebar's `FileListEntry`: streamed `Item` rows plus a synthetic
/// `Create` leading row.
#[derive(Clone, Debug, PartialEq)]
pub enum StreamRow {
    Item(String),
    Create(String),
}

impl SearchRow for StreamRow {
    fn to_list_item(
        &self,
        _t: &crate::settings::themes::Theme,
        _i: &crate::settings::icons::Icons,
        _sel: bool,
    ) -> ratatui::widgets::ListItem<'static> {
        let text = match self {
            StreamRow::Item(s) => s.clone(),
            StreamRow::Create(q) => format!("create:{q}"),
        };
        ratatui::widgets::ListItem::new(text)
    }
    fn match_text(&self) -> Option<&str> {
        match self {
            StreamRow::Item(s) => Some(s),
            StreamRow::Create(_) => None,
        }
    }
}

/// Streamed source (the sidebar shape): pushes `Item` rows one at a time then
/// `done`, loads once (`reload_on_query() == false`), and exposes a query-fresh
/// `Create` leading row for any non-empty query.
pub struct ScriptedStreamLeadSource {
    pub items: Vec<String>,
}

#[async_trait]
impl RowSource<StreamRow> for ScriptedStreamLeadSource {
    async fn load(&self, _query: &str, emit: Emit<StreamRow>) {
        for s in &self.items {
            emit.push(StreamRow::Item(s.clone()));
        }
        emit.done();
    }
    fn leading_row(&self, query: &str) -> Option<StreamRow> {
        if query.is_empty() {
            None
        } else {
            Some(StreamRow::Create(query.to_string()))
        }
    }
    fn reload_on_query(&self) -> bool {
        false
    }
}
