#![allow(non_snake_case)]

// use axum::Router;
use dioxus::prelude::*;
use kimun_core::nfs::VaultPath;

use crate::pages::editor::Editor;
use crate::pages::main::Main;
use crate::pages::settings::Settings;

/// An enum of all of the possible routes in the app.
#[derive(Clone, Routable, Debug, PartialEq)]
pub enum Route {
    #[route("/")]
    Main {},
    #[route("/edit/:note_path?:create")]
    Editor { note_path: VaultPath, create: bool },
    #[route("/settings")]
    Settings {},
}
