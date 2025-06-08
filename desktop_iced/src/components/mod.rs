use iced::{Element, Task};

use crate::KimunMessage;

pub mod easing;
pub mod filtered_list;
pub mod linear_progress;
pub mod list;

pub trait KimunComponent {
    fn update(&mut self, message: KimunMessage) -> Task<KimunMessage>;
    fn view(&self) -> Element<KimunMessage>;
    fn key_press(
        &self,
        key: &iced::keyboard::Key,
        modifiers: &iced::keyboard::Modifiers,
    ) -> Task<KimunMessage>;
}

pub trait KimunListElement: std::fmt::Debug + Clone {
    fn get_view(&self) -> Element<KimunMessage>;
    fn get_height(&self) -> f32;
    fn on_select(&self) -> Task<KimunMessage>;
}
