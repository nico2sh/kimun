// tui/src/cli/commands/mod.rs
pub mod journal;
pub mod labels;
pub mod mcp;
pub mod note_ops;
pub mod notes;
pub mod search;
pub mod update;
pub mod workspace;

// Re-export for convenience
pub use journal::JournalArgs;
pub use note_ops::NoteSubcommand;
pub use workspace::WorkspaceSubcommand;
