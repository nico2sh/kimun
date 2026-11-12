//! Replacing the desktop app's own `Kimun.app` bundle on macOS (ADR-0056).
//!
//! Frontend-specific host work that does not touch vault content, so it lives
//! here and not in core. The modules compile under `cfg(test)` on every
//! platform: they hold logic over a [`signature::CommandRunner`] seam, and the
//! tests run that logic against canned tool output.
//!
//! - [`signature`]: whether a staged app may replace the running one
//!   (valid Developer ID signature, same Team ID).
//! - [`swap`]: mount the release image, stage and check the new app, swap it
//!   in, and undo on failure.

#[cfg(any(target_os = "macos", test))]
pub(crate) mod signature;

#[cfg(any(target_os = "macos", test))]
pub(crate) mod swap;
