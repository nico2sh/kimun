// tui/src/cli/commands/mod.rs
pub mod search;
pub mod notes;
pub mod workspace;
pub mod note_ops;
pub mod journal;
pub mod mcp;

// Re-export for convenience
pub use workspace::WorkspaceSubcommand;
pub use note_ops::NoteSubcommand;
pub use journal::JournalArgs;
