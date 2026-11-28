//! The network fetch of one web page, under a [`FetchPolicy`].

use std::error::Error as _;
use std::fmt;
use std::io::{self, Read};
use std::net::{SocketAddr, ToSocketAddrs};
use std::time::Instant;

use url::Url;

use super::{ClipError, FetchPolicy, check_url, is_public_address};

const ACCEPT: &str = "text/html, text/markdown, text/plain;q=0.9";

/// A fetched page, before any extraction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fetched {
    /// The address of the last hop, after redirects.
    pub final_url: String,
    /// The `Content-Type` header as the server sent it, if any.
    pub content_type: Option<String>,
    /// The body, decoded as UTF-8.
    pub body: String,
}

/// Fetches `url` under `policy`. It blocks, so run it on a blocking thread.
///
/// Redirects are followed one at a time, and every hop is checked against
/// the policy before any connection. The time limit covers all the hops
/// together. The body is decoded as UTF-8. Pages in other encodings can show
/// wrong characters.
pub fn fetch(url: &str, policy: &FetchPolicy) -> Result<Fetched, ClipError> {
    let mut current = Url::parse(url).map_err(|e| ClipError::BadUrl(e.to_string()))?;
    let agent = ureq::AgentBuilder::new()
        .redirects(0)
        .timeout_connect(policy.connect_timeout)
        .timeout(policy.total_timeout)
        .user_agent(concat!("kimun-clip/", env!("CARGO_PKG_VERSION")))
        .resolver(PolicyResolver {
            allow_private: policy.allow_private,
        })
        // Never use a proxy from the environment. With a proxy, the resolver
        // would check the proxy's address and not the target's. This keeps
        // the rule even if a dependency turns on ureq's `proxy-from-env`.
        .try_proxy_from_env(false)
        .build();
    // The limit is approximate. ureq does not bound the DNS lookup, and a
    // connect that starts just before the deadline can run past it by up to
    // the connect timeout.
    let deadline = Instant::now() + policy.total_timeout;
    let mut hops: u8 = 0;
    loop {
        check_url(&current, policy)?;
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .filter(|d| !d.is_zero())
            .ok_or(ClipError::Timeout)?;
        let response = match agent
            .request_url("GET", &current)
            .set("Accept", ACCEPT)
            .timeout(remaining)
            .call()
        {
            Ok(response) => response,
            Err(ureq::Error::Status(code, _)) => return Err(ClipError::Status(code)),
            Err(ureq::Error::Transport(t)) => return Err(transport_error(&t)),
        };
        if matches!(response.status(), 301 | 302 | 303 | 307 | 308) {
            let Some(location) = response.header("Location") else {
                return Err(ClipError::Network(
                    "the server sent a redirect without a target".to_string(),
                ));
            };
            let next = current
                .join(location)
                .map_err(|e| ClipError::BadUrl(e.to_string()))?;
            if hops >= policy.max_redirects {
                return Err(ClipError::TooManyRedirects);
            }
            hops += 1;
            current = next;
            continue;
        }
        let content_type = response.header("Content-Type").map(str::to_string);
        let body = read_capped(response.into_reader(), policy.max_bytes)?;
        return Ok(Fetched {
            final_url: current.to_string(),
            content_type,
            body,
        });
    }
}

/// Reads at most `max_bytes` from `reader`, then decodes it as UTF-8.
fn read_capped(reader: impl Read, max_bytes: usize) -> Result<String, ClipError> {
    let mut bytes = Vec::new();
    reader
        .take(max_bytes as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| {
            if is_timeout(&e) {
                ClipError::Timeout
            } else {
                ClipError::Network("the connection failed while reading the page".to_string())
            }
        })?;
    if bytes.len() > max_bytes {
        return Err(ClipError::TooLarge { limit: max_bytes });
    }
    if bytes.is_empty() {
        return Err(ClipError::NoContent);
    }
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

fn is_timeout(e: &io::Error) -> bool {
    // A socket read timeout is `WouldBlock` on Unix and `TimedOut` on Windows.
    matches!(
        e.kind(),
        io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
    )
}

/// Turns a transport error into a clip error. The message never holds the
/// URL. The resolver's refusal is found by its marker type, not by its text.
fn transport_error(t: &ureq::Transport) -> ClipError {
    let mut source = t.source();
    while let Some(err) = source {
        if let Some(io_err) = err.downcast_ref::<io::Error>() {
            if let Some(refused) = io_err
                .get_ref()
                .and_then(|inner| inner.downcast_ref::<NoAllowedAddress>())
            {
                return ClipError::PrivateAddress(refused.host.clone());
            }
            if is_timeout(io_err) {
                return ClipError::Timeout;
            }
        }
        source = err.source();
    }
    let why = match t.kind() {
        ureq::ErrorKind::Dns => "the host name could not be found",
        ureq::ErrorKind::ConnectionFailed => "the server could not be reached",
        ureq::ErrorKind::BadStatus | ureq::ErrorKind::BadHeader => {
            "the server sent a response that could not be read"
        }
        _ => "the connection failed",
    };
    ClipError::Network(why.to_string())
}

/// The resolver's error when a host has no address the policy allows.
#[derive(Debug)]
struct NoAllowedAddress {
    host: String,
}

impl fmt::Display for NoAllowedAddress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "the host {} has no public address", self.host)
    }
}

impl std::error::Error for NoAllowedAddress {}

/// Keeps the addresses the policy allows: all of them when `allow_private`
/// is set, else only the public ones.
fn keep_allowed(addrs: Vec<SocketAddr>, allow_private: bool) -> Vec<SocketAddr> {
    if allow_private {
        return addrs;
    }
    addrs
        .into_iter()
        .filter(|a| is_public_address(&a.ip()))
        .collect()
}

/// Resolves host names and drops the addresses the policy refuses, so a
/// public name that points at a private address cannot be reached.
struct PolicyResolver {
    allow_private: bool,
}

impl ureq::Resolver for PolicyResolver {
    fn resolve(&self, netloc: &str) -> io::Result<Vec<SocketAddr>> {
        let addrs = keep_allowed(netloc.to_socket_addrs()?.collect(), self.allow_private);
        if addrs.is_empty() {
            // The netloc is `host:port`. Keep only the host for the message.
            let host = netloc
                .rsplit_once(':')
                .map_or(netloc, |(host, _)| host)
                .trim_start_matches('[')
                .trim_end_matches(']')
                .to_string();
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                NoAllowedAddress { host },
            ));
        }
        Ok(addrs)
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;
    use std::net::TcpListener;
    use std::time::{Duration, Instant};

    use ureq::Resolver;

    use super::super::test_server::{ok_page, reply, serve};
    use super::*;

    #[test]
    fn fetch_returns_body_content_type_and_final_url() {
        let s = serve(false, |_| vec![ok_page("<p>hello</p>")]);
        let url = format!("{}/page", s.base);
        let got = fetch(&url, &FetchPolicy::for_tests()).unwrap();
        assert_eq!(got.body, "<p>hello</p>");
        assert_eq!(
            got.content_type.as_deref(),
            Some("text/html; charset=utf-8")
        );
        assert_eq!(got.final_url, url);
    }

    #[test]
    fn fetch_follows_a_redirect_to_the_same_server() {
        // A relative Location.
        let s = serve(false, |_| {
            vec![
                reply("302 Found", &["Location: /next"], b""),
                ok_page("moved here"),
            ]
        });
        let got = fetch(&format!("{}/start", s.base), &FetchPolicy::for_tests()).unwrap();
        assert_eq!(got.body, "moved here");
        assert_eq!(got.final_url, format!("{}/next", s.base));
        assert_eq!(s.accepted(), 2);

        // An absolute Location.
        let s = serve(false, |base| {
            vec![
                reply(
                    "301 Moved Permanently",
                    &[&format!("Location: {base}/abs")],
                    b"",
                ),
                ok_page("absolute"),
            ]
        });
        let got = fetch(&format!("{}/start", s.base), &FetchPolicy::for_tests()).unwrap();
        assert_eq!(got.body, "absolute");
        assert_eq!(got.final_url, format!("{}/abs", s.base));
    }

    #[test]
    fn fetch_applies_check_url_to_a_redirect_hop() {
        // The test policy allows http, but ftp is never allowed.
        let s = serve(false, |_| {
            vec![reply("302 Found", &["Location: ftp://example.com/x"], b"")]
        });
        let err = fetch(&format!("{}/", s.base), &FetchPolicy::for_tests()).unwrap_err();
        assert!(matches!(err, ClipError::NotHttps(_)), "{err:?}");
        assert_eq!(s.accepted(), 1);
    }

    #[test]
    fn strict_policy_refuses_a_loopback_literal_and_makes_no_connection() {
        let s = serve(false, |_| vec![ok_page("never")]);
        for url in [
            format!("https://127.0.0.1:{}/", s.port),
            format!("https://[::1]:{}/", s.port),
        ] {
            let err = fetch(&url, &FetchPolicy::strict()).unwrap_err();
            assert!(
                matches!(err, ClipError::PrivateAddress(_)),
                "{url}: {err:?}"
            );
        }
        std::thread::sleep(Duration::from_millis(100));
        assert_eq!(s.accepted(), 0);
    }

    #[test]
    fn strict_policy_refuses_localhost_by_name_and_makes_no_connection() {
        // The name passes `check_url`, so the refusal comes from the resolver
        // and travels back through ureq's error chain.
        let s = serve(false, |_| vec![ok_page("never")]);
        let url = format!("https://localhost:{}/", s.port);
        let err = fetch(&url, &FetchPolicy::strict()).unwrap_err();
        assert_eq!(err, ClipError::PrivateAddress("localhost".to_string()));
        std::thread::sleep(Duration::from_millis(100));
        assert_eq!(s.accepted(), 0);
    }

    #[test]
    fn resolver_refuses_localhost_under_a_strict_policy() {
        // The resolver is tested directly for the private-redirect case: a
        // plain-http test server cannot be a first hop under a policy that
        // refuses private addresses.
        let s = serve(false, |_| vec![ok_page("never")]);
        let netloc = format!("localhost:{}", s.port);
        let strict = PolicyResolver {
            allow_private: false,
        };
        let err = strict.resolve(&netloc).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::PermissionDenied);
        let open = PolicyResolver {
            allow_private: true,
        };
        assert!(!open.resolve(&netloc).unwrap().is_empty());
        std::thread::sleep(Duration::from_millis(100));
        assert_eq!(s.accepted(), 0);
    }

    #[test]
    fn resolver_keeps_only_public_addresses() {
        let private: SocketAddr = "10.0.0.1:443".parse().unwrap();
        let loopback: SocketAddr = "[::1]:443".parse().unwrap();
        let public: SocketAddr = "93.184.216.34:443".parse().unwrap();
        assert_eq!(keep_allowed(vec![private, public], false), vec![public]);
        assert!(keep_allowed(vec![private, loopback], false).is_empty());
        assert_eq!(
            keep_allowed(vec![private, public], true),
            vec![private, public]
        );
    }

    #[test]
    fn fetch_stops_at_the_byte_cap() {
        let big = vec![b'a'; 2 * 1024 * 1024];
        let s = serve(false, move |_| {
            vec![reply("200 OK", &["Content-Type: text/plain"], &big)]
        });
        let mut policy = FetchPolicy::for_tests();
        policy.max_bytes = 1024;
        let start = Instant::now();
        let err = fetch(&format!("{}/", s.base), &policy).unwrap_err();
        assert_eq!(err, ClipError::TooLarge { limit: 1024 });
        assert!(start.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn a_body_of_exactly_the_cap_is_allowed() {
        let body = vec![b'a'; 1024];
        let s = serve(false, move |_| {
            vec![reply("200 OK", &["Content-Type: text/plain"], &body)]
        });
        let mut policy = FetchPolicy::for_tests();
        policy.max_bytes = 1024;
        let got = fetch(&format!("{}/", s.base), &policy).unwrap();
        assert_eq!(got.body.len(), 1024);
    }

    #[test]
    fn a_gzip_response_is_still_bounded_by_the_cap() {
        use flate2::Compression;
        use flate2::write::GzEncoder;
        let mut enc = GzEncoder::new(Vec::new(), Compression::best());
        enc.write_all(&vec![b'a'; 64 * 1024]).unwrap();
        let gz = enc.finish().unwrap();
        // The wire body is under the cap. Only the expanded body is over it.
        assert!(gz.len() < 1024, "{}", gz.len());
        let s = serve(false, move |_| {
            vec![reply(
                "200 OK",
                &["Content-Type: text/plain", "Content-Encoding: gzip"],
                &gz,
            )]
        });
        let mut policy = FetchPolicy::for_tests();
        policy.max_bytes = 1024;
        let err = fetch(&format!("{}/", s.base), &policy).unwrap_err();
        assert_eq!(err, ClipError::TooLarge { limit: 1024 });
    }

    #[test]
    fn fetch_stops_at_the_time_limit() {
        // The headers promise more body than the server sends, then it stalls.
        let s = serve(true, |_| {
            vec![
                b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 1000\r\n\r\nstart"
                    .to_vec(),
            ]
        });
        let mut policy = FetchPolicy::for_tests();
        policy.total_timeout = Duration::from_secs(1);
        let start = Instant::now();
        let err = fetch(&format!("{}/", s.base), &policy).unwrap_err();
        assert_eq!(err, ClipError::Timeout);
        assert!(start.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn too_many_redirects_is_an_error() {
        let s = serve(false, |_| {
            vec![reply("302 Found", &["Location: /again"], b"")]
        });
        let mut policy = FetchPolicy::for_tests();
        policy.max_redirects = 2;
        let err = fetch(&format!("{}/", s.base), &policy).unwrap_err();
        assert_eq!(err, ClipError::TooManyRedirects);
        // The first request and the two redirects it may follow.
        assert_eq!(s.accepted(), 3);
    }

    #[test]
    fn a_redirect_without_location_is_a_network_error() {
        let s = serve(false, |_| vec![reply("302 Found", &[], b"")]);
        let err = fetch(&format!("{}/", s.base), &FetchPolicy::for_tests()).unwrap_err();
        assert!(matches!(err, ClipError::Network(_)), "{err:?}");
    }

    #[test]
    fn status_404_gives_status_with_the_code() {
        let s = serve(false, |_| vec![reply("404 Not Found", &[], b"gone")]);
        let err = fetch(&format!("{}/", s.base), &FetchPolicy::for_tests()).unwrap_err();
        assert_eq!(err, ClipError::Status(404));
    }

    #[test]
    fn status_500_gives_status() {
        let s = serve(false, |_| {
            vec![reply("500 Internal Server Error", &[], b"oops")]
        });
        let err = fetch(&format!("{}/", s.base), &FetchPolicy::for_tests()).unwrap_err();
        assert_eq!(err, ClipError::Status(500));
    }

    #[test]
    fn empty_body_is_no_content() {
        let s = serve(false, |_| {
            vec![reply("200 OK", &["Content-Type: text/html"], b"")]
        });
        let err = fetch(&format!("{}/", s.base), &FetchPolicy::for_tests()).unwrap_err();
        assert_eq!(err, ClipError::NoContent);
    }

    #[test]
    fn a_bad_url_is_bad_url_not_a_panic() {
        let policy = FetchPolicy::for_tests();
        for url in ["not a url", "https://", ""] {
            let err = fetch(url, &policy).unwrap_err();
            assert!(matches!(err, ClipError::BadUrl(_)), "{url:?}: {err:?}");
        }
        // This one parses, so the scheme rule refuses it.
        let err = fetch("ftp://x", &policy).unwrap_err();
        assert!(matches!(err, ClipError::NotHttps(_)), "{err:?}");
    }

    #[test]
    fn a_refused_connection_is_a_network_error_without_the_url() {
        // Bind and drop, so nothing listens on the port.
        let port = TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let err = fetch(
            &format!("http://127.0.0.1:{port}/secret-path"),
            &FetchPolicy::for_tests(),
        )
        .unwrap_err();
        assert!(matches!(err, ClipError::Network(_)), "{err:?}");
        assert!(!err.to_string().contains("secret-path"), "{err}");
    }
}
