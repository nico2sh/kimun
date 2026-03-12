use std::rc::Rc;

use color_eyre::eyre;
use kimun_core::NoteVault;

use crate::{
    app_screen::{AppScreen, start::StartScreen},
    settings::AppSettings,
};

pub struct App {
    pub key_input: String,                  // the currently being edited json key.
    pub value_input: String,                // the currently being edited json value.
    pub current_screen: Box<dyn AppScreen>, // the current screen the user is looking at, and will later determine what is rendered.
    pub settings: AppSettings,
}

impl App {
    pub fn new() -> eyre::Result<Self> {
        let settings = AppSettings::load_from_disk()?;
        Ok(Self {
            key_input: String::new(),
            value_input: String::new(),
            current_screen: Box::new(StartScreen {}),
            settings,
        })
    }
}
