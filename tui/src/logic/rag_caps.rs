//! What the Kimün server can do right now, from its [`RagStatus`]: whether the
//! **SEM** (semantic search) and **ASK** surfaces are offered, when either
//! flips, and the flash a route to an unavailable one shows. The background
//! sync (`crate::rag::spawn_rag_sync`) reports the status; a UI rebuilds its
//! rail on a [`Flip`] and swaps the Ask client with [`ask_client`].

use std::sync::Arc;

use kimun_core::NoteVault;

use crate::rag::RagStatus;
use crate::server_client::RagClient;
use crate::settings::SharedSettings;

/// Which server-gated surfaces are offered. Named fields instead of two
/// positional bools so a caller can't silently swap SEM and ASK.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Caps {
    /// SEM appears when the server is reachable for search (an embedder is
    /// configured) — `RagStatus::search_available`, mirroring how `ask` tracks
    /// `RagStatus::llm_available`.
    pub semantic: bool,
    /// ASK appears when the server can answer questions (an LLM configured).
    pub ask: bool,
}

/// Which capabilities changed between two statuses.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Flip {
    pub semantic: bool,
    pub ask: bool,
}

impl Flip {
    /// Either capability changed — the rail's entries must be rebuilt.
    pub fn any(self) -> bool {
        self.semantic || self.ask
    }
}

impl Caps {
    /// The capabilities `status` grants.
    pub fn from_status(status: RagStatus) -> Self {
        Self {
            semantic: status.search_available(),
            ask: status.llm_available(),
        }
    }

    /// What changed from `prev` to `next`. A rail rebuilds on [`Flip::any`];
    /// only an ASK flip needs a fresh Ask client.
    pub fn changed(prev: Caps, next: Caps) -> Flip {
        Flip {
            semantic: prev.semantic != next.semantic,
            ask: prev.ask != next.ask,
        }
    }
}

/// The flash for a route to SEM that the server can't serve, or `None` when
/// search is available. `configured` = a server URL is set
/// (`crate::rag::rag_configured`).
pub fn semantic_unavailable_message(status: RagStatus, configured: bool) -> Option<&'static str> {
    if status.search_available() {
        None
    } else if configured {
        Some("Semantic search needs a reachable server with an embedder")
    } else {
        Some("Set kimun_server_url in config to use semantic search")
    }
}

/// The flash for a route into ASK that the server can't serve, or `None` when
/// it can answer questions.
pub fn ask_unavailable_message(status: RagStatus) -> Option<&'static str> {
    (!status.llm_available())
        .then_some("Ask needs an LLM-configured server; this one is semantic-search only")
}

/// The Ask client for `status`: built from config when the server can answer
/// questions, else `None` (the composer disables; the thread stays readable).
/// Async: reads the vault id.
pub async fn ask_client(
    settings: &SharedSettings,
    vault: &NoteVault,
    status: RagStatus,
) -> Option<Arc<RagClient>> {
    if status.llm_available() {
        crate::rag::rag_client(settings, vault).await.map(Arc::new)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every status, with the caps it grants: `(status, semantic, ask)`.
    const TABLE: [(RagStatus, bool, bool); 8] = [
        (RagStatus::Disabled, false, false),
        (RagStatus::Offline, false, false),
        (RagStatus::Unauthorized, false, false),
        (RagStatus::NotConfigured, false, false),
        (
            RagStatus::Syncing {
                llm_available: false,
            },
            true,
            false,
        ),
        (
            RagStatus::Syncing {
                llm_available: true,
            },
            true,
            true,
        ),
        (
            RagStatus::Online {
                llm_available: false,
            },
            true,
            false,
        ),
        (
            RagStatus::Online {
                llm_available: true,
            },
            true,
            true,
        ),
    ];

    #[test]
    fn caps_follow_every_status() {
        for (status, semantic, ask) in TABLE {
            assert_eq!(
                Caps::from_status(status),
                Caps { semantic, ask },
                "{status:?}"
            );
        }
    }

    #[test]
    fn flips_name_what_changed() {
        let none = Caps::default();
        let sem = Caps {
            semantic: true,
            ask: false,
        };
        let both = Caps {
            semantic: true,
            ask: true,
        };
        assert_eq!(Caps::changed(none, none), Flip::default());
        assert!(!Caps::changed(both, both).any());
        assert_eq!(
            Caps::changed(none, sem),
            Flip {
                semantic: true,
                ask: false
            }
        );
        assert_eq!(
            Caps::changed(sem, both),
            Flip {
                semantic: false,
                ask: true
            }
        );
        let lost = Caps::changed(both, none);
        assert!(lost.semantic && lost.ask && lost.any());
    }

    #[test]
    fn unavailable_messages_match_each_status() {
        for (status, semantic, ask) in TABLE {
            if semantic {
                assert_eq!(semantic_unavailable_message(status, true), None);
                assert_eq!(semantic_unavailable_message(status, false), None);
            } else {
                assert_eq!(
                    semantic_unavailable_message(status, true),
                    Some("Semantic search needs a reachable server with an embedder")
                );
                assert_eq!(
                    semantic_unavailable_message(status, false),
                    Some("Set kimun_server_url in config to use semantic search")
                );
            }
            let expected = (!ask)
                .then_some("Ask needs an LLM-configured server; this one is semantic-search only");
            assert_eq!(ask_unavailable_message(status), expected, "{status:?}");
        }
    }
}
