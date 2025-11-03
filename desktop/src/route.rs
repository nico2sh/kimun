#![allow(non_snake_case)]

// use axum::Router;
use dioxus::prelude::*;

use crate::pages::main_view::MainView;
use crate::pages::settings::Settings;
use crate::pages::start::Start;

/// An enum of all of the possible routes in the app.
#[derive(Clone, Routable, Debug, PartialEq)]
pub enum Route {
    #[route("/")]
    Start {},
    #[route("/edit/:encoded_path?:create")]
    MainView { encoded_path: String, create: bool },
    #[route("/settings")]
    Settings {},
}
