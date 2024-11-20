use dioxus::prelude::*;

use crate::noters::nfs::NotePath;

#[allow(non_snake_case)]
pub fn RowElement<R: RowItem>(item: R) -> Element {
    rsx! {
        //
    }
}

pub trait RowItem: Eq {}

impl RowItem for NotePath {}
