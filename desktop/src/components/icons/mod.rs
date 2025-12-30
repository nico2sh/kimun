use dioxus::prelude::*;

#[component]
pub fn DoubleCircle() -> Element {
    rsx! {
        svg { view_box: "0 0 24 24",
            circle { cx: "12", cy: "12", r: "10" }
            circle {
                cx: "12",
                cy: "12",
                r: "3",
                fill: "currentColor",
            }
        }
    }
}

#[component]
pub fn FatArrowRight() -> Element {
    rsx! {
        svg { view_box: "0 0 24 24",
            polygon { points: "4,10 4,14 20,12", fill: "currentColor" }
            polygon { points: "12,7 20,12 12,17", fill: "currentColor" }
        }
    }
}

#[component]
pub fn SortTitle() -> Element {
    rsx! {
        svg { view_box: "0 0 24 24",
            path { d: "M4 7V4h16v3M9 20h6M12 4v16" }
        }
    }
}

#[component]
pub fn SortFileName() -> Element {
    rsx! {
        svg { view_box: "0 0 24 24",
            path { d: "M13 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V9z" }
            polyline { points: "13 2 13 9 20 9" }
        }
    }
}
