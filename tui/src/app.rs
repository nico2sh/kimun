use color_eyre::eyre;

use crate::{
    app_screen::{AppScreen, start::StartScreen},
    settings::AppSettings,
};

pub struct App {
    /// The currently active screen. Held as `Option` so we can temporarily
    /// `take()` it when calling screen methods (avoids double-borrow of `App`).
    pub current_screen: Option<Box<dyn AppScreen>>,

    pub settings: AppSettings,
}

impl App {
    pub fn new() -> eyre::Result<Self> {
        let settings = AppSettings::load_from_disk()?;
        Ok(Self {
            current_screen: Some(Box::new(StartScreen::new(settings.clone()))),
            settings,
        })
    }
}
