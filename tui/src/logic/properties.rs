//! Property write errors, classified from core's `VaultError`.

#[derive(Debug, Clone)]
pub enum PropertyWriteError {
    /// Core refused the value (`VaultError::InvalidProperty`).
    Invalid(String),
    /// The note's frontmatter block does not parse; the dialog turns read-only.
    Malformed(String),
    /// Anything else (IO, lock timeout).
    Other(String),
}

impl PropertyWriteError {
    pub(crate) fn from_vault(e: kimun_core::error::VaultError) -> Self {
        use kimun_core::error::{FSError, VaultError};
        match &e {
            VaultError::InvalidProperty { message, .. } => Self::Invalid(message.clone()),
            VaultError::FSError(FSError::InvalidFrontmatter { .. }) => {
                Self::Malformed(e.to_string())
            }
            _ => Self::Other(e.to_string()),
        }
    }

    pub fn message(&self) -> &str {
        match self {
            Self::Invalid(m) | Self::Malformed(m) | Self::Other(m) => m,
        }
    }
}
