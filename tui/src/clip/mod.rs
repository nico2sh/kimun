//! Fetching a web page to save as a source: the fetch policy, the error type,
//! and the address filter that keeps clips off private networks.

mod address;
mod extract;
mod fetch;
#[cfg(test)]
pub(crate) mod test_server;

use std::fmt;
use std::time::Duration;

use kimun_core::error::VaultError;
use kimun_core::nfs::VaultPath;
use kimun_core::{NoteVault, SaveSourceRequest};

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
            Self::Status(code @ (401 | 403 | 429 | 451)) => write!(
                f,
                "The server answered with status {code}. Some sites refuse automated requests."
            ),
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

/// A page with this many words or fewer counts as very short. A login page or
/// a block page that a server sends with status 200 is usually this short.
pub const THIN_PAGE_WORDS: usize = 100;

/// What a clip saved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClipOutcome {
    /// The note that was written.
    pub path: VaultPath,
    /// The title the page gave, or the host of its address.
    pub title: String,
    /// How many words the saved text has.
    pub words: usize,
}

impl ClipOutcome {
    /// Whether the text is so short that the page may be a login page or a
    /// block page.
    pub fn is_thin(&self) -> bool {
        self.words <= THIN_PAGE_WORDS
    }
}

/// Why `clip_url` failed: the page could not be used, or the vault refused it.
#[derive(Debug)]
pub enum ClipFailure {
    Clip(ClipError),
    Vault(VaultError),
}

impl fmt::Display for ClipFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Clip(e) => e.fmt(f),
            Self::Vault(e) => match e.user_message() {
                Some(message) => f.write_str(&message),
                None => e.fmt(f),
            },
        }
    }
}

impl std::error::Error for ClipFailure {}

impl From<ClipError> for ClipFailure {
    fn from(e: ClipError) -> Self {
        Self::Clip(e)
    }
}

impl From<VaultError> for ClipFailure {
    fn from(e: VaultError) -> Self {
        Self::Vault(e)
    }
}

/// The time the conversion of a fetched page may take. The converter bounds
/// its own parse time, and this is the backstop for anything it does not see.
const CONVERT_TIMEOUT: Duration = Duration::from_secs(30);

/// The time added to the fetch limit before the whole fetch is given up. The
/// fetch bounds itself, but the name lookup is not timed by the HTTP client.
const FETCH_SLACK: Duration = Duration::from_secs(5);

/// Run `work` on a blocking thread and give up after `limit`. The thread is
/// not stopped: it ends when `work` returns, so a stuck call leaks a thread.
async fn blocking_within<T: Send + 'static>(
    limit: Duration,
    work: impl FnOnce() -> T + Send + 'static,
) -> Result<T, ClipError> {
    match tokio::time::timeout(limit, tokio::task::spawn_blocking(work)).await {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(_)) => Err(ClipError::Network("the clip task failed".to_string())),
        Err(_) => Err(ClipError::Timeout),
    }
}

/// Fetch `url`, turn the page into markdown, and save it as a source note.
///
/// The wiki declaration is read first, so a vault without a wiki fails before
/// any request leaves the machine.
pub async fn clip_url(
    vault: &NoteVault,
    url: &str,
    policy: &FetchPolicy,
) -> Result<ClipOutcome, ClipFailure> {
    if vault.wiki_declaration().await?.is_none() {
        return Err(VaultError::WikiNotDeclared.into());
    }
    let (fetch_url, fetch_policy) = (url.to_string(), policy.clone());
    let fetched = blocking_within(policy.total_timeout + FETCH_SLACK, move || {
        fetch(&fetch_url, &fetch_policy)
    })
    .await??;
    let content_type = fetched.content_type.clone();
    let final_url = fetched.final_url.clone();
    let article = blocking_within(CONVERT_TIMEOUT, move || {
        extract_markdown(&fetched.body, content_type.as_deref(), &final_url)
    })
    .await??;
    let title = article.title.clone().or_else(|| host_of(url));
    let words = article.markdown.split_whitespace().count();
    let path = vault
        .save_source(&SaveSourceRequest {
            url: url.to_string(),
            title: article.title,
            content_type: media_type(fetched.content_type.as_deref()),
            markdown: article.markdown,
        })
        .await?;
    Ok(ClipOutcome {
        path,
        title: title.unwrap_or_default(),
        words,
    })
}

/// The media type of a `Content-Type` header, without parameters. A missing
/// header counts as HTML, which is what `extract_markdown` sniffs for.
fn media_type(header: Option<&str>) -> String {
    header
        .and_then(|h| h.split(';').next())
        .map(|m| m.trim().to_ascii_lowercase())
        .filter(|m| !m.is_empty())
        .unwrap_or_else(|| "text/html".to_string())
}

fn host_of(url: &str) -> Option<String> {
    url::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(str::to_string))
}

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

    use super::test_server::{ok_page, reply, serve};
    use kimun_core::VaultConfig;
    use kimun_core::system::SystemPath;
    use tempfile::TempDir;

    async fn wiki_vault(declared: bool) -> (NoteVault, TempDir) {
        let dir = TempDir::new().unwrap();
        let vault = NoteVault::new(VaultConfig::new(
            SystemPath::try_absolute(dir.path()).unwrap(),
        ))
        .await
        .unwrap();
        vault.validate_and_init().await.unwrap();
        if declared {
            vault
                .wiki_init(kimun_core::WikiInitOptions::default())
                .await
                .unwrap();
        }
        (vault, dir)
    }

    fn article() -> String {
        let para = "This is a long paragraph about the topic, with enough words to look like \
                    real content, and a few commas, here and there. ";
        format!(
            "<html><head><title>My Post</title></head><body><article><h1>Big Heading</h1>\
             <p>{p}</p><p>{p}</p></article></body></html>",
            p = para.repeat(3)
        )
    }

    #[tokio::test]
    async fn clip_url_saves_a_source_from_an_html_page() {
        let (vault, _dir) = wiki_vault(true).await;
        let server = serve(false, |_| vec![ok_page(&article())]);
        let url = format!("{}/post", server.base);
        let outcome = clip_url(&vault, &url, &FetchPolicy::for_tests())
            .await
            .unwrap();
        assert_eq!(outcome.title, "My Post");
        assert!(
            outcome.path.to_bare_string().starts_with("/sources/"),
            "{}",
            outcome.path.to_bare_string()
        );
        let text = vault.get_note_text(&outcome.path).await.unwrap();
        assert!(text.contains("Big Heading"), "{text}");
        assert!(text.contains(&format!("source_url: {url}")), "{text}");
        assert!(text.contains("content_type: text/html"), "{text}");
        assert!(!text.contains("charset"), "{text}");
        let report = vault.wiki_check().await.unwrap();
        assert_eq!(report.problems(), 0, "{:?}", report.findings);
    }

    #[tokio::test]
    async fn clip_url_saves_markdown_as_is() {
        let (vault, _dir) = wiki_vault(true).await;
        let page = reply(
            "200 OK",
            &["Content-Type: text/markdown"],
            b"# Notes\n\n[[x]] <b>raw</b>\n",
        );
        let server = serve(false, |_| vec![page]);
        let url = format!("{}/notes.md", server.base);
        let outcome = clip_url(&vault, &url, &FetchPolicy::for_tests())
            .await
            .unwrap();
        let text = vault.get_note_text(&outcome.path).await.unwrap();
        assert!(text.contains("[[x]] <b>raw</b>"), "{text}");
        assert!(text.contains("content_type: text/markdown"), "{text}");
        // The first heading of the text is the title.
        assert_eq!(outcome.title, "Notes");
    }

    #[tokio::test]
    async fn clip_url_counts_the_words_of_the_saved_page() {
        let (vault, _dir) = wiki_vault(true).await;
        let server = serve(false, |_| vec![ok_page(&article())]);
        let url = format!("{}/post", server.base);
        let outcome = clip_url(&vault, &url, &FetchPolicy::for_tests())
            .await
            .unwrap();
        assert!(outcome.words > THIN_PAGE_WORDS, "{}", outcome.words);
        assert!(!outcome.is_thin());
    }

    #[tokio::test]
    async fn clip_url_flags_a_very_short_page() {
        let (vault, _dir) = wiki_vault(true).await;
        let page = reply(
            "200 OK",
            &["Content-Type: text/plain"],
            b"Log in to continue",
        );
        let server = serve(false, |_| vec![page]);
        let url = format!("{}/wall", server.base);
        let outcome = clip_url(&vault, &url, &FetchPolicy::for_tests())
            .await
            .unwrap();
        assert_eq!(outcome.words, 4);
        assert!(outcome.is_thin());
    }

    #[test]
    fn a_refused_request_hints_that_sites_block_automated_requests() {
        for code in [401, 403, 429, 451] {
            let m = ClipError::Status(code).to_string();
            assert_eq!(
                m,
                format!(
                    "The server answered with status {code}. Some sites refuse automated requests."
                )
            );
        }
        for code in [404, 410, 500, 503] {
            let m = ClipError::Status(code).to_string();
            assert_eq!(m, format!("The server answered with status {code}."));
        }
    }

    #[tokio::test]
    async fn clip_url_on_a_vault_without_a_declaration_is_wiki_not_declared_and_fetches_nothing() {
        let (vault, _dir) = wiki_vault(false).await;
        let server = serve(false, |_| vec![ok_page(&article())]);
        let url = format!("{}/post", server.base);
        let err = clip_url(&vault, &url, &FetchPolicy::for_tests())
            .await
            .unwrap_err();
        assert!(
            matches!(err, ClipFailure::Vault(VaultError::WikiNotDeclared)),
            "{err:?}"
        );
        assert_eq!(server.accepted(), 0);
    }

    #[tokio::test]
    async fn clip_url_with_an_unsupported_type_saves_nothing() {
        let (vault, _dir) = wiki_vault(true).await;
        let before = vault.wiki_check().await.unwrap().notes_checked;
        let page = reply("200 OK", &["Content-Type: image/png"], b"\x89PNG");
        let server = serve(false, |_| vec![page]);
        let url = format!("{}/cat.png", server.base);
        let err = clip_url(&vault, &url, &FetchPolicy::for_tests())
            .await
            .unwrap_err();
        assert!(
            matches!(err, ClipFailure::Clip(ClipError::UnsupportedType(_))),
            "{err:?}"
        );
        assert_eq!(vault.wiki_check().await.unwrap().notes_checked, before);
    }

    #[tokio::test]
    async fn blocking_work_that_runs_too_long_is_a_timeout() {
        let slow = blocking_within(Duration::from_millis(50), || {
            std::thread::sleep(Duration::from_millis(500));
            1
        })
        .await;
        assert_eq!(slow, Err(ClipError::Timeout));
        let fast = blocking_within(Duration::from_secs(5), || 7).await;
        assert_eq!(fast, Ok(7));
    }
}
