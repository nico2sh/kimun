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
        Self { name: name.to_string() }
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
            self.rows.iter().filter(|r| r.name.contains(query)).cloned().collect()
        } else {
            self.rows.clone()
        };
        emit.replace(out);
    }
    fn reload_on_query(&self) -> bool {
        self.reload
    }
}
