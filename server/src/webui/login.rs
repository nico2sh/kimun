//! Login and logout: the form that exchanges the bearer token for the
//! `HttpOnly` session cookie, and the handler that clears it.

use std::sync::Arc;

use axum::{
    Form,
    extract::State,
    http::HeaderMap,
    response::{IntoResponse, Redirect, Response},
};
use maud::{DOCTYPE, Markup, html};
use serde::Deserialize;

use crate::auth::session::{
    clear_session_cookie, redirect_with_cookie, same_origin, set_session_cookie,
};
use crate::server_state::AppState;

use super::shell::styles;

pub(super) async fn login_page(State(state): State<Arc<AppState>>) -> Response {
    // No token configured → nothing to log into.
    if state.config.auth.token.is_none() {
        return Redirect::to("/").into_response();
    }
    login_markup(false).into_response()
}

#[derive(Deserialize)]
pub(super) struct LoginForm {
    token: String,
}

pub(super) async fn login_submit(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Form(form): Form<LoginForm>,
) -> Response {
    if !same_origin(&headers) {
        return axum::http::StatusCode::FORBIDDEN.into_response();
    }
    match state.config.auth.token.as_deref() {
        Some(expected)
            if crate::auth::constant_time_eq(form.token.as_bytes(), expected.as_bytes()) =>
        {
            redirect_with_cookie("/", set_session_cookie(expected))
        }
        Some(_) => login_markup(true).into_response(),
        None => Redirect::to("/").into_response(),
    }
}

pub(super) async fn logout() -> Response {
    redirect_with_cookie("/login", clear_session_cookie())
}

fn login_markup(error: bool) -> Markup {
    html! {
        (DOCTYPE)
        html {
            head {
                meta charset="utf-8";
                title { "Kimün RAG — Sign in" }
                link rel="icon" type="image/png" href="/assets/img/kimun.png";
                (styles())
            }
            body {
                main .login {
                    img .logo-lg src="/assets/img/kimun.png" alt="" width="40" height="40";
                    h1 { "Kimün RAG" }
                    @if error { p .flash.err { "Incorrect token." } }
                    form method="post" action="/login" {
                        label { "Server token" }
                        input type="password" name="token" autofocus?;
                        button type="submit" { "Sign in" }
                    }
                }
            }
        }
    }
}
