use dioxus::prelude::*;

const THEME_LIGHT: Asset = asset!("/assets/styling/light.css");

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct Theme {
    pub css: String,
    pub name: String,
}

impl Default for Theme {
    fn default() -> Self {
        Self {
            css: THEME_LIGHT.to_string(),
            name: "Light".to_string(),
        }
    }
}

impl Theme {
    pub fn new<S: AsRef<str>, T: AsRef<str>>(css: S, name: T) -> Self {
        Self {
            css: css.as_ref().to_string(),
            name: name.as_ref().to_string(),
        }
    }
}
