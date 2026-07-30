//! A text editing model for terminal editors.
//!
//! `ropetext` owns text, the cursor, the selection, the edit history, motions,
//! and soft-wrap layout. It owns nothing else, and the omissions are deliberate:
//!
//! - **No renderer.** Layout returns visual lines and cell coordinates in this
//!   crate's own plain geometry types. Painting belongs to the caller.
//! - **No input handling.** Keys never reach this crate. It exposes operations;
//!   deciding which key performs which operation is the caller's business.
//! - **No syntax, no markdown.** What a construct *means* never enters. Where a
//!   syntax layer must be heard from — wrapping has to know which characters are
//!   hidden and how far a row is inset — it is heard as data passed in, not as a
//!   trait this crate calls back into.
//! - **No search, no clipboard, no registers.** The crate hands out the text in
//!   a range; what a caller matches against it or stores it in is not its
//!   concern.
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
mod position;
mod text;

pub use buffer::{EditBuffer, Snapshot, Txn};
pub use change::{Change, Edit};
pub use position::{Column, Position, Revision, Span};
pub use text::Text;
