pub mod row;
mod selector;

use core::f32;
use std::sync::mpsc::{Receiver, Sender};

use eframe::egui;

use log::{debug, info};
use row::{RowItem, RowMessage};
use selector::Selector;

use super::{Message, View};

pub const ID_SEARCH: &str = "Search Popup";
pub const ID_POPUP_OPEN: &str = "Popup Open";

pub struct FilteredList<R>
where
    R: RowItem + 'static,
{
    filter_text: String,
    selector: Selector<R>,
    rx: Option<Receiver<R>>,
    to_clear: bool,
    requested_focus: bool,
}

impl<R> FilteredList<R>
where
    R: RowItem + 'static,
{
    pub fn new(elements: Vec<R>) -> Self {
        let selector = Selector::new(elements);
        // let (tx, rx) = std::sync::mpsc::channel();

        Self {
            filter_text: String::new(),
            selector,
            rx: None,
            to_clear: false,
            requested_focus: false,
        }
    }

    // Provides a channel to send rows
    // for populating the selection list
    pub fn get_channel_rows(&mut self) -> Sender<R> {
        // We create new channels
        let (tx, rx) = std::sync::mpsc::channel();

        self.rx = Some(rx);
        tx
    }

    pub fn clear(&mut self) {
        self.to_clear = true;
    }

    pub fn request_focus(&mut self) {
        self.requested_focus = true;
    }

    fn update_filter(&mut self) {
        if let Some(rx) = &self.rx {
            let trigger_filter = if let Ok(row) = rx.try_recv() {
                info!("adding to list {}", row.as_ref());
                self.selector.add_element(row);
                true
            } else {
                false
            };
            while let Ok(row) = rx.try_recv() {
                self.selector.add_element(row);
            }
            if trigger_filter {
                self.selector.filter_content(&self.filter_text);
            }
        }
    }
}

impl<R> View for FilteredList<R>
where
    R: RowItem + 'static,
{
    fn view(&mut self, ui: &mut egui::Ui) -> Message {
        if self.to_clear {
            self.selector.clear();
            self.to_clear = false;
        }

        self.update_filter();

        let window = egui::Window::new("")
            .id(egui::Id::new(ID_POPUP_OPEN)) // required since we change the title
            .resizable(false)
            .constrain(true)
            .collapsible(false)
            .title_bar(false)
            .scroll(false)
            .enabled(true)
            .anchor(egui::Align2::CENTER_TOP, egui::Vec2::new(0.0, 100.0));

        window.show(ui.ctx(), |ui| {
            let _text_height = egui::TextStyle::Body
                .resolve(ui.style())
                .size
                .max(ui.spacing().interact_size.y);
            let _available_height = ui.available_height();

            ui.with_layout(
                egui::Layout {
                    main_dir: egui::Direction::TopDown,
                    main_wrap: false,
                    main_align: egui::Align::Center,
                    main_justify: false,
                    cross_align: egui::Align::Min,
                    cross_justify: false,
                },
                |ui| {
                    let response = ui.add(
                        egui::TextEdit::singleline(&mut self.filter_text)
                            .desired_width(f32::INFINITY)
                            .id(ID_SEARCH.into()),
                    );

                    let mut selected = self.selector.get_selected();
                    egui::scroll_area::ScrollArea::vertical()
                        .auto_shrink(true)
                        .show(ui, |ui| {
                            self.selector.get_elements().iter().enumerate().for_each(
                                |(pos, element)| {
                                    let mut frame = egui::Frame {
                                        inner_margin: egui::Margin::same(6.0),
                                        ..Default::default()
                                    }
                                    .begin(ui);
                                    {
                                        // everything here in their own scope
                                        let response = element.get_label(&mut frame.content_ui);
                                        if response.hovered() {
                                            selected = Some(pos);
                                        }
                                        if Some(pos) == selected {
                                            response.highlight();
                                            // frame.frame.fill = egui::Color32::LIGHT_GRAY;
                                        }
                                    }
                                    ui.allocate_space(ui.available_size());
                                    frame.allocate_space(ui);

                                    frame.paint(ui);
                                },
                            )
                        });
                    self.selector.set_selected(selected);

                    if response.changed() {
                        self.selector.filter_content(&self.filter_text);
                    }
                },
            );
        });

        if self.requested_focus {
            ui.ctx()
                .memory_mut(|mem| mem.request_focus(ID_SEARCH.into()));
            self.requested_focus = false;
        }

        if ui.ctx().input(|i| i.key_pressed(egui::Key::ArrowUp)) {
            self.selector.select_prev();
        }
        if ui.ctx().input(|i| i.key_pressed(egui::Key::ArrowDown)) {
            self.selector.select_next();
        }

        if ui.ctx().input(|i| i.key_pressed(egui::Key::Enter)) {
            let row_message = if let Some(selected) = self.selector.get_selection() {
                selected.get_message()
            } else {
                RowMessage::Nothing
            };
            Message::SelectionMessage(row_message)
        } else if ui.ctx().input(|i| i.key_pressed(egui::Key::Escape)) {
            Message::CloseWindow
        } else {
            Message::None
        }
    }
}
