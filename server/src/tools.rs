//! The four tools the model can call during an agentic ask: `search`,
//! `list_date_keys`, `read_note` and `related_notes`.
//!
//! Every tool is locked to the asking collection: a [`ToolRunner`] holds the
//! collection key, and no tool argument can name another one. A tool never
//! fails the loop: a problem comes back as a [`ToolOutput`] whose text starts
//! with `error:`, so the model can read it and change course.

use crate::dbembeddings::{DateFilter, QueryFilter};
use crate::document::JOURNAL_DATE_FORMAT;
use crate::{CollectionKey, KimunRag, ScoredChunk};
use serde_json::{Value, json};
use std::collections::BTreeSet;

/// A tool as the model sees it.
pub struct ToolSpec {
    pub name: &'static str,
    pub description: &'static str,
    /// JSON schema of the arguments.
    pub schema: Value,
}

/// A parsed, validated tool call.
#[derive(Debug)]
pub enum ToolCall {
    Search {
        query: String,
        filter: QueryFilter,
    },
    ListDateKeys,
    ReadNote {
        path: String,
        section: Option<String>,
    },
    RelatedNotes {
        path: String,
    },
}

/// What a tool hands back to the loop.
pub struct ToolOutput {
    /// The text the model reads. Starts with `error:` when the call failed.
    pub text: String,
    /// The chunks the model can cite: the search results, or the chunks of the
    /// note that was read. Only chunks whose text made it into `text`.
    pub chunks: Vec<ScoredChunk>,
    /// One plain line describing the step, for the trail.
    pub trail: String,
}

/// Longest tool result text, in characters. A longer result is cut and ends
/// with `TRUNCATED_MARKER`.
pub const TOOL_RESULT_CAP: usize = 12_000;

pub(crate) const TRUNCATED_MARKER: &str = "[truncated]";

pub fn tool_specs() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            name: "search",
            description: "Search the notes by meaning. Returns the best matching note passages, \
                          best first. Optional filters narrow the notes searched. Use a date \
                          range for questions about a period of time. With a date range the \
                          query only orders the results, so use a broad query such as the \
                          topic, or the word notes. Dates are inclusive, YYYY-MM-DD; work them \
                          out from today's date given in the system prompt. Daily or journal \
                          notes use date_key journal. Other date_key values come from \
                          list_date_keys. A result ending in [truncated] means: narrow the \
                          range or the query.",
            schema: json!({
                "type": "object",
                "properties": {
                    "query": {"type": "string", "description": "What to look for."},
                    "date_from": {"type": "string", "description": "First day of the range, YYYY-MM-DD. Give it together with date_to."},
                    "date_to": {"type": "string", "description": "Last day of the range, YYYY-MM-DD. Give it together with date_from."},
                    "date_key": {"type": "string", "description": "Which note date the range applies to: \"journal\" (the date in a journal note's name) or a date property such as \"due\". See list_date_keys. Omit to match any."},
                    "folder": {"type": "string", "description": "Only notes in this folder and its subfolders."},
                    "tags": {"type": "array", "items": {"type": "string"}, "description": "Only notes that have every one of these tags."}
                },
                "required": ["query"]
            }),
        },
        ToolSpec {
            name: "list_date_keys",
            description: "List the date keys the notes have: \"journal\" and any date property \
                          such as \"due\". Use it to pick a date_key for search.",
            schema: json!({"type": "object", "properties": {}}),
        },
        ToolSpec {
            name: "read_note",
            description: "Read a note by its path, whole or one section of it.",
            schema: json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string", "description": "Path of the note, as shown in search results."},
                    "section": {"type": "string", "description": "Optional. The last part of a section title as shown after ›. Omit to read the whole note."}
                },
                "required": ["path"]
            }),
        },
        ToolSpec {
            name: "related_notes",
            description: "For a note, list the notes it links to and the notes that link to it.",
            schema: json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string", "description": "Path of the note, as shown in search results."}
                },
                "required": ["path"]
            }),
        },
    ]
}

fn str_arg(args: &Value, name: &str) -> Result<Option<String>, String> {
    match args.get(name) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.clone())),
        Some(_) => Err(format!("argument `{name}` must be a string")),
    }
}

fn required_str(args: &Value, name: &str) -> Result<String, String> {
    match str_arg(args, name)? {
        Some(s) if !s.trim().is_empty() => Ok(s),
        _ => Err(format!("argument `{name}` is required")),
    }
}

fn date_arg(args: &Value, name: &str) -> Result<Option<chrono::NaiveDate>, String> {
    str_arg(args, name)?
        .map(|s| {
            chrono::NaiveDate::parse_from_str(s.trim(), JOURNAL_DATE_FORMAT)
                .map_err(|_| format!("argument `{name}` must be a date as YYYY-MM-DD, got `{s}`"))
        })
        .transpose()
}

/// Validates a tool call the model made. The message of an `Err` goes back to
/// the model.
pub fn parse_call(name: &str, args: &Value) -> Result<ToolCall, String> {
    match name {
        "search" => {
            let query = required_str(args, "query")?;
            let from = date_arg(args, "date_from")?;
            let to = date_arg(args, "date_to")?;
            let key = str_arg(args, "date_key")?
                .map(|k| k.trim().to_lowercase())
                .filter(|k| !k.is_empty());
            let dates = match (from, to) {
                (Some(from), Some(to)) => Some(DateFilter {
                    source: key,
                    from,
                    to,
                }),
                (None, None) if key.is_none() => None,
                (None, None) => {
                    return Err("`date_key` needs `date_from` and `date_to`".to_string());
                }
                _ => return Err("`date_from` and `date_to` must be given together".to_string()),
            };
            let folder = str_arg(args, "folder")?
                .map(|f| f.trim().trim_matches('/').to_string())
                .filter(|f| !f.is_empty());
            let tags = match args.get("tags") {
                None | Some(Value::Null) => Vec::new(),
                Some(Value::Array(items)) => items
                    .iter()
                    .map(|t| {
                        t.as_str()
                            .map(|t| t.trim().trim_start_matches('#').to_lowercase())
                            .ok_or_else(|| "argument `tags` must be a list of strings".to_string())
                    })
                    .collect::<Result<Vec<_>, _>>()?
                    .into_iter()
                    .filter(|t| !t.is_empty())
                    .collect(),
                Some(_) => return Err("argument `tags` must be a list of strings".to_string()),
            };
            Ok(ToolCall::Search {
                query,
                filter: QueryFilter {
                    dates,
                    folder,
                    tags,
                },
            })
        }
        "list_date_keys" => Ok(ToolCall::ListDateKeys),
        "read_note" => Ok(ToolCall::ReadNote {
            path: required_str(args, "path")?,
            section: str_arg(args, "section")?.filter(|s| !s.trim().is_empty()),
        }),
        "related_notes" => Ok(ToolCall::RelatedNotes {
            path: required_str(args, "path")?,
        }),
        other => Err(format!("unknown tool `{other}`")),
    }
}

fn error_output(message: impl std::fmt::Display, trail: String) -> ToolOutput {
    ToolOutput {
        text: format!("error: {message}"),
        chunks: Vec::new(),
        trail,
    }
}

/// Cuts `text` to the cap on a char boundary and appends the marker.
fn cap_text(text: String) -> String {
    match text.char_indices().nth(TOOL_RESULT_CAP) {
        None => text,
        Some((cut, _)) => format!("{}\n{TRUNCATED_MARKER}", &text[..cut]),
    }
}

/// Joins rendered chunks while they fit under the cap. Returns the text and
/// how many leading chunks it holds, so the model is never offered a source
/// it did not get to read.
fn fit_chunks(pieces: &[String]) -> (String, usize) {
    let mut text = String::new();
    let mut used = 0;
    let mut count = 0;
    for piece in pieces {
        let len = piece.chars().count() + 2;
        if used + len > TOOL_RESULT_CAP && count > 0 {
            text.push_str(TRUNCATED_MARKER);
            return (text, count);
        }
        text.push_str(piece);
        text.push_str("\n\n");
        used += len;
        count += 1;
    }
    (text.trim_end().to_string(), count)
}

fn trail_for_search(query: &str, filter: &QueryFilter) -> String {
    match &filter.dates {
        Some(d) => {
            let what = match d.source.as_deref() {
                Some("journal") => "journal entries".to_string(),
                Some(key) => format!("notes by {key}"),
                None => "notes dated".to_string(),
            };
            format!("Searching {what} from {} to {}", d.from, d.to)
        }
        None => format!("Searching: \"{query}\""),
    }
}

/// Runs tool calls against one collection.
pub struct ToolRunner<'a> {
    rag: &'a KimunRag,
    collection: &'a CollectionKey,
}

impl<'a> ToolRunner<'a> {
    pub fn new(rag: &'a KimunRag, collection: &'a CollectionKey) -> Self {
        Self { rag, collection }
    }

    pub async fn run(&self, call: &ToolCall) -> ToolOutput {
        match call {
            ToolCall::Search { query, filter } => self.search(query, filter).await,
            ToolCall::ListDateKeys => self.list_date_keys().await,
            ToolCall::ReadNote { path, section } => self.read_note(path, section.as_deref()).await,
            ToolCall::RelatedNotes { path } => self.related_notes(path).await,
        }
    }

    async fn search(&self, query: &str, filter: &QueryFilter) -> ToolOutput {
        let trail = trail_for_search(query, filter);
        if let Some(d) = &filter.dates
            && d.from > d.to
        {
            return error_output(
                format!(
                    "date_from ({}) is after date_to ({}); swap them",
                    d.from, d.to
                ),
                trail,
            );
        }
        let found = match self
            .rag
            .retrieve_filtered(self.collection, query, filter)
            .await
        {
            Ok(found) => found,
            Err(e) => return error_output(e, trail),
        };
        if found.is_empty() {
            return ToolOutput {
                text: "No matching notes.".to_string(),
                chunks: Vec::new(),
                trail,
            };
        }
        let pieces: Vec<String> = found
            .iter()
            .map(|(_, c)| {
                let dates: Vec<String> = c
                    .meta
                    .dates
                    .iter()
                    .map(|d| format!("{} {}", d.source, d.day))
                    .collect();
                let dates = if dates.is_empty() {
                    String::new()
                } else {
                    format!("dates: {}\n", dates.join(", "))
                };
                format!("[{}] {}\n{dates}{}", c.doc_path, c.title, c.text)
            })
            .collect();
        let (text, count) = fit_chunks(&pieces);
        ToolOutput {
            text,
            chunks: found.into_iter().take(count).collect(),
            trail,
        }
    }

    async fn list_date_keys(&self) -> ToolOutput {
        let trail = "Listing date properties".to_string();
        match self.rag.store.date_sources(self.collection.as_str()).await {
            Ok(keys) if keys.is_empty() => ToolOutput {
                text: "No notes have a date.".to_string(),
                chunks: Vec::new(),
                trail,
            },
            Ok(keys) => ToolOutput {
                text: cap_text(keys.join("\n")),
                chunks: Vec::new(),
                trail,
            },
            Err(e) => error_output(e, trail),
        }
    }

    /// The indexed paths of the collection.
    async fn note_paths(&self) -> anyhow::Result<Vec<String>> {
        let mut paths: Vec<String> = self
            .rag
            .store
            .indexed_notes(self.collection.as_str())
            .await?
            .into_keys()
            .collect();
        paths.sort();
        Ok(paths)
    }

    /// The collection's note path the model meant, if there is one.
    async fn find_note(&self, path: &str) -> Result<String, String> {
        let wanted = path_key(path);
        let paths = self.note_paths().await.map_err(|e| e.to_string())?;
        paths
            .into_iter()
            .find(|p| path_key(p) == wanted)
            .ok_or_else(|| format!("no note at `{path}` in this vault"))
    }

    async fn read_note(&self, path: &str, section: Option<&str>) -> ToolOutput {
        let trail = format!("Reading {path}");
        let found = match self.find_note(path).await {
            Ok(p) => p,
            Err(e) => return error_output(e, trail),
        };
        let rows = match self
            .rag
            .store
            .chunks_with_vectors(self.collection.as_str(), std::slice::from_ref(&found))
            .await
        {
            Ok(rows) => rows,
            Err(e) => return error_output(e, trail),
        };
        let chunks: Vec<&crate::document::FlattenedChunk> = rows
            .iter()
            .map(|r| &r.chunk)
            .filter(|c| section.is_none_or(|s| section_matches(&c.title, s)))
            .collect();
        if chunks.is_empty() {
            let message = match section {
                Some(s) => format!("no section `{s}` in `{found}`"),
                None => format!("`{found}` has no text"),
            };
            return error_output(message, trail);
        }
        let mut last_title: Option<&str> = None;
        let pieces: Vec<String> = chunks
            .iter()
            .map(|c| {
                let heading = if last_title != Some(c.title.as_str()) && !c.title.is_empty() {
                    format!("## {}\n", c.title.replace('\u{1f}', " \u{203a} "))
                } else {
                    String::new()
                };
                last_title = Some(c.title.as_str());
                format!("{heading}{}", c.text)
            })
            .collect();
        let (text, count) = fit_chunks(&pieces);
        ToolOutput {
            text,
            chunks: chunks
                .into_iter()
                .take(count)
                .map(|c| (1.0, c.clone()))
                .collect(),
            trail,
        }
    }

    async fn related_notes(&self, path: &str) -> ToolOutput {
        let trail = format!("Finding notes related to {path}");
        let found = match self.find_note(path).await {
            Ok(p) => p,
            Err(e) => return error_output(e, trail),
        };
        let (paths, links) = match self.note_paths().await {
            Ok(paths) => match self.rag.store.note_links(self.collection.as_str()).await {
                Ok(links) => (paths, links),
                Err(e) => return error_output(e, trail),
            },
            Err(e) => return error_output(e, trail),
        };
        let index: Vec<(String, String)> = paths.iter().map(|p| (p.clone(), path_key(p))).collect();

        let mut to = BTreeSet::new();
        let mut missing = BTreeSet::new();
        for link in links.get(&found).map(Vec::as_slice).unwrap_or_default() {
            let target = clean_target(link);
            if target.is_empty() {
                continue;
            }
            let hits = resolve_link(&found, &target, &index);
            if hits.is_empty() {
                missing.insert(target);
            }
            to.extend(hits.into_iter().filter(|h| *h != found));
        }
        let mut from = BTreeSet::new();
        for (source, written) in &links {
            if *source == found {
                continue;
            }
            let hit = written.iter().any(|link| {
                let target = clean_target(link);
                !target.is_empty() && resolve_link(source, &target, &index).contains(&found)
            });
            if hit {
                from.insert(source.clone());
            }
        }

        let section = |heading: &str, items: &BTreeSet<String>| {
            let body = if items.is_empty() {
                "(none)".to_string()
            } else {
                items.iter().cloned().collect::<Vec<_>>().join("\n")
            };
            format!("{heading}:\n{body}")
        };
        let text = [
            section("Links to", &to),
            section("Linked from", &from),
            section("Missing", &missing),
        ]
        .join("\n\n");
        ToolOutput {
            text: cap_text(text),
            chunks: Vec::new(),
            trail,
        }
    }
}

/// Separators between the parts of a breadcrumb title: U+001F as core joins
/// them, ` › ` as the model sees them, and `>`. Never `/`: a heading can hold
/// one (`Q3/Q4 plan`).
const BREADCRUMB_SEPARATORS: [char; 3] = ['\u{1f}', '›', '>'];

/// The parts of a breadcrumb, trimmed and lowercase.
fn breadcrumb_parts(text: &str) -> Vec<String> {
    text.split(BREADCRUMB_SEPARATORS)
        .map(|p| p.trim().to_lowercase())
        .collect()
}

/// A section matches by its whole breadcrumb title (as core joins it or as
/// the model sees it) or by the last part of it, ignoring case.
fn section_matches(title: &str, wanted: &str) -> bool {
    let title = breadcrumb_parts(title);
    let wanted = breadcrumb_parts(wanted);
    title == wanted || (wanted.len() == 1 && title.last() == wanted.last())
}

// ── Link matching ───────────────────────────────────────────────────────────
// A small local copy of the rules of adr/0051: a link written as a name
// matches every note whose path ends with it, component by component. A link
// written as a path (`/`, `./`, `../`) matches exactly one path. Both ignore
// case, the `.md` extension, and any `#fragment`, `^block` or `|alias`.
// The server has no `kimun_core`, so paths are the canonical wire paths.

/// A path or link target in comparison form: lowercase, no leading `/`, no
/// `.md`.
fn path_key(path: &str) -> String {
    let lower = path.trim().to_lowercase();
    let trimmed = lower.trim_start_matches('/');
    trimmed.strip_suffix(".md").unwrap_or(trimmed).to_string()
}

/// A link target as written, minus fragment, block id and alias.
fn clean_target(raw: &str) -> String {
    let end = raw.find(['#', '^', '|']).unwrap_or(raw.len());
    raw[..end].trim().to_string()
}

/// The notes (original paths) a link written in `from` points at. `index`
/// holds `(path, path_key)` pairs.
fn resolve_link(from: &str, target: &str, index: &[(String, String)]) -> Vec<String> {
    let is_path = target.starts_with('/') || target.starts_with("./") || target.starts_with("../");
    if is_path {
        let mut parts: Vec<String> = Vec::new();
        if !target.starts_with('/') {
            let from_key = path_key(from);
            let folder = from_key.rsplit_once('/').map_or("", |(f, _)| f);
            parts.extend(
                folder
                    .split('/')
                    .filter(|p| !p.is_empty())
                    .map(String::from),
            );
        }
        let lower = target.to_lowercase();
        let lower = lower.strip_suffix(".md").unwrap_or(&lower);
        for part in lower.split('/') {
            match part {
                "" | "." => {}
                ".." => {
                    if parts.pop().is_none() {
                        return Vec::new();
                    }
                }
                other => parts.push(other.to_string()),
            }
        }
        let wanted = parts.join("/");
        index
            .iter()
            .filter(|(_, key)| *key == wanted)
            .map(|(p, _)| p.clone())
            .collect()
    } else {
        let key = path_key(target);
        let wanted: Vec<&str> = key.split('/').filter(|p| !p.is_empty()).collect();
        index
            .iter()
            .filter(|(_, k)| {
                let have: Vec<&str> = k.split('/').collect();
                have.len() >= wanted.len() && have[have.len() - wanted.len()..] == wanted[..]
            })
            .map(|(p, _)| p.clone())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dbembeddings::vecsqlite::VecSqlite;
    use crate::document::{KimunDoc, KimunSection, NoteDate};
    use crate::test_support::{FakeEmbedder, FakeVectorStore};
    use serde_json::json;
    use std::sync::Arc;

    fn key(s: &str) -> CollectionKey {
        CollectionKey::parse(s).unwrap()
    }

    async fn sqlite_rag() -> (KimunRag, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let store = VecSqlite::new(dir.path(), 8).await.unwrap();
        (
            KimunRag::new(Arc::new(store), Arc::new(FakeEmbedder), None),
            dir,
        )
    }

    fn doc(path: &str, text: &str) -> KimunDoc {
        KimunDoc {
            path: path.to_string(),
            hash: format!("h-{path}"),
            sections: vec![KimunSection {
                title: "T".to_string(),
                text: text.to_string(),
            }],
            ..Default::default()
        }
    }

    fn linking(path: &str, links: &[&str]) -> KimunDoc {
        KimunDoc {
            links: links.iter().map(|s| s.to_string()).collect(),
            ..doc(path, &format!("body of {path}"))
        }
    }

    fn day(s: &str) -> chrono::NaiveDate {
        chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    fn search_all(query: &str, filter: QueryFilter) -> ToolCall {
        ToolCall::Search {
            query: query.to_string(),
            filter,
        }
    }

    /// The lines under `heading:` up to the next blank line, minus `(none)`.
    fn listed(text: &str, heading: &str) -> Vec<String> {
        text.lines()
            .skip_while(|l| *l != format!("{heading}:"))
            .skip(1)
            .take_while(|l| !l.is_empty())
            .filter(|l| *l != "(none)")
            .map(String::from)
            .collect()
    }

    #[test]
    fn parse_call_reads_search_filters() {
        let call = parse_call(
            "search",
            &json!({
                "query": "standup",
                "date_from": "2026-11-17",
                "date_to": "2026-11-23",
                "date_key": "Journal",
                "folder": "work",
                "tags": ["Plan", "q4"],
            }),
        )
        .unwrap();
        let ToolCall::Search { query, filter } = call else {
            panic!("not a search");
        };
        assert_eq!(query, "standup");
        let dates = filter.dates.unwrap();
        assert_eq!(dates.source.as_deref(), Some("journal"));
        assert_eq!(dates.from, day("2026-11-17"));
        assert_eq!(dates.to, day("2026-11-23"));
        assert_eq!(filter.folder.as_deref(), Some("work"));
        assert_eq!(filter.tags, vec!["plan", "q4"]);

        let ToolCall::Search { filter, .. } = parse_call(
            "search",
            &json!({"query": "x", "date_from": "2026-11-17", "date_to": "2026-11-23", "date_key": "Due"}),
        )
        .unwrap() else {
            panic!()
        };
        assert_eq!(filter.dates.unwrap().source.as_deref(), Some("due"));

        let ToolCall::Search { filter, .. } = parse_call("search", &json!({"query": "x"})).unwrap()
        else {
            panic!()
        };
        assert!(filter.is_empty());
    }

    #[test]
    fn parse_call_rejects_bad_input() {
        assert!(parse_call("delete_everything", &json!({})).is_err());
        assert!(parse_call("search", &json!({})).is_err());
        let bad_date = json!({"query": "x", "date_from": "yesterday", "date_to": "2026-11-23"});
        assert!(parse_call("search", &bad_date).is_err());
        let one_date = json!({"query": "x", "date_from": "2026-11-17"});
        assert!(parse_call("search", &one_date).is_err());
        assert!(parse_call("read_note", &json!({})).is_err());
        assert!(parse_call("related_notes", &json!({})).is_err());
    }

    #[test]
    fn parse_call_cleans_the_folder() {
        for (given, want) in [
            ("work/", Some("work")),
            ("/work", Some("work")),
            ("/work/", Some("work")),
            ("/", None),
            ("", None),
        ] {
            let ToolCall::Search { filter, .. } =
                parse_call("search", &json!({"query": "x", "folder": given})).unwrap()
            else {
                panic!()
            };
            assert_eq!(filter.folder.as_deref(), want, "folder {given:?}");
        }
    }

    #[test]
    fn parse_call_reads_the_other_tools() {
        assert!(matches!(
            parse_call("list_date_keys", &json!({})).unwrap(),
            ToolCall::ListDateKeys
        ));
        let ToolCall::ReadNote { path, section } =
            parse_call("read_note", &json!({"path": "a.md", "section": "Plan"})).unwrap()
        else {
            panic!()
        };
        assert_eq!((path.as_str(), section.as_deref()), ("a.md", Some("Plan")));
        assert!(matches!(
            parse_call("related_notes", &json!({"path": "a.md"})).unwrap(),
            ToolCall::RelatedNotes { .. }
        ));
    }

    #[test]
    fn specs_name_the_four_tools() {
        let names: Vec<_> = tool_specs().iter().map(|s| s.name).collect();
        assert_eq!(
            names,
            ["search", "list_date_keys", "read_note", "related_notes"]
        );
        let specs = tool_specs();
        for spec in &specs {
            assert_eq!(spec.schema["type"], "object", "{}", spec.name);
            assert!(spec.schema["properties"].is_object(), "{}", spec.name);
        }
        assert!(specs[0].description.contains("date_key journal"));
        assert!(specs[0].description.contains("[truncated]"));
        let section = &specs[2].schema["properties"]["section"]["description"];
        assert!(section.as_str().unwrap().contains('›'), "{section}");
    }

    #[test]
    fn section_matches_breadcrumb_titles() {
        // Core breadcrumbs join headings with U+001F; the model sees ` › `.
        let title = "Title\u{1f}Q4 plan";
        assert!(section_matches(title, "Title\u{1f}Q4 plan"));
        assert!(section_matches(title, "title › q4 plan"));
        assert!(section_matches(title, "  Q4 Plan "));
        assert!(section_matches("Title > Q4 plan", "q4 plan"));
        assert!(section_matches("Title › Q4 plan", "q4 plan"));
        assert!(section_matches("Plan", "plan"));
        // `/` is part of a heading, never a separator.
        assert!(!section_matches("Q3/Q4 plan", "Q4 plan"));
        assert!(section_matches("Q3/Q4 plan", "q3/q4 plan"));
        assert!(!section_matches(title, "Title"));
    }

    #[tokio::test]
    async fn search_with_reversed_range_returns_an_error_text() {
        let (rag, _dir) = sqlite_rag().await;
        let k = key("v");
        rag.index(&k, &[doc("journal/2026-11-18.md", "standup")])
            .await
            .unwrap();
        let call = parse_call(
            "search",
            &json!({"query": "standup", "date_from": "2026-11-23", "date_to": "2026-11-17"}),
        )
        .unwrap();
        let out = ToolRunner::new(&rag, &k).run(&call).await;
        assert!(out.text.starts_with("error:"), "{}", out.text);
        assert!(out.chunks.is_empty());
    }

    #[tokio::test]
    async fn search_filters_by_journal_range() {
        let (rag, _dir) = sqlite_rag().await;
        let k = key("v");
        rag.index(
            &k,
            &[
                doc("journal/2026-11-10.md", "standup notes"),
                doc("journal/2026-11-18.md", "standup notes"),
                doc("journal/2026-12-01.md", "standup notes"),
            ],
        )
        .await
        .unwrap();
        let call = parse_call(
            "search",
            &json!({"query": "standup", "date_from": "2026-11-17", "date_to": "2026-11-23", "date_key": "journal"}),
        )
        .unwrap();
        let out = ToolRunner::new(&rag, &k).run(&call).await;
        assert!(!out.chunks.is_empty());
        assert!(
            out.chunks
                .iter()
                .all(|(_, c)| c.doc_path == "journal/2026-11-18.md")
        );
        assert!(out.text.contains("journal/2026-11-18.md"));
        assert!(!out.text.contains("2026-11-10.md"));
        assert_eq!(
            out.trail,
            "Searching journal entries from 2026-11-17 to 2026-11-23"
        );
    }

    #[tokio::test]
    async fn search_without_filters_trails_the_query() {
        let (rag, _dir) = sqlite_rag().await;
        let k = key("v");
        rag.index(&k, &[doc("a.md", "alpha")]).await.unwrap();
        let out = ToolRunner::new(&rag, &k)
            .run(&search_all("alpha", QueryFilter::default()))
            .await;
        assert_eq!(out.trail, "Searching: \"alpha\"");
        assert_eq!(out.chunks.len(), 1);
    }

    #[tokio::test]
    async fn store_error_on_a_filter_becomes_an_error_text() {
        let store = FakeVectorStore {
            refuse_filtered: true,
            ..Default::default()
        };
        let rag = KimunRag::new(Arc::new(store), Arc::new(FakeEmbedder), None);
        let k = key("v");
        let call = parse_call("search", &json!({"query": "x", "folder": "work"})).unwrap();
        let out = ToolRunner::new(&rag, &k).run(&call).await;
        assert!(out.text.starts_with("error:"), "{}", out.text);
        assert!(
            out.text
                .contains("filtered search is not supported on the qdrant store")
        );
        assert!(out.chunks.is_empty());
    }

    #[tokio::test]
    async fn list_date_keys_lists_sources() {
        let (rag, _dir) = sqlite_rag().await;
        let k = key("v");
        let mut due = doc("tasks.md", "pay rent");
        due.dates = vec![NoteDate {
            source: "due".into(),
            day: day("2026-12-01"),
        }];
        rag.index(&k, &[due, doc("journal/2026-11-18.md", "x")])
            .await
            .unwrap();
        let out = ToolRunner::new(&rag, &k).run(&ToolCall::ListDateKeys).await;
        let lines: Vec<_> = out.text.lines().collect();
        assert!(lines.contains(&"journal"), "{}", out.text);
        assert!(lines.contains(&"due"), "{}", out.text);
        assert_eq!(out.trail, "Listing date properties");
    }

    #[tokio::test]
    async fn read_note_returns_whole_note_and_one_section() {
        let (rag, _dir) = sqlite_rag().await;
        let k = key("v");
        let mut d = doc("a.md", "");
        d.sections = vec![
            KimunSection {
                title: "Intro".into(),
                text: "first words".into(),
            },
            KimunSection {
                title: "Plan".into(),
                text: "second words".into(),
            },
        ];
        rag.index(&k, &[d]).await.unwrap();
        let runner = ToolRunner::new(&rag, &k);

        let whole = runner
            .run(&ToolCall::ReadNote {
                path: "a.md".into(),
                section: None,
            })
            .await;
        assert!(whole.text.contains("first words"));
        assert!(whole.text.contains("second words"));
        assert_eq!(whole.chunks.len(), 2);
        assert_eq!(whole.trail, "Reading a.md");

        let part = runner
            .run(&ToolCall::ReadNote {
                path: "a.md".into(),
                section: Some("plan".into()),
            })
            .await;
        assert!(part.text.contains("second words"));
        assert!(!part.text.contains("first words"));
        assert_eq!(part.chunks.len(), 1);

        let missing = runner
            .run(&ToolCall::ReadNote {
                path: "a.md".into(),
                section: Some("nope".into()),
            })
            .await;
        assert!(missing.text.starts_with("error:"));
        assert!(missing.chunks.is_empty());
    }

    #[tokio::test]
    async fn read_note_unknown_path_is_an_error_text() {
        let (rag, _dir) = sqlite_rag().await;
        let (mine, theirs) = (key("mine"), key("theirs"));
        rag.index(&mine, &[doc("a.md", "my words")]).await.unwrap();
        rag.index(&theirs, &[doc("secret.md", "other vault secret")])
            .await
            .unwrap();
        let runner = ToolRunner::new(&rag, &mine);
        for path in ["nope.md", "secret.md"] {
            let out = runner
                .run(&ToolCall::ReadNote {
                    path: path.into(),
                    section: None,
                })
                .await;
            assert!(out.text.starts_with("error:"), "{path}: {}", out.text);
            assert!(!out.text.contains("other vault secret"));
            assert!(out.chunks.is_empty());
        }
    }

    #[tokio::test]
    async fn read_note_cuts_at_the_cap_with_a_marker() {
        let (rag, _dir) = sqlite_rag().await;
        let k = key("v");
        rag.index(&k, &[doc("big.md", &"word ".repeat(5000))])
            .await
            .unwrap();
        let out = ToolRunner::new(&rag, &k)
            .run(&ToolCall::ReadNote {
                path: "big.md".into(),
                section: None,
            })
            .await;
        assert!(out.text.ends_with("[truncated]"));
        assert!(out.text.chars().count() <= TOOL_RESULT_CAP + "\n[truncated]".len());
    }

    #[tokio::test]
    async fn related_notes_resolves_links_and_builds_backlinks() {
        let (rag, _dir) = sqlite_rag().await;
        let k = key("v");
        rag.index(
            &k,
            &[
                linking(
                    "journal/today.md",
                    &["Dup", "work/dup", "./local", "../x/other", "ghost#part"],
                ),
                linking("b.md", &["WORK/DUP"]),
                linking("dup.md", &[]),
                linking("work/dup.md", &[]),
                linking("a/work/dup.md", &[]),
                linking("notwork/dup.md", &[]),
                linking("journal/local.md", &[]),
                linking("x/other.md", &[]),
            ],
        )
        .await
        .unwrap();
        let runner = ToolRunner::new(&rag, &k);
        let related = |p: &str| {
            let call = ToolCall::RelatedNotes { path: p.into() };
            let runner = &runner;
            async move { runner.run(&call).await }
        };

        let today = related("journal/today.md").await;
        assert_eq!(today.trail, "Finding notes related to journal/today.md");
        let mut to = listed(&today.text, "Links to");
        to.sort();
        assert_eq!(
            to,
            [
                "a/work/dup.md",
                "dup.md",
                "journal/local.md",
                "notwork/dup.md",
                "work/dup.md",
                "x/other.md"
            ]
        );
        assert_eq!(listed(&today.text, "Missing"), ["ghost"]);
        assert!(listed(&today.text, "Linked from").is_empty());

        // `work/dup` ends with a path, so notwork/dup.md is not a backlink
        // target of b.md's link (but it is of today's bare `Dup`).
        let mut from = listed(&related("work/dup.md").await.text, "Linked from");
        from.sort();
        assert_eq!(from, ["b.md", "journal/today.md"]);
        let from = listed(&related("notwork/dup.md").await.text, "Linked from");
        assert_eq!(from, ["journal/today.md"]);
        let from = listed(&related("journal/local.md").await.text, "Linked from");
        assert_eq!(from, ["journal/today.md"]);

        let unknown = related("nope.md").await;
        assert!(unknown.text.starts_with("error:"));
    }

    /// Real clients push canonical absolute paths (`/work/a.md`). Every tool
    /// works on them, whether the model writes the leading `/` or not.
    #[tokio::test]
    async fn tools_work_on_absolute_pushed_paths() {
        let (rag, _dir) = sqlite_rag().await;
        let k = key("v");
        rag.index(
            &k,
            &[
                linking("/journal/2026-11-18.md", &["dup", "./local", "/work/plan"]),
                linking("/journal/local.md", &[]),
                linking("/work/plan.md", &[]),
                linking("/work/dup.md", &[]),
                linking("/other/work/x.md", &[]),
            ],
        )
        .await
        .unwrap();
        let runner = ToolRunner::new(&rag, &k);

        let call = parse_call("search", &json!({"query": "body", "folder": "work"})).unwrap();
        let out = runner.run(&call).await;
        let mut found: Vec<&str> = out
            .chunks
            .iter()
            .map(|(_, c)| c.doc_path.as_str())
            .collect();
        found.sort();
        assert_eq!(found, ["/work/dup.md", "/work/plan.md"]);

        let call = parse_call(
            "search",
            &json!({"query": "body", "date_from": "2026-11-18", "date_to": "2026-11-18", "date_key": "journal"}),
        )
        .unwrap();
        let out = runner.run(&call).await;
        let found: Vec<&str> = out
            .chunks
            .iter()
            .map(|(_, c)| c.doc_path.as_str())
            .collect();
        assert_eq!(found, ["/journal/2026-11-18.md"]);

        for path in ["work/plan.md", "/work/plan.md", "/Work/Plan"] {
            let out = runner
                .run(&ToolCall::ReadNote {
                    path: path.into(),
                    section: None,
                })
                .await;
            assert!(
                out.text.contains("body of /work/plan.md"),
                "{path}: {}",
                out.text
            );
            assert_eq!(out.chunks.len(), 1, "{path}");
        }

        let out = runner
            .run(&ToolCall::RelatedNotes {
                path: "journal/2026-11-18.md".into(),
            })
            .await;
        let mut to = listed(&out.text, "Links to");
        to.sort();
        assert_eq!(to, ["/journal/local.md", "/work/dup.md", "/work/plan.md"]);
        assert!(listed(&out.text, "Missing").is_empty(), "{}", out.text);
        let out = runner
            .run(&ToolCall::RelatedNotes {
                path: "/work/plan.md".into(),
            })
            .await;
        assert_eq!(listed(&out.text, "Linked from"), ["/journal/2026-11-18.md"]);
    }
}
