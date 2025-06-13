// The dioxus prelude contains a ton of common items used in dioxus apps. It's a good idea to import wherever you
// need dioxus
use dioxus::prelude::*;

use route::Route;
use settings::AppSettings;

/// Define a components module that contains all shared components for our app.
mod components;
mod pages;
mod route;
mod settings;

// The asset macro also minifies some assets like CSS and JS to make bundled smaller
// const MAIN_CSS: Asset = asset!("/assets/styling/main.css");
// Urls are relative to your Cargo.toml file
const THEME: Asset = asset!("/assets/styling/gruvbox.css");
const FONTS: Asset = asset!("/assets/styling/fonts.css");
const ICONS: Asset = asset!("/assets/styling/icons.css");
const STYLE: Asset = asset!("/assets/styling/main.css");

fn main() {
    // The `launch` function is the main entry point for a dioxus app. It takes a component and renders it with the platform feature
    // you have enabled
    dioxus::launch(App);
}

/// App is the main component of our app. Components are the building blocks of dioxus apps. Each component is a function
/// that takes some props and returns an Element. In this case, App takes no props because it is the root of our app.
///
/// Components should be annotated with `#[component]` to support props, better error messages, and autocomplete
#[component]
fn App() -> Element {
    let app_settings = AppSettings::load_from_disk().unwrap();
    use_context_provider(move || Signal::new(app_settings));

    // The `rsx!` macro lets us define HTML inside of rust. It expands to an Element with all of our HTML inside.
    rsx! {
        // In addition to element and text (which we will see later), rsx can contain other components. In this case,
        // we are using the `document::Link` component to add a link to our favicon and main CSS file into the head of our app.
        document::Link { rel: "stylesheet", href: THEME }
        document::Link { rel: "stylesheet", href: FONTS }
        document::Link { rel: "stylesheet", href: ICONS }
        document::Link { rel: "stylesheet", href: STYLE }

        Router::<Route> {}
    }
}
