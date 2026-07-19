//! Session-cookie auth for the web UI: a login form exchanges the bearer token
//! for an `HttpOnly` session cookie holding that same shared secret (hashed).
//! With no token configured the UI is open (matching the API's localhost-dev
//! posture).

use std::sync::Arc;

use axum::{
    extract::{Request, State},
    http::{
        HeaderMap, HeaderValue,
        header::{COOKIE, HOST, ORIGIN, SET_COOKIE},
    },
    middleware::Next,
    response::{IntoResponse, Redirect, Response},
};
use sha2::{Digest, Sha256};

use crate::server_state::AppState;

pub(crate) const SESSION_COOKIE: &str = "kimun_session";

/// The session-cookie value for a token: the token's SHA-256 as hex. Keeping the
/// hash (not the token) in the cookie means the value is always cookie-safe
/// (`0-9a-f`, so a token with spaces/`;`/control chars can't corrupt the cookie
/// or lock the admin out), and a leaked cookie doesn't hand over the raw API
/// secret.
pub(crate) fn session_value(token: &str) -> String {
    let digest = Sha256::digest(token.as_bytes());
    let mut hex = String::with_capacity(digest.len() * 2);
    for b in digest {
        use std::fmt::Write;
        let _ = write!(hex, "{b:02x}");
    }
    hex
}

/// The `Set-Cookie` value establishing a session for `token`: the hashed
/// session value plus the attributes the whole session contract relies on.
/// The cookie holds the token's hash, not the token — always cookie-safe, and
/// `HttpOnly` keeps it out of page scripts; `SameSite=Strict` backs up the
/// same-origin POST guard; `Path=/` must match [`clear_session_cookie`] or
/// logout would silently fail to delete it.
pub(crate) fn set_session_cookie(token: &str) -> String {
    format!(
        "{SESSION_COOKIE}={}; HttpOnly; SameSite=Strict; Path=/",
        session_value(token)
    )
}

/// The `Set-Cookie` value clearing the session (logout). Carries the same
/// `Path` as [`set_session_cookie`] — a mismatched path is a different cookie
/// to the browser and nothing gets deleted.
pub(crate) fn clear_session_cookie() -> String {
    format!("{SESSION_COOKIE}=; Max-Age=0; Path=/")
}

/// Rejects a state-changing POST a browser marks as cross-origin. A same-origin
/// form POST (or a non-browser client like curl/tests) sends no mismatching
/// `Origin`, so it passes; a drive-by CSRF from another site is blocked even in
/// open mode (no token, where there's no SameSite cookie to lean on).
pub(crate) fn same_origin(headers: &HeaderMap) -> bool {
    let Some(origin) = headers.get(ORIGIN).and_then(|v| v.to_str().ok()) else {
        return true; // no Origin → non-browser or not cross-site; allow
    };
    let origin_host = origin.split("://").nth(1).unwrap_or(origin);
    let host = headers
        .get(HOST)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    origin_host == host
}

/// Rejects cross-origin state-changing requests on every route it wraps: any
/// non-GET/HEAD request must pass [`same_origin`]. Layered on the protected
/// router so a new mutating route cannot ship without CSRF protection — the
/// guard is structural, not a per-handler convention. The login POST sits
/// outside the protected router and keeps its inline check.
pub(crate) async fn csrf_guard(req: Request, next: Next) -> Response {
    use axum::http::{Method, StatusCode};
    let method = req.method();
    if method != Method::GET && method != Method::HEAD && !same_origin(req.headers()) {
        return StatusCode::FORBIDDEN.into_response();
    }
    next.run(req).await
}

/// Gates every protected page. Open when no token is configured; otherwise the
/// session cookie must carry the configured token. Unauthorized → `/login`.
pub(crate) async fn web_auth(
    State(state): State<Arc<AppState>>,
    req: Request,
    next: Next,
) -> Response {
    let Some(expected) = state.config.auth.token.as_deref() else {
        return next.run(req).await;
    };
    let want = session_value(expected);
    let ok = cookie_value(&req, SESSION_COOKIE)
        .map(|v| crate::auth::constant_time_eq(v.as_bytes(), want.as_bytes()))
        .unwrap_or(false);
    if ok {
        next.run(req).await
    } else {
        Redirect::to("/login").into_response()
    }
}

fn cookie_value(req: &Request, name: &str) -> Option<String> {
    let header = req.headers().get(COOKIE)?.to_str().ok()?;
    header.split(';').find_map(|kv| {
        let (k, v) = kv.trim().split_once('=')?;
        (k == name).then(|| v.to_string())
    })
}

pub(crate) fn redirect_with_cookie(location: &str, cookie: String) -> Response {
    let mut resp = Redirect::to(location).into_response();
    if let Ok(val) = HeaderValue::from_str(&cookie) {
        resp.headers_mut().insert(SET_COOKIE, val);
    }
    resp
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;

    #[test]
    fn session_value_is_hex_sha256_of_the_token() {
        // Known SHA-256 vector: sha256("abc").
        assert_eq!(
            session_value("abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn session_value_shape_determinism_and_distinctness() {
        let cases = ["secret", "", "a b;c", "🦀"];
        for token in cases {
            let v = session_value(token);
            assert_eq!(v.len(), 64, "64 hex chars for {token:?}");
            assert!(
                v.chars()
                    .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
            );
            assert_eq!(v, session_value(token), "deterministic for {token:?}");
        }
        assert_ne!(session_value("secret"), session_value("secrets"));
    }

    fn headers(pairs: &[(axum::http::HeaderName, &str)]) -> HeaderMap {
        let mut map = HeaderMap::new();
        for (name, value) in pairs {
            map.insert(name.clone(), HeaderValue::from_str(value).unwrap());
        }
        map
    }

    #[test]
    fn same_origin_table() {
        let cases: &[(&str, HeaderMap, bool)] = &[
            (
                "no Origin header",
                headers(&[(HOST, "localhost:7573")]),
                true,
            ),
            (
                "matching origin and host",
                headers(&[(ORIGIN, "http://localhost:7573"), (HOST, "localhost:7573")]),
                true,
            ),
            (
                "scheme stripped before comparing",
                headers(&[(ORIGIN, "https://example.com"), (HOST, "example.com")]),
                true,
            ),
            (
                "mismatching origin",
                headers(&[(ORIGIN, "http://evil.example"), (HOST, "localhost:7573")]),
                false,
            ),
            (
                "origin present but no Host",
                headers(&[(ORIGIN, "http://localhost:7573")]),
                false,
            ),
        ];
        for (case, map, expected) in cases {
            assert_eq!(same_origin(map), *expected, "{case}");
        }
    }

    fn request_with_cookies(cookie_header: Option<&str>) -> Request {
        let builder = Request::builder().uri("/");
        let builder = match cookie_header {
            Some(v) => builder.header(COOKIE, v),
            None => builder,
        };
        builder.body(Body::empty()).unwrap()
    }

    #[test]
    fn cookie_value_table() {
        let cases: &[(&str, Option<&str>, Option<&str>)] = &[
            (
                "finds named cookie among several",
                Some("a=1; kimun_session=abc123; b=2"),
                Some("abc123"),
            ),
            (
                "trims spaces around pairs",
                Some("  kimun_session=xyz ; other=1"),
                Some("xyz"),
            ),
            ("absent cookie", Some("a=1; b=2"), None),
            ("no Cookie header at all", None, None),
        ];
        for (case, header, expected) in cases {
            let req = request_with_cookies(*header);
            assert_eq!(
                cookie_value(&req, SESSION_COOKIE).as_deref(),
                *expected,
                "{case}"
            );
        }
    }
}
