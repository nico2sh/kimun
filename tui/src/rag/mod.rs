//! All wiring for the optional RAG server lives in this module — config
//! reading, client construction ([`client`]) and the background sync loop
//! ([`sync`]). Everything talks to the server through `kimun_server_client`;
//! the rest of the TUI only consumes these helpers and renders status.

mod client;
mod sync;

pub use client::{rag_client, rag_configured};
pub use sync::spawn_rag_sync;

/// RAG connection status surfaced in the footer. `Disabled` (no server
/// configured) is never sent — the loop simply doesn't start — so the footer
/// shows nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RagStatus {
    Disabled,
    Offline,
    /// Reachable but the server rejects our credentials: it requires a bearer
    /// token and none is configured, or API calls come back 401/403 (wrong
    /// token). Distinct from `Offline` so the user learns it's a token
    /// problem, not an unreachable server.
    Unauthorized,
    /// Reachable but the server has no embedder configured (adr/0024): nothing
    /// works server-side, so the loop skips pushing and reconciling entirely —
    /// every call would 503 — and just reports the state.
    NotConfigured,
    /// Reachable, a sync pass in flight. `llm_available` carries whether the
    /// server has an LLM configured (question-answering possible), so Ask stays
    /// gated consistently while syncing.
    Syncing {
        llm_available: bool,
    },
    /// Reachable and idle. `llm_available` = the server has an LLM (Q&A on);
    /// `false` = semantic-only (search only).
    Online {
        llm_available: bool,
    },
}

impl RagStatus {
    /// Short footer label, or `None` when nothing should show.
    pub fn label(self) -> Option<&'static str> {
        match self {
            RagStatus::Disabled => None,
            RagStatus::Offline => Some("rag: offline"),
            RagStatus::Unauthorized => Some("rag: unauthorized"),
            RagStatus::NotConfigured => Some("rag: not configured"),
            RagStatus::Syncing { .. } => Some("rag: syncing"),
            RagStatus::Online { .. } => Some("rag: online"),
        }
    }

    /// Whether question-answering (Ask) is available right now: the server is
    /// reachable AND has an LLM configured. `false` when offline, disabled, or
    /// connected to a semantic-only server — the ASK rail entry is hidden in
    /// those cases (adr/0022).
    pub fn llm_available(self) -> bool {
        matches!(
            self,
            RagStatus::Online {
                llm_available: true
            } | RagStatus::Syncing {
                llm_available: true
            }
        )
    }

    /// Whether semantic search is usable right now: the server is reachable AND
    /// has an embedder — i.e. `Online`/`Syncing`, regardless of `llm_available`
    /// (a semantic-only server still searches). `false` for `Offline`,
    /// `Unauthorized`, `NotConfigured` and `Disabled`. The SEM rail entry is
    /// driven by this, mirroring how ASK is driven by `llm_available` — a
    /// configured-but-unreachable server hides SEM just as it hides ASK.
    pub fn search_available(self) -> bool {
        matches!(self, RagStatus::Online { .. } | RagStatus::Syncing { .. })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn not_configured_status_labels_and_gates() {
        assert_eq!(
            RagStatus::NotConfigured.label(),
            Some("rag: not configured")
        );
        assert!(!RagStatus::NotConfigured.llm_available());
    }

    #[test]
    fn unauthorized_status_labels_and_gates() {
        assert_eq!(RagStatus::Unauthorized.label(), Some("rag: unauthorized"));
        assert!(!RagStatus::Unauthorized.llm_available());
    }

    #[test]
    fn search_available_tracks_reachable_with_embedder() {
        // Online/Syncing → searchable, whether or not an LLM is configured
        // (a semantic-only server still searches).
        assert!(
            RagStatus::Online {
                llm_available: false
            }
            .search_available()
        );
        assert!(
            RagStatus::Online {
                llm_available: true
            }
            .search_available()
        );
        assert!(
            RagStatus::Syncing {
                llm_available: false
            }
            .search_available()
        );
        assert!(
            RagStatus::Syncing {
                llm_available: true
            }
            .search_available()
        );
        // Not reachable / no embedder → not searchable.
        assert!(!RagStatus::Offline.search_available());
        assert!(!RagStatus::Unauthorized.search_available());
        assert!(!RagStatus::NotConfigured.search_available());
        assert!(!RagStatus::Disabled.search_available());
    }

    #[test]
    fn semantic_only_server_searches_but_does_not_answer() {
        let semantic_only = RagStatus::Online {
            llm_available: false,
        };
        assert!(semantic_only.search_available(), "SEM must show");
        assert!(!semantic_only.llm_available(), "ASK must stay hidden");
    }
}
