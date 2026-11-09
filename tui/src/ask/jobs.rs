//! The Ask workspace's server job, toolkit-free: one question (with its
//! conversation history) in, one [`AskData::AnswerReady`] out. The TUI's
//! `ThreadPanel` spawns it for a new turn and for a regenerate; a GUI calls the
//! same function on its own runtime.

use std::sync::Arc;

use crate::logic::message::{AppMessage, AppTx, AskData};
use crate::server_client::RagClient;

use super::AskSource;

/// Ask the server `question` with `history` and map the reply: the answer text
/// plus its freshly retrieved sources, each normalised through
/// [`AskSource::from_chunk`] (so a `0` wire ordinal becomes the 1-based
/// position). A failure is the error's display text.
pub async fn answer(
    client: &RagClient,
    question: &str,
    history: &[(String, String)],
) -> Result<(String, Vec<AskSource>), String> {
    client
        .ask(question, history, None)
        .await
        .map(|a| {
            // Normalize the wire ordinal ONCE here (position → 1-based
            // fallback for an older server); downstream sees real ordinals.
            let sources = a
                .sources
                .into_iter()
                .enumerate()
                .map(|(i, c)| AskSource::from_chunk(i, c))
                .collect();
            (a.answer, sources)
        })
        .map_err(|e| e.to_string())
}

/// The ask job for turn `turn_id`: [`answer`], then deliver
/// `AppMessage::Ask(AskData::AnswerReady { turn_id, result })` on `tx`. Both a
/// new turn and a regenerate run it — both re-retrieve, so both take the fresh
/// sources from the response. Spawn it (`tokio::spawn(ask(..))`); a closed
/// channel drops the result.
pub async fn ask(
    client: Arc<RagClient>,
    question: String,
    history: Vec<(String, String)>,
    turn_id: u64,
    tx: AppTx,
) {
    let result = answer(&client, &question, &history).await;
    let _ = tx.send(AppMessage::Ask(AskData::AnswerReady { turn_id, result }));
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use async_trait::async_trait;

    use super::*;
    use crate::server_client::{AnswerResult, ChunkResult, ContextSize, QueryTransport, RagError};

    fn chunk(name: &str, ordinal: usize) -> ChunkResult {
        ChunkResult {
            path: kimun_core::nfs::VaultPath::note_path_from(name).to_string(),
            title: "Heading".to_string(),
            date: None,
            content: "text".to_string(),
            hash: String::new(),
            similarity_score: 0.5,
            ordinal,
        }
    }

    type Asked = Vec<(String, Vec<(String, String)>)>;

    /// Answers every question with `reply` (or fails with `error`), recording
    /// what was asked.
    struct Stub {
        reply: Option<(String, Vec<ChunkResult>)>,
        asked: Mutex<Asked>,
    }

    #[async_trait]
    impl QueryTransport for Stub {
        async fn search(
            &self,
            _query: &str,
            _context_size: Option<ContextSize>,
        ) -> Result<Vec<ChunkResult>, RagError> {
            Ok(Vec::new())
        }
        async fn ask(
            &self,
            query: &str,
            history: &[(String, String)],
            _context_size: Option<ContextSize>,
        ) -> Result<AnswerResult, RagError> {
            self.asked
                .lock()
                .unwrap()
                .push((query.to_string(), history.to_vec()));
            match &self.reply {
                Some((answer, sources)) => Ok(AnswerResult {
                    answer: answer.clone(),
                    sources: sources.clone(),
                }),
                None => Err(RagError::Protocol("answer job failed".into())),
            }
        }
    }

    fn client(stub: Arc<Stub>) -> Arc<RagClient> {
        Arc::new(RagClient::with_transport(stub))
    }

    #[tokio::test]
    async fn an_answer_lands_with_normalised_ordinals_and_its_turn_id() {
        let stub = Arc::new(Stub {
            // The first source omits its ordinal (an older server), the second
            // carries one: 0 becomes the 1-based position, 7 stays.
            reply: Some(("see [1] and [7]".into(), vec![chunk("a", 0), chunk("b", 7)])),
            asked: Mutex::new(Vec::new()),
        });
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let history = vec![("q0".to_string(), "a0".to_string())];
        ask(client(stub.clone()), "why?".into(), history.clone(), 42, tx).await;

        let Some(AppMessage::Ask(AskData::AnswerReady { turn_id, result })) = rx.recv().await
        else {
            panic!("expected AnswerReady");
        };
        assert_eq!(turn_id, 42);
        let (answer, sources) = result.expect("ok");
        assert_eq!(answer, "see [1] and [7]");
        let ordinals: Vec<usize> = sources.iter().map(|s| s.ordinal).collect();
        assert_eq!(ordinals, vec![1, 7]);
        assert_eq!(sources[0].path.get_clean_name(), "a");
        assert_eq!(
            stub.asked.lock().unwrap().as_slice(),
            &[("why?".to_string(), history)]
        );
    }

    #[tokio::test]
    async fn a_failure_lands_as_its_message() {
        let stub = Arc::new(Stub {
            reply: None,
            asked: Mutex::new(Vec::new()),
        });
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        ask(client(stub), "why?".into(), Vec::new(), 3, tx).await;

        let Some(AppMessage::Ask(AskData::AnswerReady { turn_id, result })) = rx.recv().await
        else {
            panic!("expected AnswerReady");
        };
        assert_eq!(turn_id, 3);
        assert_eq!(result.unwrap_err(), "answer job failed");
    }
}
