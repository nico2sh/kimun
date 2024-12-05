pub mod row;

use std::sync::mpsc::{Receiver, Sender};

use iced::{
    widget::{column, container, keyed_column, text_input},
    Length, Task,
};
use nucleo::{
    pattern::{CaseMatching, Normalization, Pattern},
    Config, Matcher,
};
use row::RowItem;

use super::{Message, NoteTakerView};

#[derive(Clone, Debug)]
pub enum FilterAction {
    TriggerFilter(String),
    ApplyFilter,
}

pub struct FilteredList<R>
where
    R: RowItem + 'static,
{
    main_message_bus: iced::futures::channel::mpsc::Sender<Message>,
    pub filter_text: String,
    elements: Vec<R>,
    filter_tx: Sender<Vec<R>>,
    filter_rx: Receiver<Vec<R>>,
    filtered_elements: Vec<R>,
}

impl<R> FilteredList<R>
where
    R: RowItem,
{
    pub fn new(main_sender: iced::futures::channel::mpsc::Sender<Message>) -> Self {
        let elements = vec![];
        let filtered_elements = vec![];
        let (filter_tx, filter_rx) = std::sync::mpsc::channel();
        Self {
            main_message_bus: main_sender,
            filter_text: String::new(),
            filter_tx,
            filter_rx,
            elements,
            filtered_elements,
        }
    }

    fn apply_filter(&mut self) {
        let elements = self.elements.clone();
        let filter_text = self.filter_text.clone();
        let tx = self.filter_tx.clone();
        let mut mmb = self.main_message_bus.clone();
        std::thread::spawn(move || {
            let mut matcher = Matcher::new(Config::DEFAULT);
            let mut res = Pattern::parse(&filter_text, CaseMatching::Ignore, Normalization::Smart)
                .match_list(elements.iter(), &mut matcher)
                .iter()
                .map(|e| e.0.to_owned())
                .collect::<Vec<R>>();
            res.sort_by_key(|a| a.get_sort_string());
            tx.send(res).unwrap();
            mmb.try_send(Message::FilterAction(FilterAction::ApplyFilter))
                .expect("Error sending event of applying the filter");
        });
    }

    pub fn add_element(&mut self, element: R) {
        let element = element.to_owned();
        self.elements.push(element);
        self.apply_filter();
    }
}

impl<R> NoteTakerView<FilterAction> for FilteredList<R>
where
    R: RowItem,
{
    fn get_view(&self) -> iced::Element<Message> {
        let text_filter = text_input("Type something", &self.filter_text)
            .on_input(|filter| Message::FilterAction(FilterAction::TriggerFilter(filter)));

        let elements = &self.filtered_elements;
        let rows = elements.iter().enumerate().map(|(i, e)| {
            let row_element = e.get_view();
            (i, row_element)
        });
        let list = keyed_column(rows).padding(5);

        container(column![text_filter, list].spacing(10))
            .width(Length::Fill)
            .padding(10)
            .into()
    }

    fn update(&mut self, message: FilterAction) -> Task<Message> {
        match message {
            FilterAction::TriggerFilter(filter) => {
                self.filter_text = filter;
                self.apply_filter();
            }
            FilterAction::ApplyFilter => {
                while let Ok(filtered) = self.filter_rx.try_recv() {
                    self.filtered_elements = filtered;
                }
            }
        };

        Task::none()
    }

    fn subscription(&self) {}
}
