//! Remote images: the fetcher and the shared store (CONTEXT.md, "Remote
//! image").
//!
//! The store keeps the state of each remote address for the session. It
//! starts a fetch only when a reader asks (`load`) or when the host is on the
//! allowed list and the embed is on screen (`auto_load`). A fetch runs on
//! tokio's blocking pool, never on the UI thread. The bytes go to core's
//! [`ImageCache`], which writes the file. The GUI never writes a file. A saved
//! copy is an ordinary image file that [`crate::inline_image::ImageStore`]
//! reads.
//!
//! One HTTP client serves every fetch: the store builds one `ureq::Agent`
//! when it is built, and the real fetcher reuses it for each fetch and each
//! redirect hop. No other module names `ureq`.

use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::io::Read;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use gpui_kit::{App, Global};
use kimun_core::ImageCache;
use kimun_core::image_extension;
use kimun_core::note::{AllowedHosts, host_matches, host_of, normalize_image_url};

/// The largest image we download: 20 MB.
pub(crate) const MAX_BYTES: u64 = 20 * 1024 * 1024;
/// The most fetches that run at one time. Others wait in a queue.
const MAX_RUNNING: usize = 4;
/// The most redirects a fetch follows. The next one fails.
const MAX_REDIRECTS: usize = 3;
/// The wait for the connection and for each piece of data.
const WAIT_LIMIT: Duration = Duration::from_secs(15);
/// The limit for a whole fetch.
const TOTAL_LIMIT: Duration = Duration::from_secs(60);
/// The `User-Agent` header. It names no user, system, or version.
const USER_AGENT: &str = "Kimun";

/// Why a remote image did not load.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum FetchFailure {
    /// The server did not answer in time.
    TimedOut,
    /// The server cannot be reached.
    Offline,
    /// The server answered with an error status.
    Status(u16),
    /// The image is over the size limit.
    TooLarge,
    /// The bytes are not an image.
    NotAnImage,
    /// An image format we do not draw, for example SVG.
    Unsupported,
    /// More redirects than the limit.
    TooManyRedirects,
    /// Any other reason, as text.
    Other(String),
}

impl FetchFailure {
    /// The one-line reason that stands in for the image.
    pub(crate) fn note(&self) -> String {
        let reason = match self {
            FetchFailure::TimedOut => "timed out".to_string(),
            FetchFailure::Offline => "server not reachable".to_string(),
            FetchFailure::Status(code) => format!("HTTP {code}"),
            FetchFailure::TooLarge => "image over 20 MB".to_string(),
            FetchFailure::NotAnImage => "not an image".to_string(),
            FetchFailure::Unsupported => "format not supported".to_string(),
            FetchFailure::TooManyRedirects => "too many redirects".to_string(),
            FetchFailure::Other(text) => text.clone(),
        };
        format!("Load failed: {reason}")
    }

    /// Whether "Retry" can help. An address that is not a web address
    /// never loads, so it offers no retry.
    pub(crate) fn can_retry(&self) -> bool {
        *self != not_a_web_address()
    }
}

/// The answer to one request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Hop {
    /// The body of a success answer.
    Body(Vec<u8>),
    /// A redirect, with the `Location` as the server wrote it.
    Redirect(String),
    Failed(FetchFailure),
}

/// The seam that makes one request. The real one uses the network. Tests
/// install a fake.
pub(crate) trait Fetcher: Send + Sync {
    /// One request for `url`. It never follows a redirect.
    fn get(&self, url: &str) -> Hop;

    /// One request for `url` that ends by `deadline`, the end of the whole
    /// fetch. A fake can keep the default, which ignores the deadline.
    fn get_until(&self, url: &str, deadline: Instant) -> Hop {
        let _ = deadline;
        self.get(url)
    }
}

/// What a surface draws for a remote address now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum RemoteState {
    /// No saved copy and no fetch. The placeholder names the host. An
    /// allowed host is also in this state until `auto_load` starts it.
    Blocked { host: String },
    /// The fetch stopped at a redirect to a host that is not allowed.
    RedirectBlocked { host: String },
    /// The address uses `http`. It never loads.
    Insecure,
    /// A fetch is under way or waits in the queue.
    Loading,
    /// A copy is in the cache, at this path.
    Saved(PathBuf),
    /// The fetch failed in this session.
    Failed(FetchFailure),
}

/// The real fetcher: one `ureq` agent with the limits of a remote image.
///
/// The agent follows no redirect (`redirects(0)`), so the store checks each
/// hop. It has no cookie store (the `cookies` feature is off), sends no
/// `Referer` (`ureq` never adds one), and sends the generic `User-Agent`.
///
/// The agent has no overall `timeout`: in `ureq` 2 that replaces the wait for
/// each read. The wait limit applies to the connection and to each read. The
/// total limit is a deadline that the caller passes for the whole fetch.
struct RealFetcher {
    agent: ureq::Agent,
    /// The body limit in bytes. Tests set a small one.
    limit: u64,
    /// The wait for the connection and for each read.
    wait: Duration,
    /// The total limit of a call to [`Fetcher::get`].
    total: Duration,
}

impl RealFetcher {
    fn new() -> Self {
        Self::with_limits(MAX_BYTES, WAIT_LIMIT, TOTAL_LIMIT)
    }

    /// A fetcher with these limits. Tests set small ones.
    fn with_limits(limit: u64, wait: Duration, total: Duration) -> Self {
        let agent = ureq::AgentBuilder::new()
            .timeout_connect(wait)
            .timeout_read(wait)
            .timeout_write(wait)
            .redirects(0)
            .user_agent(USER_AGENT)
            .build();
        Self {
            agent,
            limit,
            wait,
            total,
        }
    }

    /// The hop for an answer that is not an error status.
    fn read(&self, response: ureq::Response, deadline: Instant) -> Hop {
        let status = response.status();
        if (300..400).contains(&status) {
            let location = response.header("location").map(str::to_string);
            // Read the short body to its end, so the connection goes back to
            // the agent's pool.
            let _ = std::io::copy(
                &mut response.into_reader().take(64 * 1024),
                &mut std::io::sink(),
            );
            return match location {
                Some(location) => Hop::Redirect(location),
                None => Hop::Failed(FetchFailure::Status(status)),
            };
        }
        if !(200..300).contains(&status) {
            return Hop::Failed(FetchFailure::Status(status));
        }
        let declared = response
            .header("content-length")
            .and_then(|v| v.trim().parse::<u64>().ok());
        if declared.is_some_and(|len| len > self.limit) {
            return Hop::Failed(FetchFailure::TooLarge);
        }
        let mut body = Vec::new();
        let reader = UntilDeadline {
            inner: response.into_reader(),
            deadline,
        };
        match reader.take(self.limit + 1).read_to_end(&mut body) {
            Ok(_) if body.len() as u64 > self.limit => Hop::Failed(FetchFailure::TooLarge),
            Ok(_) => Hop::Body(body),
            Err(e) if timed_out(&e) => Hop::Failed(FetchFailure::TimedOut),
            Err(e) => Hop::Failed(FetchFailure::Other(e.to_string())),
        }
    }
}

impl Fetcher for RealFetcher {
    fn get(&self, url: &str) -> Hop {
        self.get_until(url, Instant::now() + self.total)
    }

    fn get_until(&self, url: &str, deadline: Instant) -> Hop {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Hop::Failed(FetchFailure::TimedOut);
        }
        let mut request = self.agent.get(url);
        // Near the end, the deadline is shorter than the wait for a read.
        // A request deadline then also bounds the connection and the head.
        if left < self.wait {
            request = request.timeout(left);
        }
        match request.call() {
            Ok(response) => self.read(response, deadline),
            Err(ureq::Error::Status(code, _)) => Hop::Failed(FetchFailure::Status(code)),
            Err(ureq::Error::Transport(t)) => Hop::Failed(transport_failure(&t)),
        }
    }
}

/// A reader that fails with `TimedOut` once the deadline has passed. Each
/// read is already capped by the wait limit, so a fetch ends at most one
/// wait after its deadline.
struct UntilDeadline<R> {
    inner: R,
    deadline: Instant,
}

impl<R: Read> Read for UntilDeadline<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if Instant::now() >= self.deadline {
            return Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "total time limit",
            ));
        }
        self.inner.read(buf)
    }
}

fn timed_out(e: &std::io::Error) -> bool {
    matches!(
        e.kind(),
        std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
    )
}

/// The failure for a request that got no answer.
fn transport_failure(t: &ureq::Transport) -> FetchFailure {
    let mut source = std::error::Error::source(t);
    while let Some(e) = source {
        if e.downcast_ref::<std::io::Error>().is_some_and(timed_out) {
            return FetchFailure::TimedOut;
        }
        source = e.source();
    }
    match t.kind() {
        ureq::ErrorKind::Dns | ureq::ErrorKind::ConnectionFailed => FetchFailure::Offline,
        _ => FetchFailure::Other(t.to_string()),
    }
}

/// The scheme of an address, from its literal start. `https:foo` has no
/// `//` and is neither.
#[derive(PartialEq, Eq)]
enum Scheme {
    Https,
    Http,
    Other,
}

fn scheme_of(url: &str) -> Scheme {
    let start: String = url.trim().chars().take(8).collect::<String>();
    let start = start.to_ascii_lowercase();
    if start.starts_with("https://") {
        Scheme::Https
    } else if start.starts_with("http://") {
        Scheme::Http
    } else {
        Scheme::Other
    }
}

/// The failure of an address that is not a web address.
fn not_a_web_address() -> FetchFailure {
    FetchFailure::Other("not a web address".to_string())
}

/// The session key of an address: the cache key, so two spellings of one
/// image share one state.
fn key_of(url: &str) -> String {
    normalize_image_url(url).unwrap_or_else(|| url.trim().to_string())
}

/// How a fetch ended.
enum Outcome {
    Saved,
    RedirectBlocked(String),
    Failed(FetchFailure),
}

/// The inputs of one fetch, copied from the store when it starts.
struct Job {
    fetcher: Arc<dyn Fetcher>,
    fetches: Arc<AtomicUsize>,
    cache: Option<ImageCache>,
    allowed: AllowedHosts,
    /// The hosts this address may follow a redirect to for the session.
    permits: Vec<String>,
    url: String,
    /// The limit for the whole fetch, every hop included.
    total: Duration,
}

impl Job {
    /// Fetches the address, follows the redirects the rules allow, checks
    /// the bytes and saves them through core. It runs off the UI thread.
    fn run(self) -> Outcome {
        let Some(cache) = &self.cache else {
            return Outcome::Failed(FetchFailure::Other("no image cache".to_string()));
        };
        let deadline = Instant::now() + self.total;
        let origin = host_of(&self.url);
        let mut current = self.url.trim().to_string();
        let mut redirects = 0;
        loop {
            if Instant::now() >= deadline {
                return Outcome::Failed(FetchFailure::TimedOut);
            }
            self.fetches.fetch_add(1, Ordering::SeqCst);
            let location = match self.fetcher.get_until(&current, deadline) {
                Hop::Body(bytes) => return save(cache, &self.url, &bytes),
                Hop::Failed(f) => return Outcome::Failed(f),
                Hop::Redirect(location) => location,
            };
            redirects += 1;
            if redirects > MAX_REDIRECTS {
                return Outcome::Failed(FetchFailure::TooManyRedirects);
            }
            let Some(next) = url::Url::parse(&current)
                .and_then(|base| base.join(location.trim()))
                .ok()
            else {
                return Outcome::Failed(FetchFailure::Other("bad redirect".to_string()));
            };
            if next.scheme() != "https" {
                return Outcome::Failed(FetchFailure::Other(
                    "redirect to an address that is not https".to_string(),
                ));
            }
            let next = next.to_string();
            let Some(host) = host_of(&next) else {
                return Outcome::Failed(FetchFailure::Other("bad redirect".to_string()));
            };
            // The reader chose the first host. Any other host must be
            // allowed, or be one this address may follow for the session.
            let ok = origin.as_deref() == Some(host.as_str())
                || self.allowed.matches(&host)
                || self.permits.iter().any(|p| host_matches(p, &host));
            if !ok {
                return Outcome::RedirectBlocked(host);
            }
            current = next;
        }
    }
}

/// Checks the first bytes of a body and saves it as the copy of `url`.
fn save(cache: &ImageCache, url: &str, bytes: &[u8]) -> Outcome {
    if bytes.len() as u64 > MAX_BYTES {
        return Outcome::Failed(FetchFailure::TooLarge);
    }
    if image_extension(bytes).is_none() {
        return Outcome::Failed(if other_image_format(bytes) {
            FetchFailure::Unsupported
        } else {
            FetchFailure::NotAnImage
        });
    }
    match cache.save(url, bytes) {
        Ok(_) => Outcome::Saved,
        Err(e) => Outcome::Failed(FetchFailure::Other(e.to_string())),
    }
}

/// Whether `bytes` are an image in a format we do not draw: SVG, or a format
/// that the `image` crate knows (BMP, TIFF, ICO, AVIF and others). The
/// formats we draw are [`image_extension`]'s, checked first.
fn other_image_format(bytes: &[u8]) -> bool {
    if image::guess_format(bytes).is_ok() {
        return true;
    }
    let head = &bytes[..bytes.len().min(1024)];
    let text = String::from_utf8_lossy(head).to_ascii_lowercase();
    let text = text.trim_start_matches('\u{feff}').trim_start();
    text.starts_with('<') && text.contains("<svg")
}

/// A call to run when a fetch ends.
type OnDone = Box<dyn FnOnce(&mut App)>;

/// A fetch that waits for a free slot.
struct Queued {
    key: String,
    url: String,
    /// A Reload: it fetches although a saved copy exists, and a failure
    /// keeps that copy.
    force: bool,
}

/// A state of the session that is not a saved copy.
enum Session {
    Failed(FetchFailure),
    RedirectBlocked(String),
}

struct Inner {
    fetcher: Arc<dyn Fetcher>,
    /// Calls of [`Fetcher::get`] so far (tests count them).
    fetches: Arc<AtomicUsize>,
    cache: Option<ImageCache>,
    allowed: AllowedHosts,
    /// Failures and blocked redirects, by address key.
    sessions: HashMap<String, Session>,
    /// The one-address permissions to follow a redirect: the hosts, by
    /// address key.
    permits: HashMap<String, Vec<String>>,
    /// The addresses with a fetch running or queued, with their callers.
    in_flight: HashMap<String, Vec<OnDone>>,
    /// The fetches that wait for a free slot.
    queue: VecDeque<Queued>,
    /// The fetches that run now.
    running: usize,
    /// A test store with no fake installed: a fetch panics on the UI
    /// thread, so no test reaches the network or the real cache.
    #[cfg(test)]
    stand_in: bool,
}

/// The state of every remote address for the session. A clone shares the
/// same state. The app keeps one as a global, beside the image store.
#[derive(Clone)]
pub(crate) struct RemoteImages {
    inner: Rc<RefCell<Inner>>,
}

impl Default for RemoteImages {
    /// The app's store: [`Self::in_app_folder`].
    #[cfg(not(test))]
    fn default() -> Self {
        Self::in_app_folder()
    }

    /// A test store: no cache, and a fetcher that panics. A fetch panics on
    /// the UI thread first. A test installs a fake fetcher and a temp cache
    /// (`set_fetcher` and `set_cache`), so no test reaches the network or
    /// the developer's own cache.
    #[cfg(test)]
    fn default() -> Self {
        let store = Self::new(Arc::new(NoFetcher), None);
        store.inner.borrow_mut().stand_in = true;
        store
    }
}

/// The fetcher of a test store with no fake installed.
#[cfg(test)]
struct NoFetcher;

#[cfg(test)]
impl Fetcher for NoFetcher {
    fn get(&self, url: &str) -> Hop {
        panic!("a test fetched {url} with no fake fetcher installed (set_fetcher)")
    }
}

impl Global for RemoteImages {}

impl RemoteImages {
    fn new(fetcher: Arc<dyn Fetcher>, cache: Option<ImageCache>) -> Self {
        Self {
            inner: Rc::new(RefCell::new(Inner {
                fetcher,
                fetches: Arc::new(AtomicUsize::new(0)),
                cache,
                allowed: AllowedHosts::default(),
                sessions: HashMap::new(),
                permits: HashMap::new(),
                in_flight: HashMap::new(),
                queue: VecDeque::new(),
                running: 0,
                #[cfg(test)]
                stand_in: false,
            })),
        }
    }

    /// The store of the app: the real fetcher, so the one HTTP client, and
    /// the cache in the app folder. With no app folder, it has no cache and
    /// remote images cannot load.
    fn in_app_folder() -> Self {
        let cache = match ImageCache::in_app_dir() {
            Ok(cache) => Some(cache),
            Err(e) => {
                tracing::warn!("no image cache, remote images cannot load: {e}");
                None
            }
        };
        Self::new(Arc::new(RealFetcher::new()), cache)
    }

    /// Keeps a copy of the allowed hosts. The workspace calls it when the
    /// settings load and after each change of the list.
    pub(crate) fn set_allowed_hosts(&mut self, hosts: &[String]) {
        self.inner.borrow_mut().allowed = hosts.iter().cloned().collect();
    }

    /// Whether `host` is on the allowed list. The rule is core's
    /// [`host_matches`], as for the settings.
    pub(crate) fn is_allowed(&self, host: &str) -> bool {
        self.inner.borrow().allowed.matches(host)
    }

    /// The image cache the store saves to and finds copies in: a cheap
    /// handle over a folder path. The preview's planner, which runs off
    /// the UI thread where the store cannot go, finds saved copies with it.
    pub(crate) fn cache(&self) -> Option<ImageCache> {
        self.inner.borrow().cache.clone()
    }

    /// The state of `url` now. It starts no work. A saved copy wins, then a
    /// failure of the session, then a fetch under way, then the scheme. An
    /// address on an allowed host is `Blocked` too, until `auto_load`
    /// starts it.
    pub(crate) fn state(&self, url: &str) -> RemoteState {
        let inner = self.inner.borrow();
        if let Some(path) = inner.cache.as_ref().and_then(|c| c.find(url)) {
            return RemoteState::Saved(path);
        }
        let key = key_of(url);
        match inner.sessions.get(&key) {
            Some(Session::Failed(f)) => return RemoteState::Failed(f.clone()),
            Some(Session::RedirectBlocked(host)) => {
                return RemoteState::RedirectBlocked { host: host.clone() };
            }
            None => {}
        }
        if inner.in_flight.contains_key(&key) {
            return RemoteState::Loading;
        }
        match scheme_of(url) {
            Scheme::Http => return RemoteState::Insecure,
            Scheme::Other => return RemoteState::Failed(not_a_web_address()),
            Scheme::Https => {}
        }
        match host_of(url) {
            Some(host) => RemoteState::Blocked { host },
            None => RemoteState::Failed(not_a_web_address()),
        }
    }

    /// Starts the fetch of `url` at the reader's request. It runs off the UI
    /// thread, and `on_done` runs on the UI thread when it ends. At most four
    /// fetches run at once, and the others wait in a queue. For an address
    /// that is loading, it adds `on_done` and starts nothing. It forgets a
    /// failure of the session. It does nothing for a saved copy and for an
    /// address that is not `https`.
    pub(crate) fn load(
        &mut self,
        url: &str,
        cx: &mut App,
        on_done: impl FnOnce(&mut App) + 'static,
    ) {
        self.fetch(url, false, cx, Box::new(on_done));
    }

    /// Fetches `url` again, as [`Self::load`] does, also when it has a saved
    /// copy. The old copy stays, and stays drawn, until the new bytes
    /// arrive. Core's save then replaces it in one step. A failed fetch keeps
    /// the old copy and records no failure, so the old image stays. With no
    /// saved copy it is a Load.
    pub(crate) fn reload(
        &mut self,
        url: &str,
        cx: &mut App,
        on_done: impl FnOnce(&mut App) + 'static,
    ) {
        self.fetch(url, true, cx, Box::new(on_done));
    }

    /// Queues the fetch of `url`. `force` fetches past a saved copy.
    fn fetch(&mut self, url: &str, force: bool, cx: &mut App, on_done: OnDone) {
        let key = key_of(url);
        {
            let mut inner = self.inner.borrow_mut();
            #[cfg(test)]
            assert!(
                !inner.stand_in,
                "a test asked to fetch {url} with the default store: install a fake \
                 fetcher and a temp cache first (set_fetcher and set_cache)"
            );
            if let Some(callers) = inner.in_flight.get_mut(&key) {
                callers.push(on_done);
                return;
            }
            if scheme_of(url) != Scheme::Https || host_of(url).is_none() {
                return;
            }
            if !force && inner.cache.as_ref().and_then(|c| c.find(url)).is_some() {
                return;
            }
            inner.sessions.remove(&key);
            inner.in_flight.insert(key.clone(), vec![on_done]);
            inner.queue.push_back(Queued {
                key,
                url: url.trim().to_string(),
                force,
            });
        }
        self.start_queued(cx);
    }

    /// Starts the queued fetches while a slot is free.
    fn start_queued(&self, cx: &mut App) {
        loop {
            let (key, url, force, job) = {
                let mut inner = self.inner.borrow_mut();
                if inner.running >= MAX_RUNNING {
                    return;
                }
                let Some(Queued { key, url, force }) = inner.queue.pop_front() else {
                    return;
                };
                inner.running += 1;
                let job = Job {
                    fetcher: inner.fetcher.clone(),
                    fetches: inner.fetches.clone(),
                    cache: inner.cache.clone(),
                    allowed: inner.allowed.clone(),
                    permits: inner.permits.get(&key).cloned().unwrap_or_default(),
                    url: url.clone(),
                    total: TOTAL_LIMIT,
                };
                (key, url, force, job)
            };
            let work = crate::tasks::blocking(move || job.run());
            let this = self.clone();
            cx.spawn(async move |cx| {
                let outcome = work
                    .await
                    .unwrap_or_else(|e| Outcome::Failed(FetchFailure::Other(e)));
                cx.update(|cx| this.finish(key, &url, force, outcome, cx));
            })
            .detach();
        }
    }

    /// Records how a fetch ended, starts the next queued fetch, and calls the
    /// callers of the address. A forced fetch that failed while a saved copy
    /// is still there records nothing: the old copy stays drawn.
    fn finish(&self, key: String, url: &str, force: bool, outcome: Outcome, cx: &mut App) {
        let callers = {
            let mut inner = self.inner.borrow_mut();
            inner.running -= 1;
            let kept = force && inner.cache.as_ref().and_then(|c| c.find(url)).is_some();
            match outcome {
                _ if kept => {}
                Outcome::Saved => {}
                Outcome::RedirectBlocked(host) => {
                    inner
                        .sessions
                        .insert(key.clone(), Session::RedirectBlocked(host));
                }
                Outcome::Failed(f) => {
                    inner.sessions.insert(key.clone(), Session::Failed(f));
                }
            }
            inner.in_flight.remove(&key).unwrap_or_default()
        };
        self.start_queued(cx);
        for on_done in callers {
            on_done(cx);
        }
    }

    /// Lets `url` follow its blocked redirect, to the host it names, for the
    /// session. The host does not join the allowed list. Each call adds a
    /// host, and the hosts added before stay, so a chain through several
    /// hosts loads after one call for each. It does nothing when the
    /// redirect of `url` is not blocked.
    pub(crate) fn allow_redirect_once(&mut self, url: &str) {
        let key = key_of(url);
        let mut inner = self.inner.borrow_mut();
        if let Some(Session::RedirectBlocked(host)) = inner.sessions.get(&key) {
            let host = host.clone();
            let hosts = inner.permits.entry(key).or_default();
            if !hosts.contains(&host) {
                hosts.push(host);
            }
        }
    }

    /// Forgets the failure and the redirect permission of `url`, so it can
    /// load again.
    pub(crate) fn retry(&mut self, url: &str) {
        let key = key_of(url);
        let mut inner = self.inner.borrow_mut();
        inner.sessions.remove(&key);
        inner.permits.remove(&key);
    }

    /// Removes the saved copy of `url` and its session state. The address
    /// shows its placeholder again.
    pub(crate) fn clear_copy(&mut self, url: &str) {
        self.remove_copy(url);
        self.retry(url);
    }

    /// Forgets every failure, blocked redirect and one-address redirect
    /// permission of the session. Fetches under way go on. The Preferences
    /// "Clear cached images" calls it after core empties the cache, so each
    /// cleared address starts over from its placeholder.
    ///
    /// It drops every session failure, so a dead address on an allowed host
    /// gets one more request once it is on screen again. It drops every
    /// one-address redirect permission, so a redirect that the reader
    /// approved once asks again.
    pub(crate) fn forget_sessions(&mut self) {
        let mut inner = self.inner.borrow_mut();
        inner.sessions.clear();
        inner.permits.clear();
    }

    /// Removes the saved copy of `url` through core.
    fn remove_copy(&self, url: &str) {
        let inner = self.inner.borrow();
        if let Some(cache) = &inner.cache
            && let Err(e) = cache.remove(url)
        {
            tracing::warn!("cannot remove the saved copy of {url}: {e}");
        }
    }

    /// Loads each address of `urls` that is on an allowed host, with no saved
    /// copy, no fetch, and no failure of the session. The host check uses
    /// the host the store reads from the address. It starts nothing else. `on_done` runs once for each fetch
    /// that ends.
    pub(crate) fn auto_load(
        &mut self,
        urls: &[String],
        cx: &mut App,
        on_done: impl Fn(&mut App) + 'static,
    ) {
        let on_done = Rc::new(on_done);
        for url in urls {
            let RemoteState::Blocked { host } = self.state(url) else {
                continue;
            };
            if !self.is_allowed(&host) {
                continue;
            }
            let on_done = on_done.clone();
            self.load(url, cx, move |cx| on_done(cx));
        }
    }
}

/// Installs `fetcher` in the app's store.
#[cfg(test)]
pub(crate) fn set_fetcher(cx: &mut App, fetcher: Arc<dyn Fetcher>) {
    let store = cx.default_global::<RemoteImages>();
    let mut inner = store.inner.borrow_mut();
    inner.fetcher = fetcher;
    inner.stand_in = false;
}

/// Installs `cache` in the app's store.
#[cfg(test)]
pub(crate) fn set_cache(cx: &mut App, cache: ImageCache) {
    cx.default_global::<RemoteImages>().inner.borrow_mut().cache = Some(cache);
}

impl RemoteImages {
    /// Puts `url` in `state` with no fetch: `Failed`, `RedirectBlocked`,
    /// or `Loading`. Other states come from the cache and the scheme.
    #[cfg(test)]
    pub(crate) fn set_state_for_test(&self, url: &str, state: RemoteState) {
        let key = key_of(url);
        let mut inner = self.inner.borrow_mut();
        match state {
            RemoteState::Failed(f) => {
                inner.sessions.insert(key, Session::Failed(f));
            }
            RemoteState::RedirectBlocked { host } => {
                inner.sessions.insert(key, Session::RedirectBlocked(host));
            }
            RemoteState::Loading => {
                inner.in_flight.insert(key, Vec::new());
            }
            other => panic!("cannot set {other:?} for a test"),
        }
    }
}

/// The calls of [`Fetcher::get`] so far.
#[cfg(test)]
pub(crate) fn fetch_count(cx: &App) -> usize {
    cx.global::<RemoteImages>()
        .inner
        .borrow()
        .fetches
        .load(Ordering::SeqCst)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;
    use std::path::Path;
    use std::sync::{Condvar, Mutex};

    use gpui_kit::TestAppContext;
    use kimun_core::system::SystemPath;
    use tokio::runtime::Runtime;

    const PNG: &[u8] = &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0, 13];
    const A: &str = "https://img.example.com/a.png";

    /// Holds every call until the test opens it.
    #[derive(Default)]
    struct Gate {
        open: Mutex<bool>,
        cv: Condvar,
        entered: AtomicUsize,
    }

    impl Gate {
        fn release(&self) {
            *self.open.lock().unwrap() = true;
            self.cv.notify_all();
        }

        /// Holds the next calls again.
        fn close(&self) {
            *self.open.lock().unwrap() = false;
        }
    }

    /// A fetcher that answers from a script and logs the calls.
    #[derive(Default)]
    struct Fake {
        script: Mutex<HashMap<String, Hop>>,
        calls: Mutex<Vec<String>>,
        gate: Option<Gate>,
    }

    impl Fake {
        fn new(script: &[(&str, Hop)]) -> Arc<Fake> {
            Arc::new(Fake {
                script: Mutex::new(
                    script
                        .iter()
                        .map(|(u, h)| (u.to_string(), h.clone()))
                        .collect(),
                ),
                ..Default::default()
            })
        }

        fn gated(script: &[(&str, Hop)]) -> Arc<Fake> {
            let fake = Fake::new(script);
            let mut fake = Arc::into_inner(fake).unwrap();
            fake.gate = Some(Gate::default());
            Arc::new(fake)
        }

        fn calls(&self) -> Vec<String> {
            self.calls.lock().unwrap().clone()
        }

        fn answer(&self, url: &str, hop: Hop) {
            self.script.lock().unwrap().insert(url.to_string(), hop);
        }
    }

    impl Fetcher for Fake {
        fn get(&self, url: &str) -> Hop {
            self.calls.lock().unwrap().push(url.to_string());
            if let Some(gate) = &self.gate {
                gate.entered.fetch_add(1, Ordering::SeqCst);
                let mut open = gate.open.lock().unwrap();
                while !*open {
                    open = gate.cv.wait(open).unwrap();
                }
            }
            self.script
                .lock()
                .unwrap()
                .get(url)
                .cloned()
                .unwrap_or(Hop::Failed(FetchFailure::Status(404)))
        }
    }

    struct Env {
        rt: Option<Runtime>,
        _dir: tempfile::TempDir,
        cache: ImageCache,
    }

    impl Env {
        fn rt(&self) -> &Runtime {
            self.rt.as_ref().unwrap()
        }
    }

    impl Drop for Env {
        /// A failed test can leave a gated fetch blocked. Do not wait for it.
        fn drop(&mut self) {
            if let Some(rt) = self.rt.take() {
                rt.shutdown_background();
            }
        }
    }

    /// A store in the app with `fake` as its fetcher and a temp cache.
    fn env(cx: &mut TestAppContext, fake: &Arc<Fake>) -> Env {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let dir = tempfile::tempdir().unwrap();
        let cache = ImageCache::at(
            SystemPath::try_absolute(dir.path())
                .unwrap()
                .join("image-cache"),
        );
        let fetcher: Arc<dyn Fetcher> = fake.clone();
        let c = cache.clone();
        cx.update(|cx| {
            set_fetcher(cx, fetcher);
            set_cache(cx, c);
        });
        Env {
            rt: Some(rt),
            _dir: dir,
            cache,
        }
    }

    fn store(cx: &mut TestAppContext) -> RemoteImages {
        cx.update(|cx| cx.global::<RemoteImages>().clone())
    }

    fn count(cx: &mut TestAppContext) -> usize {
        cx.update(|cx| fetch_count(cx))
    }

    fn state(cx: &mut TestAppContext, url: &str) -> RemoteState {
        store(cx).state(url)
    }

    fn allow(cx: &mut TestAppContext, hosts: &[&str]) {
        let hosts: Vec<String> = hosts.iter().map(|h| h.to_string()).collect();
        store(cx).set_allowed_hosts(&hosts);
    }

    /// Runs the app and the runtime until `done` holds.
    fn settle(
        env: &Env,
        cx: &mut TestAppContext,
        mut done: impl FnMut(&mut TestAppContext) -> bool,
    ) {
        for _ in 0..400 {
            let _guard = env.rt().enter();
            env.rt()
                .block_on(tokio::time::sleep(Duration::from_millis(5)));
            cx.run_until_parked();
            if done(cx) {
                return;
            }
        }
        panic!("timed out waiting for the store");
    }

    /// Loads `url` and waits for its `on_done`.
    fn load(env: &Env, cx: &mut TestAppContext, url: &str) {
        let done = Rc::new(std::cell::Cell::new(false));
        let d = done.clone();
        {
            let _guard = env.rt().enter();
            cx.update(|cx| store_of(cx).load(url, cx, move |_| d.set(true)));
        }
        settle(env, cx, |_| done.get());
    }

    fn store_of(cx: &App) -> RemoteImages {
        cx.global::<RemoteImages>().clone()
    }

    fn auto(env: &Env, cx: &mut TestAppContext, urls: &[&str]) {
        let urls: Vec<String> = urls.iter().map(|u| u.to_string()).collect();
        let _guard = env.rt().enter();
        cx.update(|cx| store_of(cx).auto_load(&urls, cx, |_| {}));
    }

    fn host(h: &str) -> RemoteState {
        RemoteState::Blocked {
            host: h.to_string(),
        }
    }

    /// The store the app makes on first use holds the cache in the app
    /// folder, so a saved copy shows in production with no test seam. On a
    /// host with no app folder it has no cache.
    #[test]
    fn the_default_store_uses_the_cache_in_the_app_folder() {
        let store = RemoteImages::in_app_folder();
        let inner = store.inner.borrow();
        match ImageCache::in_app_dir() {
            Ok(want) => assert_eq!(inner.cache.as_ref().expect("a cache").root(), want.root()),
            Err(_) => assert!(inner.cache.is_none()),
        }
    }

    /// The store a test gets when it installs nothing has no cache, and a
    /// fetch panics before it reaches the network.
    #[test]
    fn the_test_default_store_has_no_cache_and_refuses_to_fetch() {
        let store = RemoteImages::default();
        assert!(store.cache().is_none());
        assert!(store.inner.borrow().stand_in);
        let caught = std::panic::catch_unwind(|| NoFetcher.get(A));
        assert!(caught.is_err());
    }

    /// A Load with the store a test gets when it installs nothing panics on
    /// the UI thread, before any fetch starts.
    #[gpui_kit::test]
    #[should_panic(expected = "install a fake fetcher")]
    fn a_load_with_no_fake_installed_panics(cx: &mut TestAppContext) {
        cx.update(|cx| {
            cx.default_global::<RemoteImages>()
                .clone()
                .load(A, cx, |_| {})
        });
    }

    #[gpui_kit::test]
    fn a_blocked_host_starts_no_fetch_until_load_is_called(cx: &mut TestAppContext) {
        let fake = Fake::new(&[(A, Hop::Body(PNG.to_vec()))]);
        let env = env(cx, &fake);
        assert_eq!(state(cx, A), host("img.example.com"));
        auto(&env, cx, &[A]);
        settle(&env, cx, |_| true);
        assert_eq!(count(cx), 0);
        assert_eq!(state(cx, A), host("img.example.com"));
        load(&env, cx, A);
        assert_eq!(count(cx), 1);
    }

    #[gpui_kit::test]
    fn load_fetches_checks_the_bytes_saves_and_reports_saved(cx: &mut TestAppContext) {
        let fake = Fake::new(&[(A, Hop::Body(PNG.to_vec()))]);
        let env = env(cx, &fake);
        load(&env, cx, A);
        let saved = env.cache.find(A).expect("a saved copy");
        assert_eq!(state(cx, A), RemoteState::Saved(saved.clone()));
        assert_eq!(std::fs::read(saved).unwrap(), PNG);
        assert_eq!(fake.calls(), vec![A.to_string()]);
    }

    #[gpui_kit::test]
    fn a_saved_copy_wins_over_blocked_and_over_a_removed_host(cx: &mut TestAppContext) {
        let fake = Fake::new(&[(A, Hop::Body(PNG.to_vec()))]);
        let env = env(cx, &fake);
        allow(cx, &["img.example.com"]);
        load(&env, cx, A);
        allow(cx, &[]);
        assert!(matches!(state(cx, A), RemoteState::Saved(_)));
        auto(&env, cx, &[A]);
        settle(&env, cx, |_| true);
        assert_eq!(count(cx), 1);
    }

    #[gpui_kit::test]
    fn an_http_address_is_insecure_and_never_fetched(cx: &mut TestAppContext) {
        let url = "http://img.example.com/a.png";
        let fake = Fake::new(&[(url, Hop::Body(PNG.to_vec()))]);
        let env = env(cx, &fake);
        allow(cx, &["img.example.com"]);
        assert_eq!(state(cx, url), RemoteState::Insecure);
        assert_eq!(
            state(cx, "HTTP://img.example.com/a.png"),
            RemoteState::Insecure
        );
        auto(&env, cx, &[url]);
        let _guard = env.rt().enter();
        cx.update(|cx| store_of(cx).load(url, cx, |_| {}));
        drop(_guard);
        settle(&env, cx, |_| true);
        assert_eq!(count(cx), 0);
        assert_eq!(state(cx, url), RemoteState::Insecure);
    }

    #[gpui_kit::test]
    fn an_address_that_is_not_http_or_https_fails_before_any_fetch(cx: &mut TestAppContext) {
        let fake = Fake::new(&[]);
        let env = env(cx, &fake);
        allow(cx, &["img.example.com"]);
        for url in ["https:img.example.com/a.png", "ftp://img.example.com/a.png"] {
            assert!(
                matches!(state(cx, url), RemoteState::Failed(_)),
                "{url}: {:?}",
                state(cx, url)
            );
            auto(&env, cx, &[url]);
            let _guard = env.rt().enter();
            cx.update(|cx| store_of(cx).load(url, cx, |_| {}));
        }
        settle(&env, cx, |_| true);
        assert_eq!(count(cx), 0);
        assert_eq!(
            state(cx, "HTTPS://img.example.com/a.png"),
            host("img.example.com")
        );
    }

    #[gpui_kit::test]
    fn a_body_that_is_not_an_image_fails_and_is_not_saved(cx: &mut TestAppContext) {
        let fake = Fake::new(&[(A, Hop::Body(b"<html>hello</html>".to_vec()))]);
        let env = env(cx, &fake);
        load(&env, cx, A);
        assert_eq!(state(cx, A), RemoteState::Failed(FetchFailure::NotAnImage));
        assert!(env.cache.find(A).is_none());
        assert_eq!(env.cache.size(), 0);
    }

    #[gpui_kit::test]
    fn an_svg_fails_as_unsupported_and_is_not_saved(cx: &mut TestAppContext) {
        let svg = b"<?xml version=\"1.0\"?>\n<svg xmlns=\"http://www.w3.org/2000/svg\"/>";
        let fake = Fake::new(&[(A, Hop::Body(svg.to_vec()))]);
        let env = env(cx, &fake);
        load(&env, cx, A);
        assert_eq!(state(cx, A), RemoteState::Failed(FetchFailure::Unsupported));
        assert!(env.cache.find(A).is_none());
    }

    #[gpui_kit::test]
    fn a_body_over_20_mb_fails_as_too_large_and_is_not_saved(cx: &mut TestAppContext) {
        let fake = Fake::new(&[(A, Hop::Failed(FetchFailure::TooLarge))]);
        let env = env(cx, &fake);
        load(&env, cx, A);
        assert_eq!(state(cx, A), RemoteState::Failed(FetchFailure::TooLarge));
        assert!(env.cache.find(A).is_none());
    }

    #[gpui_kit::test]
    fn a_failed_address_is_not_asked_again_by_auto_load(cx: &mut TestAppContext) {
        let fake = Fake::new(&[(A, Hop::Failed(FetchFailure::Status(500)))]);
        let env = env(cx, &fake);
        allow(cx, &["img.example.com"]);
        for _ in 0..3 {
            auto(&env, cx, &[A]);
            settle(&env, cx, |cx| state(cx, A) != RemoteState::Loading);
        }
        assert_eq!(count(cx), 1);
        assert_eq!(state(cx, A), RemoteState::Failed(FetchFailure::Status(500)));
    }

    #[gpui_kit::test]
    fn retry_clears_the_failure_and_load_asks_again(cx: &mut TestAppContext) {
        let fake = Fake::new(&[(A, Hop::Failed(FetchFailure::TimedOut))]);
        let env = env(cx, &fake);
        load(&env, cx, A);
        assert_eq!(state(cx, A), RemoteState::Failed(FetchFailure::TimedOut));
        store(cx).retry(A);
        assert_eq!(state(cx, A), host("img.example.com"));
        fake.answer(A, Hop::Body(PNG.to_vec()));
        load(&env, cx, A);
        assert_eq!(count(cx), 2);
        assert!(matches!(state(cx, A), RemoteState::Saved(_)));
    }

    #[gpui_kit::test]
    fn a_redirect_to_an_allowed_host_is_followed(cx: &mut TestAppContext) {
        let fake = Fake::new(&[
            (A, Hop::Redirect("https://cdn.example.com/x.png".into())),
            (
                "https://cdn.example.com/x.png",
                Hop::Redirect("/y.png".into()),
            ),
            ("https://cdn.example.com/y.png", Hop::Body(PNG.to_vec())),
        ]);
        let env = env(cx, &fake);
        allow(cx, &["cdn.example.com"]);
        load(&env, cx, A);
        assert_eq!(
            fake.calls(),
            vec![
                A.to_string(),
                "https://cdn.example.com/x.png".to_string(),
                "https://cdn.example.com/y.png".to_string(),
            ]
        );
        // The copy is saved under the address the note holds.
        assert_eq!(state(cx, A), RemoteState::Saved(env.cache.find(A).unwrap()));
    }

    #[gpui_kit::test]
    fn a_redirect_to_a_host_that_is_not_allowed_stops_with_redirect_blocked(
        cx: &mut TestAppContext,
    ) {
        let fake = Fake::new(&[
            (A, Hop::Redirect("https://tracker.example.net/x.png".into())),
            ("https://tracker.example.net/x.png", Hop::Body(PNG.to_vec())),
        ]);
        let env = env(cx, &fake);
        load(&env, cx, A);
        assert_eq!(fake.calls(), vec![A.to_string()]);
        assert_eq!(
            state(cx, A),
            RemoteState::RedirectBlocked {
                host: "tracker.example.net".into()
            }
        );
        assert!(env.cache.find(A).is_none());
    }

    #[gpui_kit::test]
    fn load_on_a_redirect_blocked_address_follows_it_once_for_the_session(cx: &mut TestAppContext) {
        let other = "https://img.example.com/b.png";
        let fake = Fake::new(&[
            (A, Hop::Redirect("https://tracker.example.net/x.png".into())),
            (
                other,
                Hop::Redirect("https://tracker.example.net/x.png".into()),
            ),
            ("https://tracker.example.net/x.png", Hop::Body(PNG.to_vec())),
        ]);
        let env = env(cx, &fake);
        load(&env, cx, A);
        store(cx).allow_redirect_once(A);
        load(&env, cx, A);
        assert!(matches!(state(cx, A), RemoteState::Saved(_)));
        // The permission stays for the session: a reload follows again.
        let done = Rc::new(std::cell::Cell::new(false));
        let d = done.clone();
        {
            let _guard = env.rt().enter();
            cx.update(|cx| store_of(cx).reload(A, cx, move |_| d.set(true)));
        }
        settle(&env, cx, |_| done.get());
        assert!(matches!(state(cx, A), RemoteState::Saved(_)));
        // It is for this one address only, and the host is not allowed.
        load(&env, cx, other);
        assert_eq!(
            state(cx, other),
            RemoteState::RedirectBlocked {
                host: "tracker.example.net".into()
            }
        );
        assert!(!store(cx).is_allowed("tracker.example.net"));
    }

    #[gpui_kit::test]
    fn retry_does_not_keep_the_one_address_permission(cx: &mut TestAppContext) {
        let fake = Fake::new(&[
            (A, Hop::Redirect("https://tracker.example.net/x.png".into())),
            (
                "https://tracker.example.net/x.png",
                Hop::Failed(FetchFailure::Status(503)),
            ),
        ]);
        let env = env(cx, &fake);
        load(&env, cx, A);
        store(cx).allow_redirect_once(A);
        load(&env, cx, A);
        assert_eq!(state(cx, A), RemoteState::Failed(FetchFailure::Status(503)));
        store(cx).retry(A);
        load(&env, cx, A);
        assert_eq!(
            state(cx, A),
            RemoteState::RedirectBlocked {
                host: "tracker.example.net".into()
            }
        );
    }

    #[gpui_kit::test]
    fn the_fourth_redirect_fails(cx: &mut TestAppContext) {
        let fake = Fake::new(&[
            (A, Hop::Redirect("/1".into())),
            ("https://img.example.com/1", Hop::Redirect("/2".into())),
            ("https://img.example.com/2", Hop::Redirect("/3".into())),
            ("https://img.example.com/3", Hop::Redirect("/4".into())),
            ("https://img.example.com/4", Hop::Body(PNG.to_vec())),
        ]);
        let env = env(cx, &fake);
        load(&env, cx, A);
        assert_eq!(
            state(cx, A),
            RemoteState::Failed(FetchFailure::TooManyRedirects)
        );
        assert_eq!(count(cx), 4);
        // Three redirects are followed.
        fake.answer("https://img.example.com/3", Hop::Body(PNG.to_vec()));
        store(cx).retry(A);
        load(&env, cx, A);
        assert!(matches!(state(cx, A), RemoteState::Saved(_)));
    }

    #[gpui_kit::test]
    fn a_redirect_to_http_fails(cx: &mut TestAppContext) {
        let fake = Fake::new(&[
            (A, Hop::Redirect("http://img.example.com/a.png".into())),
            ("http://img.example.com/a.png", Hop::Body(PNG.to_vec())),
        ]);
        let env = env(cx, &fake);
        allow(cx, &["img.example.com"]);
        load(&env, cx, A);
        assert!(matches!(state(cx, A), RemoteState::Failed(_)));
        assert_eq!(fake.calls(), vec![A.to_string()]);
    }

    #[gpui_kit::test]
    fn two_loads_of_one_address_make_one_fetch_and_call_both_on_done(cx: &mut TestAppContext) {
        let fake = Fake::gated(&[(A, Hop::Body(PNG.to_vec()))]);
        let env = env(cx, &fake);
        let calls = Rc::new(std::cell::Cell::new(0));
        {
            let _guard = env.rt().enter();
            for _ in 0..2 {
                let c = calls.clone();
                cx.update(|cx| store_of(cx).load(A, cx, move |_| c.set(c.get() + 1)));
            }
        }
        assert_eq!(state(cx, A), RemoteState::Loading);
        fake.gate.as_ref().unwrap().release();
        settle(&env, cx, |_| calls.get() == 2);
        assert_eq!(count(cx), 1);
        assert!(matches!(state(cx, A), RemoteState::Saved(_)));
    }

    #[gpui_kit::test]
    fn at_most_four_fetches_run_at_once_and_the_rest_wait(cx: &mut TestAppContext) {
        let urls: Vec<String> = (0..6)
            .map(|i| format!("https://img.example.com/{i}.png"))
            .collect();
        let script: Vec<(&str, Hop)> = urls
            .iter()
            .map(|u| (u.as_str(), Hop::Body(PNG.to_vec())))
            .collect();
        let fake = Fake::gated(&script);
        let env = env(cx, &fake);
        let done = Rc::new(std::cell::Cell::new(0));
        {
            let _guard = env.rt().enter();
            for u in &urls {
                let d = done.clone();
                cx.update(|cx| store_of(cx).load(u, cx, move |_| d.set(d.get() + 1)));
            }
        }
        let gate = fake.gate.as_ref().unwrap();
        settle(&env, cx, |_| gate.entered.load(Ordering::SeqCst) == 4);
        // A few more rounds: nothing else starts while four run.
        for _ in 0..10 {
            settle(&env, cx, |_| true);
        }
        assert_eq!(gate.entered.load(Ordering::SeqCst), 4);
        for u in &urls {
            assert_eq!(state(cx, u), RemoteState::Loading);
        }
        gate.release();
        settle(&env, cx, |_| done.get() == 6);
        assert_eq!(count(cx), 6);
        for u in &urls {
            assert!(matches!(state(cx, u), RemoteState::Saved(_)));
        }
    }

    /// Reloads `url` and waits for its `on_done`.
    fn reload(env: &Env, cx: &mut TestAppContext, url: &str) {
        let done = Rc::new(std::cell::Cell::new(false));
        let d = done.clone();
        {
            let _guard = env.rt().enter();
            cx.update(|cx| store_of(cx).reload(url, cx, move |_| d.set(true)));
        }
        settle(env, cx, |_| done.get());
    }

    /// The bytes of the saved copy of `url`.
    fn saved_bytes(cx: &mut TestAppContext, url: &str) -> Vec<u8> {
        let RemoteState::Saved(path) = state(cx, url) else {
            panic!("expected a saved copy, got {:?}", state(cx, url));
        };
        std::fs::read(path).unwrap()
    }

    const JPEG: &[u8] = &[0xFF, 0xD8, 0xFF, 0xE0, 1, 2, 3];

    /// The old copy stays, and stays drawn, while the new bytes come. A
    /// second Reload in that time adds its call and starts nothing. The new
    /// bytes then replace the copy.
    #[gpui_kit::test]
    fn a_reload_replaces_the_copy_when_the_new_bytes_arrive(cx: &mut TestAppContext) {
        let fake = Fake::gated(&[(A, Hop::Body(PNG.to_vec()))]);
        let env = env(cx, &fake);
        let gate = fake.gate.as_ref().unwrap();
        gate.release();
        load(&env, cx, A);
        gate.close();
        fake.answer(A, Hop::Body(JPEG.to_vec()));
        let done = Rc::new(std::cell::Cell::new(0));
        {
            let _guard = env.rt().enter();
            for _ in 0..2 {
                let d = done.clone();
                cx.update(|cx| store_of(cx).reload(A, cx, move |_| d.set(d.get() + 1)));
            }
        }
        settle(&env, cx, |_| gate.entered.load(Ordering::SeqCst) == 2);
        assert_eq!(
            saved_bytes(cx, A),
            PNG,
            "the old copy stays during the fetch"
        );
        gate.release();
        settle(&env, cx, |_| done.get() == 2);
        assert_eq!(count(cx), 2, "the second Reload started nothing");
        assert_eq!(saved_bytes(cx, A), JPEG);
    }

    #[gpui_kit::test]
    fn an_offline_reload_keeps_the_saved_copy(cx: &mut TestAppContext) {
        let fake = Fake::new(&[(A, Hop::Body(PNG.to_vec()))]);
        let env = env(cx, &fake);
        load(&env, cx, A);
        fake.answer(A, Hop::Failed(FetchFailure::Offline));
        reload(&env, cx, A);
        assert_eq!(count(cx), 2);
        assert!(env.cache.find(A).is_some(), "the copy is still found");
        assert_eq!(saved_bytes(cx, A), PNG);
        // The failure is not kept: once the copy goes, the address shows
        // its placeholder, not a failure that the reader never saw.
        env.cache.remove(A).unwrap();
        assert_eq!(state(cx, A), host("img.example.com"));
    }

    #[gpui_kit::test]
    fn a_reload_with_a_non_image_body_keeps_the_old_copy(cx: &mut TestAppContext) {
        let fake = Fake::new(&[(A, Hop::Body(PNG.to_vec()))]);
        let env = env(cx, &fake);
        load(&env, cx, A);
        fake.answer(A, Hop::Body(b"<html>gone</html>".to_vec()));
        reload(&env, cx, A);
        assert_eq!(count(cx), 2);
        assert_eq!(saved_bytes(cx, A), PNG);
    }

    /// A Reload with no saved copy is a Load: the placeholder shows
    /// "Loading", then the copy is saved.
    #[gpui_kit::test]
    fn a_reload_with_no_saved_copy_loads_as_load_does(cx: &mut TestAppContext) {
        let fake = Fake::gated(&[(A, Hop::Body(PNG.to_vec()))]);
        let env = env(cx, &fake);
        let done = Rc::new(std::cell::Cell::new(false));
        let d = done.clone();
        {
            let _guard = env.rt().enter();
            cx.update(|cx| store_of(cx).reload(A, cx, move |_| d.set(true)));
        }
        assert_eq!(state(cx, A), RemoteState::Loading);
        fake.gate.as_ref().unwrap().release();
        settle(&env, cx, |_| done.get());
        assert_eq!(count(cx), 1);
        assert_eq!(saved_bytes(cx, A), PNG);
    }

    #[gpui_kit::test]
    fn clear_copy_returns_the_address_to_blocked(cx: &mut TestAppContext) {
        let fake = Fake::new(&[(A, Hop::Body(PNG.to_vec()))]);
        let env = env(cx, &fake);
        load(&env, cx, A);
        store(cx).clear_copy(A);
        assert_eq!(state(cx, A), host("img.example.com"));
        assert!(env.cache.find(A).is_none());
        assert_eq!(count(cx), 1);
    }

    #[gpui_kit::test]
    fn forget_sessions_drops_every_failure_and_redirect_permission(cx: &mut TestAppContext) {
        let fake = Fake::new(&[
            (A, Hop::Redirect("https://tracker.example.net/x.png".into())),
            (
                "https://tracker.example.net/x.png",
                Hop::Failed(FetchFailure::Status(503)),
            ),
        ]);
        let env = env(cx, &fake);
        load(&env, cx, A);
        store(cx).allow_redirect_once(A);
        load(&env, cx, A);
        assert_eq!(state(cx, A), RemoteState::Failed(FetchFailure::Status(503)));
        store(cx).forget_sessions();
        assert_eq!(state(cx, A), host("img.example.com"), "the failure went");
        load(&env, cx, A);
        assert_eq!(
            state(cx, A),
            RemoteState::RedirectBlocked {
                host: "tracker.example.net".into()
            },
            "the one-address permission went"
        );
    }

    /// A local HTTP/1.1 server on 127.0.0.1. `answer` gets the request head
    /// and returns the whole answer and whether to close the connection.
    /// It logs each request head and counts the connections.
    struct Server {
        base: String,
        heads: Arc<Mutex<Vec<String>>>,
        connections: Arc<AtomicUsize>,
    }

    fn serve(answer: fn(&str, &str) -> (Vec<u8>, bool)) -> Server {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let heads = Arc::new(Mutex::new(Vec::new()));
        let connections = Arc::new(AtomicUsize::new(0));
        let (h, c, b) = (heads.clone(), connections.clone(), base.clone());
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { return };
                c.fetch_add(1, Ordering::SeqCst);
                let (h, b) = (h.clone(), b.clone());
                std::thread::spawn(move || {
                    let mut reader = BufReader::new(stream.try_clone().unwrap());
                    let mut stream = stream;
                    loop {
                        let mut head = String::new();
                        loop {
                            let mut line = String::new();
                            match reader.read_line(&mut line) {
                                Ok(0) | Err(_) => return,
                                Ok(_) => {}
                            }
                            if line == "\r\n" {
                                break;
                            }
                            head.push_str(&line);
                        }
                        h.lock().unwrap().push(head.clone());
                        let (bytes, close) = answer(&head, &b);
                        if stream.write_all(&bytes).is_err() || close {
                            return;
                        }
                    }
                });
            }
        });
        Server {
            base,
            heads,
            connections,
        }
    }

    fn ok(body: &[u8], extra: &str) -> Vec<u8> {
        let mut out = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n{extra}\r\n",
            body.len()
        )
        .into_bytes();
        out.extend_from_slice(body);
        out
    }

    fn path_of(head: &str) -> &str {
        head.split_whitespace().nth(1).unwrap_or("")
    }

    #[test]
    fn the_real_fetcher_reuses_one_agent_for_every_fetch() {
        let server = serve(|head, _| match path_of(head) {
            "/a" => (
                b"HTTP/1.1 302 Found\r\nLocation: /b\r\nContent-Length: 0\r\n\r\n".to_vec(),
                false,
            ),
            _ => (ok(PNG, ""), false),
        });
        let fetcher = RealFetcher::new();
        assert_eq!(
            fetcher.get(&format!("{}/a", server.base)),
            Hop::Redirect("/b".into())
        );
        assert_eq!(
            fetcher.get(&format!("{}/b", server.base)),
            Hop::Body(PNG.to_vec())
        );
        assert_eq!(
            fetcher.get(&format!("{}/c", server.base)),
            Hop::Body(PNG.to_vec())
        );
        // One agent keeps one connection open for every request. A new
        // agent per fetch would open a new connection each time.
        assert_eq!(server.heads.lock().unwrap().len(), 3);
        assert_eq!(server.connections.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn the_real_fetcher_stops_a_body_over_the_limit_and_sends_no_cookie_or_referer() {
        let server = serve(|head, base| match path_of(head) {
            // No length: the body streams far past the limit.
            "/big" => {
                let mut out = b"HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n".to_vec();
                out.extend(std::iter::repeat_n(0u8, 4 * 1024 * 1024));
                (out, true)
            }
            "/long" => (
                b"HTTP/1.1 200 OK\r\nContent-Length: 999999999\r\n\r\n".to_vec(),
                true,
            ),
            "/cookie" => (ok(PNG, "Set-Cookie: session=secret\r\n"), false),
            "/r" => (
                format!("HTTP/1.1 301 Moved\r\nLocation: {base}/next\r\nContent-Length: 0\r\n\r\n")
                    .into_bytes(),
                false,
            ),
            _ => (ok(PNG, ""), false),
        });
        let fetcher = RealFetcher::with_limits(64 * 1024, WAIT_LIMIT, TOTAL_LIMIT);
        let url = |p: &str| format!("{}{p}", server.base);
        assert_eq!(
            fetcher.get(&url("/big")),
            Hop::Failed(FetchFailure::TooLarge)
        );
        assert_eq!(
            fetcher.get(&url("/long")),
            Hop::Failed(FetchFailure::TooLarge)
        );
        assert_eq!(fetcher.get(&url("/cookie")), Hop::Body(PNG.to_vec()));
        assert_eq!(fetcher.get(&url("/r")), Hop::Redirect(url("/next")));
        assert_eq!(fetcher.get(&url("/after")), Hop::Body(PNG.to_vec()));
        let heads = server.heads.lock().unwrap().clone();
        // The redirect was not followed.
        assert!(!heads.iter().any(|h| path_of(h) == "/next"), "{heads:?}");
        for head in &heads {
            let lower = head.to_ascii_lowercase();
            assert!(!lower.contains("\r\ncookie:"), "{head}");
            assert!(!lower.contains("\r\nreferer:"), "{head}");
            assert!(lower.contains("\r\nuser-agent: kimun\r\n"), "{head}");
        }
    }

    #[gpui_kit::test]
    fn a_chain_through_two_blocked_hosts_loads_after_one_load_for_each(cx: &mut TestAppContext) {
        let fake = Fake::new(&[
            (A, Hop::Redirect("https://b.example.net/x.png".into())),
            (
                "https://b.example.net/x.png",
                Hop::Redirect("https://c.example.org/y.png".into()),
            ),
            ("https://c.example.org/y.png", Hop::Body(PNG.to_vec())),
        ]);
        let env = env(cx, &fake);
        let blocked = |h: &str| RemoteState::RedirectBlocked { host: h.into() };
        load(&env, cx, A);
        assert_eq!(state(cx, A), blocked("b.example.net"));
        store(cx).allow_redirect_once(A);
        load(&env, cx, A);
        assert_eq!(state(cx, A), blocked("c.example.org"));
        store(cx).allow_redirect_once(A);
        load(&env, cx, A);
        assert!(matches!(state(cx, A), RemoteState::Saved(_)));
        // Retry drops both permissions.
        store(cx).retry(A);
        env.cache.remove(A).unwrap();
        load(&env, cx, A);
        assert_eq!(state(cx, A), blocked("b.example.net"));
        store(cx).allow_redirect_once(A);
        load(&env, cx, A);
        assert_eq!(state(cx, A), blocked("c.example.org"));
    }

    /// A fake that sleeps before each answer.
    struct Slow {
        inner: Arc<Fake>,
        delay: Duration,
    }

    impl Fetcher for Slow {
        fn get(&self, url: &str) -> Hop {
            std::thread::sleep(self.delay);
            self.inner.get(url)
        }
    }

    #[test]
    fn a_redirect_chain_past_the_total_limit_fails_before_the_next_hop() {
        let fake = Fake::new(&[
            (A, Hop::Redirect("/1".into())),
            ("https://img.example.com/1", Hop::Redirect("/2".into())),
            ("https://img.example.com/2", Hop::Body(PNG.to_vec())),
        ]);
        let dir = tempfile::tempdir().unwrap();
        let cache = ImageCache::at(SystemPath::try_absolute(dir.path()).unwrap());
        let fetches = Arc::new(AtomicUsize::new(0));
        let job = Job {
            fetcher: Arc::new(Slow {
                inner: fake.clone(),
                delay: Duration::from_millis(150),
            }),
            fetches: fetches.clone(),
            cache: Some(cache.clone()),
            allowed: AllowedHosts::default(),
            permits: Vec::new(),
            url: A.to_string(),
            total: Duration::from_millis(250),
        };
        assert!(matches!(job.run(), Outcome::Failed(FetchFailure::TimedOut)));
        // The second hop ends past the limit, so the third never starts.
        assert_eq!(fake.calls().len(), 2);
        assert_eq!(fetches.load(Ordering::SeqCst), 2);
        assert!(cache.find(A).is_none());
    }

    /// A server that accepts and reads the request, then calls `then` with
    /// the stream.
    fn serve_raw(then: fn(std::net::TcpStream)) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { return };
                std::thread::spawn(move || {
                    let mut reader = BufReader::new(stream.try_clone().unwrap());
                    let mut line = String::new();
                    while reader.read_line(&mut line).is_ok_and(|n| n > 0) && line != "\r\n" {
                        line.clear();
                    }
                    then(stream);
                });
            }
        });
        base
    }

    #[test]
    fn a_server_that_sends_nothing_fails_after_the_wait_limit() {
        let base = serve_raw(|_stream| std::thread::sleep(Duration::from_secs(5)));
        let fetcher = RealFetcher::with_limits(
            MAX_BYTES,
            Duration::from_millis(200),
            Duration::from_secs(30),
        );
        let start = Instant::now();
        assert_eq!(
            fetcher.get(&format!("{base}/a")),
            Hop::Failed(FetchFailure::TimedOut)
        );
        let took = start.elapsed();
        assert!(took < Duration::from_secs(2), "{took:?}");
    }

    #[test]
    fn a_server_that_trickles_fails_at_the_total_limit() {
        let base = serve_raw(|mut stream| {
            if stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 100000\r\n\r\n")
                .is_err()
            {
                return;
            }
            // One byte each 50 ms: every read is well inside the wait limit.
            for _ in 0..200 {
                if stream.write_all(&[0]).is_err() {
                    return;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
        });
        let fetcher = RealFetcher::with_limits(
            MAX_BYTES,
            Duration::from_millis(300),
            Duration::from_millis(600),
        );
        let start = Instant::now();
        assert_eq!(
            fetcher.get(&format!("{base}/a")),
            Hop::Failed(FetchFailure::TimedOut)
        );
        let took = start.elapsed();
        assert!(
            took >= Duration::from_millis(550) && took < Duration::from_secs(3),
            "{took:?}"
        );
    }

    #[test]
    fn the_failure_notes_are_one_line() {
        assert_eq!(FetchFailure::TimedOut.note(), "Load failed: timed out");
        assert_eq!(FetchFailure::Status(404).note(), "Load failed: HTTP 404");
        for f in [
            FetchFailure::Offline,
            FetchFailure::TooLarge,
            FetchFailure::NotAnImage,
            FetchFailure::Unsupported,
            FetchFailure::TooManyRedirects,
            FetchFailure::Other("x".into()),
        ] {
            assert!(f.note().starts_with("Load failed: ") && !f.note().contains('\n'));
        }
    }

    #[test]
    fn no_other_module_builds_a_client() {
        let needle = ["ureq", "::"].concat();
        let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut found = Vec::new();
        let mut dirs = vec![src];
        while let Some(dir) = dirs.pop() {
            for entry in std::fs::read_dir(dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    dirs.push(path);
                } else if path.extension().is_some_and(|e| e == "rs")
                    && path.file_name().is_some_and(|n| n != "remote_image.rs")
                    && std::fs::read_to_string(&path).unwrap().contains(&needle)
                {
                    found.push(path);
                }
            }
        }
        assert!(found.is_empty(), "{found:?}");
    }
}
