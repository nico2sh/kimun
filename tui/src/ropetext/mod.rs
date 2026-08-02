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
//! The omissions above are not incidental — they are the reason this module was
//! written. It replaced a third-party textarea whose contract kept leaking
//! editor policy (its own undo keybindings, its own idea of what a line is) into
//! callers, and it earns its place only by staying narrower than what it
//! replaced. The name carries no `kimun` prefix for the same reason: anything
//! kimün-flavoured that tries to move in should look wrong on sight.
//!
//! This was a separate workspace crate, which made that a compiler rule — a
//! crate cannot say `crate::settings::Theme`. It was folded in because
//! kimun-notes is published to crates.io, and crates.io refuses to publish a
//! crate that path-depends on an unpublished one: a workspace member the
//! published crate depends on is either published forever or not a crate. This
//! had no API worth stabilising, so it stopped being a crate.
//!
//! The rule outlived the crate. Nothing here may name `crate::` outside
//! `crate::ropetext::`, checked in `.github/workflows/check.yml` now that the
//! crate graph no longer can. That is the one property extraction depends on:
//! going back out to a crate — for a reusable editor widget, say — is a `git mv`
//! and a manifest, with nothing to untangle first.
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
