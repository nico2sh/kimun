use dioxus::prelude::*;
use log::info;

use core_notes::nfs::NoteEntry;

pub trait RowItem: PartialEq + Eq + Clone {
    fn on_select(&self) -> Box<dyn FnMut()>;
    fn get_view(&self) -> Element;
}

impl RowItem for NoteEntry {
    fn on_select(&self) -> Box<dyn FnMut()> {
        Box::new(|| info!("Selected"))
    }

    fn get_view(&self) -> Element {
        rsx! {
            div {
                "{self.to_string()}"
            }
        }
    }
}
