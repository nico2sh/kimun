use core_notes::nfs::NotePath;
use iced::Element;

use crate::Message;

pub struct Row<'a> {
    view: Element<'a, Message>,
}

pub trait RowItem: AsRef<str> + Send + Sync + Clone + std::fmt::Debug {
    fn get_view(&self) -> Element<Message>;
    fn get_sort_string(&self) -> String;
    fn get_message(&self) -> RowMessage;
}

#[derive(PartialEq, Eq, Debug)]
pub enum RowMessage {
    Nothing,
    OpenNote(NotePath),
    OpenDirectory(NotePath),
}
