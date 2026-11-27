//! Fetching a web page to save as a source: the fetch policy, the error type,
//! and the address filter that keeps clips off private networks.

mod address;
mod extract;
mod fetch;

use std::fmt;
use std::time::Duration;

pub(crate) use address::check_url;
pub use address::is_public_address;
pub use extract::{Article, extract_markdown};
pub use fetch::{Fetched, fetch};

/// The limits and switches for one fetch.
#[derive(Debug, Clone)]
pub struct FetchPolicy {
    /// Allow `http` as well as `https`.
    pub(crate) allow_http: bool,
    /// Allow connections to private, loopback, and link-local addresses.
    pub(crate) allow_private: bool,
    /// The most bytes a response body may have.
    pub(crate) max_bytes: usize,
    /// The time allowed to open a connection.
    pub(crate) connect_timeout: Duration,
    /// The time allowed for the whole fetch.
    pub(crate) total_timeout: Duration,
    /// The most redirects the fetch follows.
    pub(crate) max_redirects: u8,
}

impl FetchPolicy {
    /// The policy for real use: `https` only, public addresses only.
    pub fn strict() -> Self {
        Self {
            allow_http: false,
            allow_private: false,
            max_bytes: 20 * 1024 * 1024,
            connect_timeout: Duration::from_secs(15),
            total_timeout: Duration::from_secs(60),
            max_redirects: 5,
        }
    }

    /// A policy for tests that talk to a local server.
    #[cfg(test)]
    pub(crate) fn for_tests() -> Self {
        Self {
            allow_http: true,
            allow_private: true,
            ..Self::strict()
        }
    }
}

/// Why a clip failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClipError {
    BadUrl(String),
    NotHttps(String),
    PrivateAddress(String),
    TooLarge { limit: usize },
    Timeout,
    TooManyRedirects,
    Status(u16),
    UnsupportedType(String),
    NoContent,
    Network(String),
}

impl fmt::Display for ClipError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadUrl(why) => write!(f, "This is not a usable web address: {why}."),
            Self::NotHttps(url) => write!(f, "Only https addresses can be clipped: {url}."),
            Self::PrivateAddress(host) => write!(
                f,
                "The host {host} is on a private or local network and is not allowed."
            ),
            Self::TooLarge { limit } => write!(
                f,
                "The page is larger than the limit of {} MB.",
                limit / (1024 * 1024)
            ),
            Self::Timeout => write!(f, "The page took too long to load."),
            Self::TooManyRedirects => write!(f, "The address redirected too many times."),
            Self::Status(code) => write!(f, "The server answered with status {code}."),
            Self::UnsupportedType(kind) => {
                write!(f, "This kind of content cannot be clipped: {kind}.")
            }
            Self::NoContent => write!(f, "The page has no readable content."),
            Self::Network(why) => write!(f, "The network request failed: {why}."),
        }
    }
}

impl std::error::Error for ClipError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strict_policy_values() {
        let p = FetchPolicy::strict();
        assert!(!p.allow_http && !p.allow_private);
        assert_eq!(p.max_bytes, 20 * 1024 * 1024);
        assert_eq!(p.connect_timeout, Duration::from_secs(15));
        assert_eq!(p.total_timeout, Duration::from_secs(60));
        assert_eq!(p.max_redirects, 5);
        let t = FetchPolicy::for_tests();
        assert!(t.allow_http && t.allow_private);
        assert_eq!(t.max_bytes, p.max_bytes);
    }

    #[test]
    fn error_messages_are_plain() {
        let all = [
            ClipError::BadUrl("x".into()),
            ClipError::NotHttps("http://a".into()),
            ClipError::PrivateAddress("10.0.0.1".into()),
            ClipError::TooLarge {
                limit: 20 * 1024 * 1024,
            },
            ClipError::Timeout,
            ClipError::TooManyRedirects,
            ClipError::Status(404),
            ClipError::UnsupportedType("image/png".into()),
            ClipError::NoContent,
            ClipError::Network("reset".into()),
        ];
        for e in all {
            let m = e.to_string();
            assert!(!m.is_empty());
            assert!(!m.contains('\u{2014}'), "{m}");
        }
        assert!(
            ClipError::PrivateAddress("10.0.0.1".into())
                .to_string()
                .contains("10.0.0.1")
        );
    }
}
