//! The answer loop of an agentic ask (adr/0065).
//!
//! The first search always runs on its own, on the conversation-conditioned
//! query. The model then reads the question with those results and may call
//! the tools of [`crate::tools`] for at most [`MAX_TOOL_ROUNDS`] rounds before
//! it must answer. Every chunk the model reads gets one number in a single
//! merged list, so each `[n]` citation points into the sources of the answer.
//! A model or endpoint that cannot call tools gets the single-pass answer.

use chrono::{Datelike, Duration, NaiveDate, Weekday};
use serde_json::Value;

use crate::document::JOURNAL_SOURCE;
use crate::llmclients::{
    ChatMessage, FOLLOWUP_INSTRUCTION, LLMClient, ModelReply, ModelToolCall, ToolsUnsupported,
    first_line, recap_line, render_context,
};
use crate::tools::{TRUNCATED_MARKER, ToolOutput, ToolRunner, parse_call, tool_specs};
use crate::{CollectionKey, KimunRag, RagError, ScoredChunk, build_retrieval_query};

/// Tool rounds the model gets before it must answer with what it has.
pub const MAX_TOOL_ROUNDS: usize = 5;

/// The label the model must put before passages from its own knowledge.
pub const MODEL_ONLY_LABEL: &str = "Not from your notes:";

/// Put before the answer when no note was gathered.
pub const NO_MATCH_NOTICE: &str =
    "No notes matched this question. The answer below comes from the model, not from your notes.";

/// Put before the answer when notes were gathered but the answer cites none
/// of them. The first search always keeps at least one chunk, so
/// [`NO_MATCH_NOTICE`] alone is rare.
pub const NO_CITATION_NOTICE: &str =
    "This answer cites none of your notes. Treat it as coming from the model, not from your notes.";

/// One plain-words step of what the assistant did, for the trail.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrailStep {
    pub text: String,
}

/// The result of [`run`].
pub struct AgentOutcome {
    pub text: String,
    /// The merged chunks, each with the `[n]` number the model saw.
    pub sources: Vec<(usize, ScoredChunk)>,
    pub trail: Vec<TrailStep>,
}

const FIRST_SEARCH_STEP: &str = "Searching your notes";
const SKIPPED_STEP: &str = "Tool loop skipped: single-pass answer";
const PROVIDER_ERROR_STEP: &str = "Tool loop stopped: provider error";
const MALFORMED_STEP: &str = "Tool loop stopped: the model sent malformed tool calls";
const CAP_STEP: &str = "Tool loop stopped: step limit reached";

/// Malformed calls in a row that end the loop.
const MALFORMED_LIMIT: usize = 2;

/// The trail as it grows: every step goes to `on_step` the moment it exists.
struct Trail<'a> {
    steps: Vec<TrailStep>,
    on_step: &'a (dyn Fn(&TrailStep) + Send + Sync),
}

impl Trail<'_> {
    fn push(&mut self, text: impl Into<String>) {
        let step = TrailStep { text: text.into() };
        (self.on_step)(&step);
        self.steps.push(step);
    }
}

/// The one numbered list of every chunk the model was shown. A chunk keeps
/// the number it got first; numbers are never reassigned.
#[derive(Default)]
struct Sources {
    list: Vec<(usize, ScoredChunk)>,
}

impl Sources {
    /// Adds `chunks` (a chunk already listed, by path, title and text, is not
    /// added again) and returns each with its number, in the given order.
    fn add(&mut self, chunks: Vec<ScoredChunk>) -> Vec<(usize, ScoredChunk)> {
        chunks
            .into_iter()
            .map(|(score, chunk)| {
                let known = self.list.iter().find(|(_, (_, c))| {
                    c.doc_path == chunk.doc_path && c.title == chunk.title && c.text == chunk.text
                });
                let number = match known {
                    Some((n, _)) => *n,
                    None => {
                        let n = self.list.len() + 1;
                        self.list.push((n, (score, chunk.clone())));
                        n
                    }
                };
                (number, (score, chunk))
            })
            .collect()
    }
}

/// Why the loop hands the answer to the single-pass path.
enum Fallback {
    Skipped,
    ProviderError,
    Malformed,
    Cap,
}

impl Fallback {
    fn step(&self) -> &'static str {
        match self {
            Fallback::Skipped => SKIPPED_STEP,
            Fallback::ProviderError => PROVIDER_ERROR_STEP,
            Fallback::Malformed => MALFORMED_STEP,
            Fallback::Cap => CAP_STEP,
        }
    }
}

/// Answers `question` with the bounded tool loop. The first search runs on
/// the conversation-conditioned query (`build_retrieval_query`) before the
/// model is asked anything; the model then gets at most [`MAX_TOOL_ROUNDS`]
/// tool rounds and one last request with no tools. If that last request is
/// refused or still asks for tools, the cap fallback answers. A
/// `ToolsUnsupported` on the first request, any model error in a later round,
/// or two malformed calls in a row, also end the loop. Every fallback is the
/// single-pass [`LLMClient::ask`] on the chunks gathered so far, with today's
/// date put before the question. Any other error on the first request fails
/// the request. An answer gets [`NO_MATCH_NOTICE`] when no chunk was gathered
/// and [`NO_CITATION_NOTICE`] when it cites none of the gathered chunks.
#[allow(clippy::too_many_arguments)]
pub async fn run(
    rag: &KimunRag,
    llm: &dyn LLMClient,
    collection: &CollectionKey,
    question: &str,
    history: &[(String, String)],
    today: NaiveDate,
    top_k: usize,
    on_step: &(dyn Fn(&TrailStep) + Send + Sync),
) -> Result<AgentOutcome, RagError> {
    let mut trail = Trail {
        steps: Vec::new(),
        on_step,
    };
    let mut sources = Sources::default();

    // The first search, exactly as the single-pass answer runs it.
    let query = build_retrieval_query(question, history);
    let raw = rag.retrieve(collection, &query).await?;
    let mut first = rag.rank(&query, raw).await;
    first.truncate(rag.cut_len(&first, top_k));
    trail.push(FIRST_SEARCH_STEP);
    let first = sources.add(first);

    let system = system_prompt(today, !history.is_empty());
    let mut messages = history_messages(history);
    messages.push(ChatMessage::User(first_message(question, history, &first)));

    let specs = tool_specs();
    let runner = ToolRunner::new(rag, collection);
    let mut rounds = 0;
    let mut malformed_in_a_row = 0;

    let answer: Result<String, Fallback> = 'turns: loop {
        let tools = if rounds < MAX_TOOL_ROUNDS {
            &specs[..]
        } else {
            &[]
        };
        let reply = match llm.chat(&system, &messages, tools).await {
            Ok(reply) => reply,
            Err(e) if e.is::<ToolsUnsupported>() && rounds == 0 => {
                break Err(Fallback::Skipped);
            }
            Err(e) if e.is::<ToolsUnsupported>() && rounds >= MAX_TOOL_ROUNDS => {
                // The tool-less cap request was refused (Anthropic rejects
                // tool_use blocks sent without `tools`): that is the cap, not
                // a provider failure.
                break Err(Fallback::Cap);
            }
            Err(e) if rounds > 0 => {
                // A later round failed (tools refused, rate limit, server
                // error, context too long): keep the gathered work. Only the
                // error's first line is logged, never note text.
                log::warn!(
                    "The model request failed after {rounds} tool round(s) ({}); \
                     answering in a single pass from what was gathered",
                    first_line(&e.to_string(), ERROR_LOG_CHARS)
                );
                break Err(Fallback::ProviderError);
            }
            Err(e) => return Err(e.into()),
        };
        let (text, calls) = match reply {
            ModelReply::Text(text) => break Ok(text),
            ModelReply::Calls { text, calls } => (text, calls),
        };
        if rounds >= MAX_TOOL_ROUNDS {
            // Tools were not offered, yet the model asked for them. Any text
            // with the calls is a preamble ("Let me search..."), not an
            // answer.
            break Err(Fallback::Cap);
        }
        rounds += 1;
        messages.push(ChatMessage::Assistant {
            text,
            calls: calls.clone(),
        });
        for call in &calls {
            let result = match parse_model_call(call) {
                Ok(parsed) => {
                    malformed_in_a_row = 0;
                    let out = runner.run(&parsed).await;
                    trail.push(out.trail.clone());
                    result_text(out, &mut sources)
                }
                Err(message) => {
                    malformed_in_a_row += 1;
                    if malformed_in_a_row >= MALFORMED_LIMIT {
                        break 'turns Err(Fallback::Malformed);
                    }
                    format!("error: {message}")
                }
            };
            messages.push(ChatMessage::ToolResult {
                call_id: call.id.clone(),
                text: result,
            });
        }
    };

    let text = match answer {
        Ok(text) => text,
        Err(fallback) => {
            trail.push(fallback.step());
            // The single-pass prompt has no date; the question carries it.
            let dated = format!("Today is {}.\n\n{question}", today_line(today));
            llm.ask(&dated, history, &sources.list).await?
        }
    };
    let text = if sources.list.is_empty() {
        format!("{NO_MATCH_NOTICE}\n\n{text}")
    } else if !cites_any(&text, sources.list.len()) {
        format!("{NO_CITATION_NOTICE}\n\n{text}")
    } else {
        text
    };
    Ok(AgentOutcome {
        text,
        sources: sources.list,
        trail: trail.steps,
    })
}

/// Most characters of a model error line that go to the log.
const ERROR_LOG_CHARS: usize = 200;

/// True when `text` holds at least one `[n]` marker with `1 <= n <= count`.
/// Other brackets (`[text](url)`, `[Not from your notes:]`) are not markers.
fn cites_any(text: &str, count: usize) -> bool {
    text.split('[').skip(1).any(|rest| {
        rest.split_once(']').is_some_and(|(inside, _)| {
            !inside.is_empty()
                && inside.bytes().all(|b| b.is_ascii_digit())
                && inside
                    .parse::<usize>()
                    .is_ok_and(|n| (1..=count).contains(&n))
        })
    })
}

/// Today as the prompts show it: `YYYY-MM-DD (Weekday)`.
fn today_line(today: NaiveDate) -> String {
    today.format("%Y-%m-%d (%A)").to_string()
}

/// A model call as a tool call, or the reason it is malformed.
fn parse_model_call(call: &ModelToolCall) -> Result<crate::tools::ToolCall, String> {
    if let Value::String(raw) = &call.args {
        return Err(format!(
            "the arguments of `{}` are not valid JSON: {raw}",
            call.name
        ));
    }
    parse_call(&call.name, &call.args)
}

/// The text the model reads for a tool result. Chunks are shown numbered with
/// their place in the merged sources, so `[n]` cites the right one; any other
/// result (and an `error:`) is the tool's own text.
fn result_text(out: ToolOutput, sources: &mut Sources) -> String {
    if out.chunks.is_empty() {
        return out.text;
    }
    let truncated = out.text.ends_with(TRUNCATED_MARKER);
    let numbered = sources.add(out.chunks);
    let mut text = String::new();
    for entry in &numbered {
        let rendered = render_context(std::slice::from_ref(entry));
        let other_dates: Vec<String> = entry
            .1
            .1
            .meta
            .dates
            .iter()
            .filter(|d| d.source != JOURNAL_SOURCE)
            .map(|d| format!("{} {}", d.source, d.day))
            .collect();
        match rendered.split_once('\n') {
            Some((header, rest)) if !other_dates.is_empty() => {
                text.push_str(&format!(
                    "{header}\nDates: {}\n{rest}",
                    other_dates.join(", ")
                ));
            }
            _ => text.push_str(&rendered),
        }
    }
    let mut text = text.trim_end().to_string();
    if truncated {
        text.push('\n');
        text.push_str(TRUNCATED_MARKER);
    }
    text
}

/// The earlier turns as chat messages. A turn whose answer is blank is left
/// out whole: providers reject an empty assistant message.
fn history_messages(history: &[(String, String)]) -> Vec<ChatMessage> {
    history
        .iter()
        .filter(|(_, a)| !a.trim().is_empty())
        .flat_map(|(q, a)| {
            [
                ChatMessage::User(q.clone()),
                ChatMessage::Assistant {
                    text: Some(a.clone()),
                    calls: Vec::new(),
                },
            ]
        })
        .collect()
}

/// The question with the first search's numbered results.
fn first_message(
    question: &str,
    history: &[(String, String)],
    first: &[(usize, ScoredChunk)],
) -> String {
    let context = if first.is_empty() {
        "(no notes matched the first search)\n".to_string()
    } else {
        render_context(first)
    };
    let recap = recap_line(history);
    format!(
        "Results of a first search of the notes:\n---------------------\n{context}---------------------\n\n{recap}Question: {question}"
    )
}

/// The dates a model needs to turn a time phrase into a range, worked out
/// from `today` so the model copies them and never does calendar arithmetic.
/// Every step falls back to `today` at the calendar's ends, so it never panics.
fn date_guide(today: NaiveDate) -> String {
    let fmt = |d: NaiveDate| d.format("%Y-%m-%d").to_string();
    let days_back = |n: i64| today.checked_sub_signed(Duration::days(n)).unwrap_or(today);
    let yesterday = days_back(1);
    let week_from = days_back(7);
    let two_weeks_from = days_back(14);
    let week_start = days_back(i64::from(today.weekday().num_days_from_monday()));
    let month_start = today.with_day(1).unwrap_or(today);
    let last_month_end = month_start.pred_opt().unwrap_or(today);
    let last_month_start = last_month_end.with_day(1).unwrap_or(today);
    let year_start = NaiveDate::from_ymd_opt(today.year(), 1, 1).unwrap_or(today);
    let last_year = today.year() - 1;
    let last_year_start = NaiveDate::from_ymd_opt(last_year, 1, 1).unwrap_or(today);
    let last_year_end = NaiveDate::from_ymd_opt(last_year, 12, 31).unwrap_or(today);

    // "Last Monday" is the most recent Monday strictly before today, so a
    // weekday that is today itself maps to seven days back.
    let weekdays = [
        ("Monday", Weekday::Mon),
        ("Tuesday", Weekday::Tue),
        ("Wednesday", Weekday::Wed),
        ("Thursday", Weekday::Thu),
        ("Friday", Weekday::Fri),
        ("Saturday", Weekday::Sat),
        ("Sunday", Weekday::Sun),
    ]
    .map(|(name, day)| {
        let ahead = i64::from(today.weekday().num_days_from_monday())
            - i64::from(day.num_days_from_monday());
        let back = if ahead <= 0 { ahead + 7 } else { ahead };
        format!("{name} {}", fmt(days_back(back)))
    })
    .join(", ");

    format!(
        "Work out date ranges from today's date, and state in the answer the range you used.\n\
A period with a length is a clear range: \"the last N days\" or \"the last N weeks\" is the N full days ending yesterday, where a week is 7 days. For another number of days or weeks, count back from yesterday in the same way. For today's date these are:\n\
- yesterday: {yesterday}\n\
- the last week: {week_from} to {yesterday}; the last two weeks: {two_weeks_from} to {yesterday}\n\
- this week (from Monday): {week_start} to {today}\n\
- this month: {month_start} to {today}; last month: {last_month_start} to {last_month_end}\n\
- this year: {year_start} to {today}; last year: {last_year_start} to {last_year_end}\n\
- last <weekday>, the most recent such day before today: {weekdays}\n\
A period with a name is used as named: a date, a month or a year (\"in March\"), resolved from today's date.\n\
Use the range of the last week as the default only when the period is vague, for example \"recently\" or \"lately\". Never say that no range was given when the question names a period.\n\
Search the whole period in one search. Do not start with a shorter range and then widen it. In the answer, name the period the user asked for together with its dates exactly as listed here, for example \"the last two weeks ({two_weeks_from} to {yesterday})\", and never attach the dates of one period to the name of another.",
        yesterday = fmt(yesterday),
        week_from = fmt(week_from),
        two_weeks_from = fmt(two_weeks_from),
        week_start = fmt(week_start),
        today = fmt(today),
        month_start = fmt(month_start),
        last_month_start = fmt(last_month_start),
        last_month_end = fmt(last_month_end),
        year_start = fmt(year_start),
        last_year_start = fmt(last_year_start),
        last_year_end = fmt(last_year_end),
    )
}

/// The system prompt: today's date, how to use the tools, and the citation
/// and disclosure rules of the single-pass prompt.
fn system_prompt(today: NaiveDate, mid_conversation: bool) -> String {
    let today_line = today_line(today);
    // Panic-free at the calendar's ends: fall back to `today`.
    let date_guide = date_guide(today);
    let followup = if mid_conversation {
        FOLLOWUP_INSTRUCTION
    } else {
        ""
    };
    format!(
        r#"You are an intelligent assistant with access to a personal knowledge base of the user's notes.
Today is {today_line}.
The user's message holds the question and the numbered results of a first search of the notes. Answer from them when they are enough. When they are not, use the tools to look further: search again (with a date range for questions about a period of time, a folder or tags), list the date keys, read a whole note or one of its sections, or find the notes linked to or from a note. Look only as far as the question needs.
{date_guide}
{followup}Every claim drawn from the notes MUST carry an inline citation in the form [n], where n is the number of the supporting note entry, from the first search or from a tool result. A sentence may carry several citations. Cite only numbers you were shown.
You may supplement with accurate, widely accepted common knowledge when the notes fall short. Start each such passage with the label "{MODEL_ONLY_LABEL}" and never cite [n] for it.
Preserve any [[wikilinks]] and #tags that appear in the notes verbatim when you quote or reference them.
If neither the notes nor common knowledge suffices, respond with: 'I don't have enough information to answer.'"#
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ContextCut;
    use crate::dbembeddings::vecsqlite::VecSqlite;
    use crate::document::{KimunDoc, KimunSection};
    use crate::llmclients::{ChatMessage, ModelReply, ModelToolCall, ToolsUnsupported};
    use crate::test_support::FakeEmbedder;
    use crate::tools::ToolSpec;
    use async_trait::async_trait;
    use serde_json::{Value, json};
    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex};

    const CANNED_ASK: &str = "single-pass answer [1]";

    /// What the fake does on one `chat` call.
    enum Script {
        Reply(ModelReply),
        Unsupported,
        Fail,
    }

    /// One `chat` call as the fake received it.
    struct ChatSeen {
        system: String,
        messages: Vec<ChatMessage>,
        tools: usize,
    }

    /// A scripted model: `chat` plays the script in order (a plain text answer
    /// once it runs out), `ask` returns `ask_reply` or [`CANNED_ASK`], or
    /// fails when `ask_fails`. Records what it got.
    #[derive(Default)]
    struct FakeLlm {
        script: Mutex<VecDeque<Script>>,
        chats: Mutex<Vec<ChatSeen>>,
        asks: Mutex<Vec<Vec<(usize, ScoredChunk)>>>,
        ask_questions: Mutex<Vec<String>>,
        ask_reply: Option<String>,
        ask_fails: bool,
    }

    impl FakeLlm {
        fn new(script: Vec<Script>) -> Self {
            Self {
                script: Mutex::new(script.into()),
                ..Default::default()
            }
        }
    }

    #[async_trait]
    impl LLMClient for FakeLlm {
        async fn ask(
            &self,
            question: &str,
            _: &[(String, String)],
            context: &[(usize, ScoredChunk)],
        ) -> anyhow::Result<String> {
            self.asks.lock().unwrap().push(context.to_vec());
            self.ask_questions
                .lock()
                .unwrap()
                .push(question.to_string());
            if self.ask_fails {
                anyhow::bail!("single pass exploded");
            }
            Ok(self.ask_reply.as_deref().unwrap_or(CANNED_ASK).to_string())
        }

        async fn chat(
            &self,
            system: &str,
            messages: &[ChatMessage],
            tools: &[ToolSpec],
        ) -> anyhow::Result<ModelReply> {
            self.chats.lock().unwrap().push(ChatSeen {
                system: system.to_string(),
                messages: messages.to_vec(),
                tools: tools.len(),
            });
            match self.script.lock().unwrap().pop_front() {
                Some(Script::Reply(reply)) => Ok(reply),
                Some(Script::Unsupported) => Err(ToolsUnsupported.into()),
                Some(Script::Fail) => Err(anyhow::anyhow!("provider exploded")),
                None => Ok(ModelReply::Text("final answer".to_string())),
            }
        }
    }

    fn calls(calls: Vec<ModelToolCall>) -> Script {
        Script::Reply(ModelReply::Calls { text: None, calls })
    }

    fn text(t: &str) -> Script {
        Script::Reply(ModelReply::Text(t.to_string()))
    }

    fn call(id: &str, name: &str, args: Value) -> ModelToolCall {
        ModelToolCall::new(id, name, args)
    }

    fn key() -> CollectionKey {
        CollectionKey::parse("v").unwrap()
    }

    fn day(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
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

    /// A real SQLite store in a temp dir, seeded with `docs`. The fixed cut
    /// keeps `top_k` chunks of the first search, so tests know its size.
    async fn rag_with(docs: &[KimunDoc]) -> (KimunRag, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let store = VecSqlite::new(dir.path(), 8).await.unwrap();
        let rag = KimunRag::new(Arc::new(store), Arc::new(FakeEmbedder), None)
            .with_context_cut(ContextCut::Fixed);
        if !docs.is_empty() {
            rag.index(&key(), docs).await.unwrap();
        }
        (rag, dir)
    }

    fn vault() -> Vec<KimunDoc> {
        vec![
            doc("ideas.md", "project ideas for the garden"),
            doc("journal/2026-11-10.md", "standup about the old release"),
            doc("journal/2026-11-18.md", "standup with the team about tests"),
            doc("journal/2026-11-20.md", "standup about the new release"),
        ]
    }

    struct Run {
        outcome: AgentOutcome,
        seen_steps: Vec<String>,
    }

    async fn ask(rag: &KimunRag, llm: &FakeLlm, history: &[(String, String)]) -> Run {
        try_ask(rag, llm, history).await.unwrap()
    }

    async fn try_ask(
        rag: &KimunRag,
        llm: &FakeLlm,
        history: &[(String, String)],
    ) -> Result<Run, RagError> {
        let seen = Mutex::new(Vec::new());
        let on_step = |s: &TrailStep| seen.lock().unwrap().push(s.text.clone());
        let outcome = run(
            rag,
            llm,
            &key(),
            "what did the team do?",
            history,
            day("2026-11-25"),
            1,
            &on_step,
        )
        .await?;
        Ok(Run {
            outcome,
            seen_steps: seen.into_inner().unwrap(),
        })
    }

    fn trail(outcome: &AgentOutcome) -> Vec<&str> {
        outcome.trail.iter().map(|s| s.text.as_str()).collect()
    }

    fn tool_results(messages: &[ChatMessage]) -> Vec<(&str, &str)> {
        messages
            .iter()
            .filter_map(|m| match m {
                ChatMessage::ToolResult { call_id, text } => {
                    Some((call_id.as_str(), text.as_str()))
                }
                _ => None,
            })
            .collect()
    }

    /// Asserts the sources are numbered 1..n with no gaps and no repeated
    /// chunk.
    fn assert_numbered_once(sources: &[(usize, ScoredChunk)]) {
        let numbers: Vec<usize> = sources.iter().map(|(n, _)| *n).collect();
        assert_eq!(numbers, (1..=sources.len()).collect::<Vec<_>>());
        for (i, (_, (_, a))) in sources.iter().enumerate() {
            for (_, (_, b)) in &sources[i + 1..] {
                assert!(
                    !(a.doc_path == b.doc_path && a.title == b.title && a.text == b.text),
                    "repeated chunk {}",
                    a.doc_path
                );
            }
        }
    }

    #[tokio::test]
    async fn answers_in_one_round_when_the_model_asks_for_nothing() {
        let (rag, _dir) = rag_with(&vault()).await;
        let llm = FakeLlm::new(vec![text("the team met [1]")]);
        let run = ask(&rag, &llm, &[]).await;

        assert_eq!(run.outcome.text, "the team met [1]");
        assert_eq!(trail(&run.outcome), ["Searching your notes"]);
        assert_eq!(run.seen_steps, ["Searching your notes"]);
        assert_eq!(run.outcome.sources.len(), 1, "the first search ran");
        let chats = llm.chats.lock().unwrap();
        assert_eq!(chats.len(), 1);
        assert_eq!(chats[0].tools, 4, "the model is offered the four tools");
        let [ChatMessage::User(first)] = chats[0].messages.as_slice() else {
            panic!("one user message, got {:?}", chats[0].messages);
        };
        assert!(first.contains("what did the team do?"), "{first}");
        let path = &run.outcome.sources[0].1.1.doc_path;
        assert!(first.contains(&format!("[1] {path}")), "{first}");
        assert!(llm.asks.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn runs_a_tool_then_answers() {
        let (rag, _dir) = rag_with(&vault()).await;
        let llm = FakeLlm::new(vec![
            calls(vec![call(
                "c1",
                "search",
                json!({"query": "standup", "date_from": "2026-11-17", "date_to": "2026-11-23", "date_key": "journal"}),
            )]),
            text("done [1]"),
        ]);
        let run = ask(&rag, &llm, &[]).await;

        assert_eq!(run.outcome.text, "done [1]");
        let steps = [
            "Searching your notes",
            "Searching journal entries from 2026-11-17 to 2026-11-23",
        ];
        assert_eq!(trail(&run.outcome), steps);
        assert_eq!(run.seen_steps, steps, "on_step saw every step, in order");
        let sources = &run.outcome.sources;
        assert_numbered_once(sources);
        let paths: Vec<&str> = sources
            .iter()
            .map(|(_, (_, c))| c.doc_path.as_str())
            .collect();
        assert!(paths.contains(&"journal/2026-11-18.md"), "{paths:?}");
        assert!(paths.contains(&"journal/2026-11-20.md"), "{paths:?}");
        assert!(!paths.contains(&"journal/2026-11-10.md"), "{paths:?}");
        let chats = llm.chats.lock().unwrap();
        assert_eq!(chats.len(), 2);
        assert!(matches!(
            &chats[1].messages[1],
            ChatMessage::Assistant { calls, .. } if calls[0].id == "c1"
        ));
        assert_eq!(tool_results(&chats[1].messages).len(), 1);
    }

    #[tokio::test]
    async fn tool_results_carry_the_final_source_numbers() {
        let (rag, _dir) = rag_with(&vault()).await;
        // An unfiltered search returns every chunk, the first result among
        // them; a read returns a chunk the search already gave.
        let llm = FakeLlm::new(vec![
            calls(vec![call("c1", "search", json!({"query": "standup"}))]),
            calls(vec![call(
                "c2",
                "read_note",
                json!({"path": "journal/2026-11-18.md"}),
            )]),
            text("done"),
        ]);
        let run = ask(&rag, &llm, &[]).await;
        let sources = &run.outcome.sources;
        assert_numbered_once(sources);
        assert_eq!(sources.len(), 4, "every note once: {sources:?}");

        let chats = llm.chats.lock().unwrap();
        let ChatMessage::User(first) = &chats[0].messages[0] else {
            panic!()
        };
        let results = tool_results(&chats[2].messages);
        let all_seen = format!("{first}\n{}\n{}", results[0].1, results[1].1);
        for (n, (_, chunk)) in sources {
            assert!(
                all_seen.contains(&format!("[{n}] {}", chunk.doc_path)),
                "[{n}] {} not in what the model read:\n{all_seen}",
                chunk.doc_path
            );
        }
        // The first result keeps number 1 when the search returns it again,
        // and the read shows the number its chunk already had.
        let first_path = &sources[0].1.1.doc_path;
        assert!(results[0].1.contains(&format!("[1] {first_path}")));
        let read_number = sources
            .iter()
            .find(|(_, (_, c))| c.doc_path == "journal/2026-11-18.md")
            .unwrap()
            .0;
        assert!(
            results[1]
                .1
                .contains(&format!("[{read_number}] journal/2026-11-18.md")),
            "{}",
            results[1].1
        );
    }

    #[tokio::test]
    async fn stops_at_the_step_cap() {
        let (rag, _dir) = rag_with(&vault()).await;
        // Every reply asks for a search, with a preamble the loop must never
        // take as the answer.
        let script = (0..10)
            .map(|i| {
                Script::Reply(ModelReply::Calls {
                    text: Some("Let me search some more...".to_string()),
                    calls: vec![call(
                        &format!("c{i}"),
                        "search",
                        json!({"query": "standup"}),
                    )],
                })
            })
            .collect();
        let llm = FakeLlm::new(script);
        let run = ask(&rag, &llm, &[]).await;

        assert_eq!(trail(&run.outcome).last(), Some(&CAP_STEP));
        assert!(!trail(&run.outcome).contains(&PROVIDER_ERROR_STEP));
        let asks = llm.asks.lock().unwrap();
        assert_eq!(asks.len(), 1, "the answer comes from the single-pass path");
        assert_eq!(asks[0].len(), 4, "asked on every gathered chunk");
        assert_eq!(asks[0], run.outcome.sources);
        assert_numbered_once(&asks[0]);
        let chats = llm.chats.lock().unwrap();
        assert_eq!(chats.len(), MAX_TOOL_ROUNDS + 1);
        assert!(chats[..MAX_TOOL_ROUNDS].iter().all(|c| c.tools == 4));
        assert_eq!(
            chats[MAX_TOOL_ROUNDS].tools, 0,
            "the last request has no tools"
        );
        assert_eq!(
            tool_results(&chats[MAX_TOOL_ROUNDS].messages).len(),
            MAX_TOOL_ROUNDS
        );
        // The model still asked for tools with none offered: the loop does
        // not go on, and the answer comes from the single-pass path.
        assert_eq!(run.outcome.text, CANNED_ASK);
        assert_eq!(run.outcome.trail.len(), 1 + MAX_TOOL_ROUNDS + 1);
    }

    #[tokio::test]
    async fn tools_unsupported_on_the_cap_request_is_the_cap_fallback() {
        // Anthropic can reject a tool-less request that still carries
        // tool_use blocks; that is the cap, not a provider failure.
        let (rag, _dir) = rag_with(&vault()).await;
        let mut script: Vec<Script> = (0..MAX_TOOL_ROUNDS)
            .map(|i| {
                calls(vec![call(
                    &format!("c{i}"),
                    "search",
                    json!({"query": "standup"}),
                )])
            })
            .collect();
        script.push(Script::Unsupported);
        let llm = FakeLlm::new(script);
        let run = ask(&rag, &llm, &[]).await;

        assert_eq!(llm.chats.lock().unwrap().len(), MAX_TOOL_ROUNDS + 1);
        assert_eq!(trail(&run.outcome).last(), Some(&CAP_STEP));
        assert!(!trail(&run.outcome).contains(&PROVIDER_ERROR_STEP));
        assert_eq!(run.outcome.text, CANNED_ASK);
        let asks = llm.asks.lock().unwrap();
        assert_eq!(asks.len(), 1);
        assert_eq!(asks[0].len(), 4, "asked on every gathered chunk");
        assert_eq!(asks[0], run.outcome.sources);
    }

    #[tokio::test]
    async fn answer_at_the_cap_is_used() {
        let (rag, _dir) = rag_with(&vault()).await;
        let mut script: Vec<Script> = (0..MAX_TOOL_ROUNDS)
            .map(|i| calls(vec![call(&format!("c{i}"), "list_date_keys", json!({}))]))
            .collect();
        script.push(text("capped answer [1]"));
        let llm = FakeLlm::new(script);
        let run = ask(&rag, &llm, &[]).await;
        assert_eq!(run.outcome.text, "capped answer [1]");
        assert_eq!(llm.chats.lock().unwrap().len(), MAX_TOOL_ROUNDS + 1);
        assert!(llm.asks.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn bad_call_is_returned_to_the_model_and_counts_once() {
        let (rag, _dir) = rag_with(&vault()).await;
        let llm = FakeLlm::new(vec![
            calls(vec![call("c1", "delete_everything", json!({}))]),
            calls(vec![call("c2", "list_date_keys", json!({}))]),
            calls(vec![call("c3", "search", json!({}))]),
            text("done [1]"),
        ]);
        let run = ask(&rag, &llm, &[]).await;

        assert_eq!(run.outcome.text, "done [1]", "the loop went on");
        assert!(llm.asks.lock().unwrap().is_empty(), "no fallback");
        let chats = llm.chats.lock().unwrap();
        let results = tool_results(&chats[3].messages);
        assert_eq!(results.len(), 3);
        assert_eq!(results[0].0, "c1");
        assert!(results[0].1.starts_with("error:"), "{}", results[0].1);
        assert!(results[0].1.contains("delete_everything"));
        assert!(!results[1].1.starts_with("error:"));
        assert!(results[2].1.starts_with("error:"), "{}", results[2].1);
        assert_eq!(
            trail(&run.outcome),
            ["Searching your notes", "Listing date properties"]
        );
    }

    #[tokio::test]
    async fn two_malformed_calls_in_a_row_fall_back_to_single_pass() {
        let (rag, _dir) = rag_with(&vault()).await;
        let llm = FakeLlm::new(vec![
            calls(vec![call("c1", "delete_everything", json!({}))]),
            calls(vec![call(
                "c2",
                "search",
                Value::String("{not json".into()),
            )]),
            text("never read"),
        ]);
        let run = ask(&rag, &llm, &[]).await;

        assert_eq!(run.outcome.text, CANNED_ASK);
        assert_eq!(llm.chats.lock().unwrap().len(), 2);
        let asks = llm.asks.lock().unwrap();
        assert_eq!(asks.len(), 1);
        assert_eq!(asks[0], run.outcome.sources, "asked on the gathered chunks");
        assert_eq!(
            trail(&run.outcome),
            [
                "Searching your notes",
                "Tool loop stopped: the model sent malformed tool calls"
            ]
        );
    }

    #[tokio::test]
    async fn tools_unsupported_falls_back_and_the_trail_says_so() {
        let (rag, _dir) = rag_with(&vault()).await;
        let llm = FakeLlm::new(vec![Script::Unsupported]);
        let run = ask(&rag, &llm, &[]).await;

        assert_eq!(run.outcome.text, CANNED_ASK);
        let steps = [
            "Searching your notes",
            "Tool loop skipped: single-pass answer",
        ];
        assert_eq!(trail(&run.outcome), steps);
        assert_eq!(run.seen_steps, steps);
        let asks = llm.asks.lock().unwrap();
        assert_eq!(asks.len(), 1);
        assert_eq!(asks[0].len(), 1, "the first results are the context");
        assert_eq!(asks[0], run.outcome.sources);
    }

    #[tokio::test]
    async fn tools_unsupported_in_a_later_round_answers_from_what_was_gathered() {
        let (rag, _dir) = rag_with(&vault()).await;
        let llm = FakeLlm::new(vec![
            calls(vec![call("c1", "search", json!({"query": "standup"}))]),
            Script::Unsupported,
        ]);
        let run = ask(&rag, &llm, &[]).await;

        assert_eq!(run.outcome.text, CANNED_ASK);
        assert_eq!(
            trail(&run.outcome).last(),
            Some(&"Tool loop stopped: provider error")
        );
        let asks = llm.asks.lock().unwrap();
        assert_eq!(asks.len(), 1);
        assert_eq!(asks[0].len(), 4, "the search results are in the context");
        assert_eq!(asks[0], run.outcome.sources);
    }

    #[tokio::test]
    async fn chat_error_in_a_later_round_answers_from_what_was_gathered() {
        let (rag, _dir) = rag_with(&vault()).await;
        let llm = FakeLlm::new(vec![
            calls(vec![call("c1", "search", json!({"query": "standup"}))]),
            Script::Fail,
        ]);
        let run = ask(&rag, &llm, &[]).await;

        assert_eq!(run.outcome.text, CANNED_ASK);
        assert_eq!(trail(&run.outcome).last(), Some(&PROVIDER_ERROR_STEP));
        let asks = llm.asks.lock().unwrap();
        assert_eq!(asks.len(), 1);
        assert_eq!(asks[0].len(), 4, "the search results are in the context");
        assert_eq!(asks[0], run.outcome.sources);
    }

    #[tokio::test]
    async fn failing_single_pass_after_a_later_round_error_fails_the_request() {
        let (rag, _dir) = rag_with(&vault()).await;
        let llm = FakeLlm {
            ask_fails: true,
            ..FakeLlm::new(vec![
                calls(vec![call("c1", "search", json!({"query": "standup"}))]),
                Script::Fail,
            ])
        };
        let err = try_ask(&rag, &llm, &[]).await.err().expect("an error");
        assert!(err.to_string().contains("single pass exploded"), "{err}");
    }

    #[tokio::test]
    async fn fallback_question_carries_today() {
        let (rag, _dir) = rag_with(&vault()).await;
        let llm = FakeLlm::new(vec![Script::Unsupported]);
        ask(&rag, &llm, &[]).await;
        let questions = llm.ask_questions.lock().unwrap();
        assert_eq!(
            questions.as_slice(),
            ["Today is 2026-11-25 (Wednesday).\n\nwhat did the team do?"]
        );
    }

    #[tokio::test]
    async fn other_chat_errors_fail_the_request() {
        let (rag, _dir) = rag_with(&vault()).await;
        let llm = FakeLlm::new(vec![Script::Fail]);
        let err = try_ask(&rag, &llm, &[]).await.err().expect("an error");
        assert!(matches!(err, RagError::Backend(_)), "{err:?}");
        assert!(err.to_string().contains("provider exploded"));
        assert!(llm.asks.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn empty_results_add_the_no_match_notice() {
        let (rag, _dir) = rag_with(&[]).await;
        let llm = FakeLlm::new(vec![text("from memory")]);
        let run = ask(&rag, &llm, &[]).await;
        assert!(run.outcome.sources.is_empty());
        assert_eq!(
            run.outcome.text,
            format!("{NO_MATCH_NOTICE}\n\nfrom memory")
        );

        // With sources and a fallback that cites them, no notice.
        let (rag, _dir) = rag_with(&vault()).await;
        let llm = FakeLlm::new(vec![Script::Unsupported]);
        let run = ask(&rag, &llm, &[]).await;
        assert_eq!(run.outcome.text, CANNED_ASK);

        // An empty list keeps the no-match notice, also on the fallback.
        let (rag, _dir) = rag_with(&[]).await;
        let llm = FakeLlm::new(vec![Script::Unsupported]);
        let run = ask(&rag, &llm, &[]).await;
        assert_eq!(
            run.outcome.text,
            format!("{NO_MATCH_NOTICE}\n\n{CANNED_ASK}")
        );
    }

    #[tokio::test]
    async fn answer_citing_no_note_adds_the_no_citation_notice() {
        let (rag, _dir) = rag_with(&vault()).await;
        for (answer, notice) in [
            ("the team met", true),
            ("the team met [1]", false),
            ("the team met [1][7]", false),
            ("the team met [7]", true),
            ("the team met [0]", true),
            (
                "see [the plan](https://x.org) and [Not from your notes:] [x]",
                true,
            ),
            ("Not from your notes: tests help. The team met [1].", false),
        ] {
            let llm = FakeLlm::new(vec![text(answer)]);
            let run = ask(&rag, &llm, &[]).await;
            assert_eq!(run.outcome.sources.len(), 1);
            let want = if notice {
                format!("{NO_CITATION_NOTICE}\n\n{answer}")
            } else {
                answer.to_string()
            };
            assert_eq!(run.outcome.text, want, "{answer}");
        }

        // The fallback answer gets the notice too when it cites nothing.
        let llm = FakeLlm {
            ask_reply: Some("from memory".to_string()),
            ..FakeLlm::new(vec![Script::Unsupported])
        };
        let run = ask(&rag, &llm, &[]).await;
        assert_eq!(
            run.outcome.text,
            format!("{NO_CITATION_NOTICE}\n\nfrom memory")
        );
    }

    #[test]
    fn system_prompt_survives_the_calendar_ends() {
        assert!(system_prompt(NaiveDate::MIN, false).contains("Today is"));
        assert!(system_prompt(NaiveDate::MAX, true).contains("Today is"));
    }

    #[tokio::test]
    async fn prompt_contains_today_and_the_label_rule() {
        let (rag, _dir) = rag_with(&vault()).await;
        let llm = FakeLlm::new(vec![text("ok")]);
        ask(&rag, &llm, &[]).await;
        let chats = llm.chats.lock().unwrap();
        let system = &chats[0].system;
        assert!(system.contains("2026-11-25 (Wednesday)"), "{system}");
        assert!(system.contains(MODEL_ONLY_LABEL), "{system}");
        assert!(
            system.contains("2026-11-18 to 2026-11-24"),
            "the default range is the seven days ending yesterday: {system}"
        );
        assert!(system.contains("[n]"), "{system}");
    }

    #[test]
    fn the_date_guide_gives_a_range_for_every_common_phrase() {
        // Reported bug: "the last two weeks" was answered with the seven-day
        // default and the claim that no range was specified. The guide now
        // spells out each common phrase for a known today (Wednesday).
        let today = NaiveDate::from_ymd_opt(2026, 11, 25).unwrap();
        let system = system_prompt(today, false);
        for (what, expected) in [
            ("yesterday", "yesterday: 2026-11-24"),
            ("the last week", "2026-11-18 to 2026-11-24"),
            ("the last two weeks", "2026-11-11 to 2026-11-24"),
            (
                "this week, from Monday",
                "this week (from Monday): 2026-11-23 to 2026-11-25",
            ),
            ("this month", "this month: 2026-11-01 to 2026-11-25"),
            ("last month", "last month: 2026-10-01 to 2026-10-31"),
            ("this year", "this year: 2026-01-01 to 2026-11-25"),
            ("last year", "last year: 2025-01-01 to 2025-12-31"),
            ("last Monday", "Monday 2026-11-23"),
            ("last Tuesday", "Tuesday 2026-11-24"),
            // Today is a Wednesday: "last Wednesday" is a week ago, not today.
            ("last Wednesday", "Wednesday 2026-11-18"),
            ("last Thursday", "Thursday 2026-11-19"),
            ("last Friday", "Friday 2026-11-20"),
            ("last Saturday", "Saturday 2026-11-21"),
            ("last Sunday", "Sunday 2026-11-22"),
        ] {
            assert!(
                system.contains(expected),
                "{what}: missing '{expected}' in {system}"
            );
        }
        assert!(
            system.contains("A period with a length is a clear range"),
            "{system}"
        );
        assert!(
            system.contains("only when the period is vague"),
            "the default must be limited to vague periods: {system}"
        );
        assert!(
            !system.contains("range is not clear"),
            "the old rule that invited the default for named periods is gone: {system}"
        );
    }

    #[test]
    fn the_guide_names_each_period_with_its_own_dates_and_does_not_repeat_the_default() {
        // Reported answer: it called 2026-11-18 to 2026-11-24 "the last two
        // weeks", then added the earlier week as a "wider window". The seven
        // days must appear once, tied to "the last week", and the model must
        // be told to search the whole period at once and to pair each period
        // name with its own dates.
        let today = NaiveDate::from_ymd_opt(2026, 11, 25).unwrap();
        let system = system_prompt(today, false);
        assert_eq!(
            system.matches("2026-11-18 to 2026-11-24").count(),
            1,
            "the seven-day dates must not be repeated as a second anchor: {system}"
        );
        assert!(
            system.contains("Search the whole period in one search"),
            "{system}"
        );
        assert!(
            system.contains("never attach the dates of one period to the name of another"),
            "{system}"
        );
        assert!(
            system.contains("the last two weeks (2026-11-11 to 2026-11-24)"),
            "the example in the rule uses the real dates: {system}"
        );
    }

    #[test]
    fn last_weekday_is_always_strictly_before_today() {
        // Every weekday of one week: the date given for "last <today's own
        // weekday>" is exactly seven days back, never today.
        for offset in 0..7 {
            let today = NaiveDate::from_ymd_opt(2026, 11, 23).unwrap() + Duration::days(offset);
            let system = system_prompt(today, false);
            let name = today.format("%A").to_string();
            let a_week_ago = (today - Duration::days(7)).format("%Y-%m-%d").to_string();
            assert!(
                system.contains(&format!("{name} {a_week_ago}")),
                "{today} ({name}): {system}"
            );
        }
    }

    #[test]
    fn the_guide_handles_month_and_year_edges() {
        // January: "last month" is December of the year before.
        let jan = system_prompt(NaiveDate::from_ymd_opt(2026, 1, 5).unwrap(), false);
        assert!(
            jan.contains("last month: 2025-12-01 to 2025-12-31"),
            "{jan}"
        );
        // March after a leap year: February has 29 days.
        let mar = system_prompt(NaiveDate::from_ymd_opt(2028, 3, 2).unwrap(), false);
        assert!(
            mar.contains("last month: 2028-02-01 to 2028-02-29"),
            "{mar}"
        );
        // A Monday: "this week" is just today.
        let monday = system_prompt(NaiveDate::from_ymd_opt(2026, 11, 23).unwrap(), false);
        assert!(
            monday.contains("this week (from Monday): 2026-11-23 to 2026-11-23"),
            "{monday}"
        );
    }

    #[test]
    fn the_date_guide_is_panic_free_at_the_calendar_ends() {
        // Same guarantee as `system_prompt_survives_the_calendar_ends`, for
        // every worked range in the guide.
        let min = system_prompt(NaiveDate::MIN, false);
        let max = system_prompt(NaiveDate::MAX, false);
        assert!(min.contains("A period with a length is a clear range"));
        assert!(max.contains("A period with a length is a clear range"));
    }

    #[tokio::test]
    async fn history_becomes_messages_and_blank_answers_are_skipped() {
        let (rag, _dir) = rag_with(&vault()).await;
        let llm = FakeLlm::new(vec![text("ok")]);
        let history = vec![
            ("first q".to_string(), "first a".to_string()),
            ("lost q".to_string(), "  ".to_string()),
        ];
        ask(&rag, &llm, &history).await;
        let chats = llm.chats.lock().unwrap();
        let messages = &chats[0].messages;
        assert_eq!(messages.len(), 3, "{messages:?}");
        assert_eq!(messages[0], ChatMessage::User("first q".into()));
        assert_eq!(
            messages[1],
            ChatMessage::Assistant {
                text: Some("first a".into()),
                calls: vec![]
            }
        );
        assert!(
            matches!(&messages[2], ChatMessage::User(q) if q.contains("what did the team do?"))
        );
    }

    #[tokio::test]
    async fn missing_today_uses_the_server_clock() {
        let dir = tempfile::tempdir().unwrap();
        let store = VecSqlite::new(dir.path(), 8).await.unwrap();
        let llm = Arc::new(FakeLlm::new(vec![text("ok")]));
        let rag = KimunRag::new(Arc::new(store), Arc::new(FakeEmbedder), Some(llm.clone()));
        rag.index(&key(), &vault()).await.unwrap();
        let before = chrono::Local::now().date_naive();
        let answer = rag
            .answer(&key(), "q", &[], 1, None, &|_: &TrailStep| {})
            .await
            .unwrap();
        let after = chrono::Local::now().date_naive();
        assert_eq!(answer.trail[0].text, "Searching your notes");
        let system = &llm.chats.lock().unwrap()[0].system;
        assert!(
            system.contains(&before.format("%Y-%m-%d").to_string())
                || system.contains(&after.format("%Y-%m-%d").to_string()),
            "{system}"
        );
    }
}
