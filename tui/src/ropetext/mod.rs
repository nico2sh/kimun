//! A text editing model for terminal editors.
//!
//! `ropetext` owns text, the cursor, the selection, the edit history, motions,
//! and soft-wrap layout. It owns nothing else, and the omissions are deliberate:
//!
//! - **No renderer.** Layout returns visual lines and cell coordinates in this
//!   module's own plain geometry types. Painting belongs to the caller.
//! - **No input handling.** Keys never reach this module. It exposes operations;
//!   deciding which key performs which operation is the caller's business.
//! - **No syntax, no markdown.** What a construct *means* never enters. Where a
//!   syntax layer must be heard from — wrapping has to know which characters are
//!   hidden and how far a row is inset — it is heard as data passed in, not as a
//!   trait this module calls back into.
//! - **No search, no clipboard, no registers.** It hands out the text in a
//!   range; what a caller matches against it or stores it in is not its concern.
//!
//! # It knows nothing of kimün
//!
//! This was a workspace crate until ADR-0042 folded it in, and it is written to
//! go back out to one on demand — a reusable editor widget is the likely reason.
//! Nothing here may name `crate::` outside `crate::ropetext::`, which is the one
//! property extraction depends on; `.github/workflows/check.yml` enforces it now
//! that the crate graph no longer can. See adr/0039 for why the engine exists at
//! all, adr/0041 for why markdown stays outside it, and adr/0042 for the move.
//!
//! # Positions are checked, never approximated
//!
//! A [`Position`] can only be built by asking a [`Text`] for one, and it carries
//! the [`Revision`] it was built against. A position the text cannot address has
//! no value at all — the constructor returns `None` rather than clamping to
//! something nearby — and a position used against a later revision is rejected
//! rather than silently read as some other place in the buffer. The one call
//! that approximates, [`Text::position_at_byte_snapped`], is named for it.

mod buffer;
mod change;
mod history;
mod layout;
pub mod motion;
mod position;
mod text;
mod width;

pub use buffer::{EditBuffer, Snapshot, Txn};
pub use change::{Change, Edit};
pub use layout::{Cell, Layout, RowHints, Viewport, VisualLine};
pub use position::{Column, Position, Revision, Span};
pub use text::Text;
pub use width::Metrics;
