//! Bearer-token auth. When a token is configured, every `/api` request must
//! present `Authorization: Bearer <token>`. When none is configured (localhost
//! dev), the server is open. `/health` is never gated so liveness probes work.
//!
//! The web UI's session-cookie half lives in [`session`] — two mechanisms,
//! one token source.

pub(crate) mod session;

use std::sync::Arc;

use axum::{
    extract::{Request, State},
    http::{StatusCode, header::AUTHORIZATION},
    middleware::Next,
    response::Response,
};

use crate::server_state::AppState;

/// Whether a request is authorized given the configured token and the request's
/// `Authorization` header value. No configured token → always authorized.
pub fn is_authorized(expected: Option<&str>, auth_header: Option<&str>) -> bool {
    match expected {
        None => true,
        Some(expected) => auth_header
            .and_then(|h| {
                // Auth-scheme is case-insensitive (RFC 7235); the token is not.
                let (scheme, token) = h.split_once(' ')?;
                scheme.eq_ignore_ascii_case("Bearer").then_some(token)
            })
            .map(|token| constant_time_eq(token.as_bytes(), expected.as_bytes()))
            .unwrap_or(false),
    }
}

/// Length-independent-branch comparison so a wrong token can't be recovered by
/// timing. (Short-circuiting `==` on the token would leak a match prefix.)
pub(crate) fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b) {
        diff |= x ^ y;
    }
    diff == 0
}

/// Axum middleware enforcing [`is_authorized`] on the routes it wraps.
pub async fn auth_middleware(
    State(state): State<Arc<AppState>>,
    request: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    let header = request
        .headers()
        .get(AUTHORIZATION)
        .and_then(|v| v.to_str().ok());
    if is_authorized(state.config.auth.token.as_deref(), header) {
        Ok(next.run(request).await)
    } else {
        Err(StatusCode::UNAUTHORIZED)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_configured_token_is_open() {
        assert!(is_authorized(None, None));
        assert!(is_authorized(None, Some("Bearer whatever")));
    }

    #[test]
    fn correct_token_authorizes() {
        assert!(is_authorized(Some("secret"), Some("Bearer secret")));
        // Scheme is case-insensitive.
        assert!(is_authorized(Some("secret"), Some("bearer secret")));
    }

    #[test]
    fn wrong_missing_or_malformed_token_rejected() {
        assert!(!is_authorized(Some("secret"), Some("Bearer nope")));
        assert!(!is_authorized(Some("secret"), None));
        assert!(!is_authorized(Some("secret"), Some("secret"))); // no "Bearer " prefix
        assert!(!is_authorized(Some("secret"), Some("Bearer secre"))); // length differs
    }
}
