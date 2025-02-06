use std::{collections::VecDeque, sync::Arc};

use crossbeam_channel::{Receiver, Sender};
use eframe::egui::{self, Widget};
use log::{debug, error, info};

use super::{EditorMessage, EditorModal};

const ID_SEARCH: &str = "Search Popup";

#[derive(Debug)]
enum StateMessage<P, D>
where
    D: ListElement + 'static,
    P: Send + Sync + Clone + 'static,
{
    Initializing,
    Initialized { provider: P },
    Filtering,
    Filtered { filter: String, data: Vec<D> },
    Ready { filter: String },
}

impl<P, D> std::fmt::Display for StateMessage<P, D>
where
    P: Send + Sync + Clone + 'static,
    D: ListElement + 'static,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StateMessage::Initializing => write!(f, "Initializing"),
            StateMessage::Initialized { provider: _ } => write!(f, "Initialized"),
            StateMessage::Filtering => write!(f, "Filtering"),
            StateMessage::Filtered { filter, data: _ } => {
                write!(f, "Filtered with filter `{}`", filter)
            }
            StateMessage::Ready { filter } => {
                write!(f, "Ready with filter `{}`", filter)
            }
        }
    }
}

pub trait FilteredListFunctions<P, D>: Clone + Send + Sync
where
    D: ListElement,
{
    fn init(&self) -> P;
    fn filter<S: AsRef<str>>(&self, filter_text: S, provider: &P) -> Vec<D>;
    fn on_entry(&self, element: &D) -> Option<FilteredListFunctionMessage<Self>>;
    fn header_element(&self, state_data: &StateData<D>) -> Option<D>;
}

pub enum FilteredListFunctionMessage<F> {
    ToEditor(EditorMessage),
    ResetState(F),
}

pub struct FilteredList<F, P, D>
where
    F: FilteredListFunctions<P, D> + 'static,
    P: Send + Sync + Clone + 'static,
    D: ListElement + 'static,
{
    state_manager: SelectorStateManager<F, P, D>,
    requested_focus: bool,
    requested_scroll: bool,
}

impl<F, P, D> FilteredList<F, P, D>
where
    F: FilteredListFunctions<P, D> + 'static,
    P: Send + Sync + Clone + 'static,
    D: ListElement + 'static,
{
    pub fn new(functions: F) -> Self {
        let mut state_manager = SelectorStateManager::new(functions);
        state_manager.initialize();
        Self {
            state_manager,
            requested_focus: true,
            requested_scroll: false,
        }
    }

    pub fn request_focus(&mut self) {
        self.requested_focus = true;
    }

    fn select(&mut self, selected: &D) -> Option<EditorMessage> {
        if let Some(message) = self.state_manager.functions.on_entry(selected) {
            match message {
                FilteredListFunctionMessage::ToEditor(editor_message) => Some(editor_message),
                FilteredListFunctionMessage::ResetState(functions) => {
                    self.state_manager.functions = Arc::new(functions);
                    self.request_focus();
                    if let Err(e) = self.state_manager.tx.send(StateMessage::Initializing) {
                        error!("Can't reset the state, Err: {}", e)
                    }
                    None
                }
            }
        } else {
            None
        }
    }

    pub fn get_selection(&self) -> Option<D> {
        self.state_manager.state_data.get_selection()
    }

    fn get_header(&self) -> Option<D> {
        self.state_manager
            .functions
            .header_element(&self.state_manager.state_data)
    }

    fn get_table(&mut self, ui: &mut egui::Ui, selected_element: &mut Option<D>) {
        let header = self.get_header();
        let text_height = egui::TextStyle::Body
            .resolve(ui.style())
            .size
            .max(ui.spacing().interact_size.y);
        if let Some(element) = &header {
            // let height = text_height * element.get_height_mult();
            let header_resp = ui.add_sized(
                [
                    ui.available_width(),
                    text_height * element.get_height_mult(),
                ],
                egui::Button::new(element.get_label()).shortcut_text(element.get_icon()),
            );
            if header_resp.clicked() {
                *selected_element = Some(element.clone());
            }
            ui.separator();
        }

        let mut selected = self.state_manager.state_data.get_selected();
        let available_height = ui.available_height();
        let mut table = egui_extras::TableBuilder::new(ui)
            .striped(true)
            .resizable(false)
            .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
            .column(egui_extras::Column::auto())
            .column(egui_extras::Column::remainder())
            .min_scrolled_height(400.0)
            .max_scroll_height(available_height)
            .sense(egui::Sense::click());

        if let Some(selected) = selected {
            table = table.scroll_to_row(selected, None);
        }
        table.body(|mut body| {
            let elements = self.state_manager.state_data.get_elements();
            for element in elements.iter() {
                let height = text_height * element.get_height_mult();
                body.row(height, |mut row| {
                    row.set_hovered(selected.map_or_else(|| false, |s| s == row.index()));
                    row.col(|ui| {
                        egui::Label::new(element.get_icon())
                            .selectable(false)
                            .ui(ui);
                    });

                    row.col(|ui| {
                        egui::Label::new(element.get_label())
                            .selectable(false)
                            .ui(ui);
                    });
                    if row.response().clicked() {
                        *selected_element = Some(element.clone());
                    }
                    if row.response().hovered() {
                        selected = Some(row.index());
                    }
                });
            }
        });
        self.state_manager.state_data.set_selected(selected);
    }
}

impl<F, P, D> EditorModal for FilteredList<F, P, D>
where
    F: FilteredListFunctions<P, D>,
    P: Send + Sync + Clone + 'static,
    D: ListElement + 'static,
{
    fn update(&mut self, ui: &mut egui::Ui) -> Option<EditorMessage> {
        self.state_manager.update();
        let mut selected_element = None;

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
                ui.horizontal(|ui| {
                    // Fantastic solution from here to have a right sided button
                    // https://github.com/emilk/egui/discussions/3908#discussioncomment-8270353
                    let id_filter_target_size = egui::Id::new("filter_target_size");
                    let this_init_max_width = ui.max_rect().width();
                    let last_others_width = ui.data(|data| {
                        data.get_temp(id_filter_target_size)
                            .unwrap_or(this_init_max_width)
                    });
                    let filter_target_width = this_init_max_width - last_others_width;

                    ui.add(
                        egui::TextEdit::singleline(&mut self.state_manager.state_data.filter_text)
                            .desired_width(filter_target_width)
                            .id(ID_SEARCH.into()),
                    );
                    let _sort_button = ui.button("S");

                    ui.data_mut(|data| {
                        data.insert_temp(
                            id_filter_target_size,
                            ui.min_rect().width() - filter_target_width,
                        )
                    });
                });

                ui.separator();

                // let body_text_size = egui::TextStyle::Body.resolve(ui.style()).size;
                egui_extras::StripBuilder::new(ui)
                    .size(egui_extras::Size::remainder().at_least(100.0))
                    // .size(egui_extras::Size::exact(body_text_size))
                    .vertical(|mut strip| {
                        strip.cell(|ui| {
                            egui::ScrollArea::horizontal().show(ui, |ui| {
                                self.get_table(ui, &mut selected_element);
                            });
                        })
                    });
            },
        );

        if self.requested_focus {
            ui.ctx()
                .memory_mut(|mem| mem.request_focus(ID_SEARCH.into()));
            self.requested_focus = false;
        }

        if ui.ctx().input(|i| i.key_pressed(egui::Key::ArrowUp)) {
            self.state_manager.state_data.select_prev();
            self.requested_scroll = true;
        }
        if ui.ctx().input(|i| i.key_pressed(egui::Key::ArrowDown)) {
            self.state_manager.state_data.select_next();
            self.requested_scroll = true;
        }

        if ui
            .ctx()
            .input_mut(|input| input.consume_key(egui::Modifiers::COMMAND, egui::Key::Enter))
        {
            if let Some(header) = self.get_header() {
                selected_element = Some(header);
            }
        } else if ui.ctx().input(|input| input.key_pressed(egui::Key::Enter)) {
            let selection = self.state_manager.state_data.get_selection();
            if let Some(selection) = selection {
                selected_element = Some(selection);
                // select_message = self.state_manager.functions.on_entry(&selected);
            } else {
                let elements = self.state_manager.state_data.get_elements();
                if !elements.is_empty() {
                    selected_element = Some(elements.first().unwrap().to_owned());
                }
            };
        }
        if let Some(se) = selected_element {
            self.select(&se)
        } else {
            None
        }
    }
}

struct SelectorStateManager<F, P, D>
where
    F: FilteredListFunctions<P, D> + 'static,
    P: Send + Sync + Clone + 'static,
    D: ListElement + 'static,
{
    state: StateMessage<P, D>,
    provider: Option<Arc<P>>,
    state_data: StateData<D>,
    functions: Arc<F>,
    tx: Sender<StateMessage<P, D>>,
    rx: Receiver<StateMessage<P, D>>,
    deduped_message_bus: VecDeque<StateMessage<P, D>>,
}

pub struct StateData<D>
where
    D: ListElement + 'static,
{
    pub filter_text: String,
    pub elements: Vec<D>,
    pub selected: Option<usize>,
}

impl<D> StateData<D>
where
    D: ListElement + 'static,
{
    fn get_elements(&self) -> &Vec<D> {
        &self.elements
    }

    fn get_selection(&self) -> Option<D> {
        if let Some(selected) = self.selected {
            let elements = self.get_elements();
            let sel = elements.get(selected);
            sel.cloned()
        } else {
            None
        }
    }

    fn get_selected(&self) -> Option<usize> {
        self.selected
    }

    fn set_selected(&mut self, number: Option<usize>) {
        let elements = self.get_elements();
        if !elements.is_empty() {
            self.selected = number.map(|n| std::cmp::min(elements.len() - 1, n));
        } else {
            self.selected = None;
        }
    }

    fn select_next(&mut self) {
        let elements = self.get_elements();
        if !elements.is_empty() {
            self.selected = Some(if let Some(mut selected) = self.selected {
                selected += 1;
                if selected > elements.len() - 1 {
                    selected - elements.len()
                } else {
                    selected
                }
            } else {
                0
            });
        } else {
            self.selected = None;
        }
    }

    fn select_prev(&mut self) {
        let elements = self.get_elements();
        if !elements.is_empty() {
            self.selected = Some(if let Some(mut selected) = self.selected {
                if selected == 0 {
                    selected = elements.len() - 1;
                } else {
                    selected -= 1;
                }
                selected
            } else {
                0
            });
        } else {
            self.selected = None;
        }
    }
}

impl<F, P, D> SelectorStateManager<F, P, D>
where
    F: FilteredListFunctions<P, D> + 'static,
    P: Send + Sync + Clone + 'static,
    D: ListElement + 'static,
{
    fn new(functions: F) -> Self {
        let (tx, rx) = crossbeam_channel::unbounded();
        let state_data = StateData {
            filter_text: String::new(),
            elements: vec![],
            selected: None,
        };
        Self {
            state: StateMessage::Initializing,
            provider: None,
            state_data,
            functions: Arc::new(functions),
            tx,
            rx,
            deduped_message_bus: VecDeque::new(),
        }
    }

    fn initialize(&mut self) {
        debug!("Initializing");
        self.state = StateMessage::Initializing;
        self.state_data.elements.clear();
        let tx = self.tx.clone();
        let functions = self.functions.clone();
        std::thread::spawn(move || {
            let provider = functions.init();
            if let Err(e) = tx.send(StateMessage::Initialized { provider }) {
                error!("Error sending initialized status: {}", e);
            }
        });
    }

    fn trigger_filter(&mut self) {
        if let Some(provider_arc) = &self.provider {
            self.state = StateMessage::Filtering;
            let tx = self.tx.clone();
            let functions = self.functions.clone();
            let filter_text = self.state_data.filter_text.clone();
            let provider = Arc::clone(provider_arc);
            std::thread::spawn(move || {
                info!("Applying filter");
                let data = functions.filter(filter_text.clone(), &provider);
                if let Err(e) = tx.send(StateMessage::Filtered {
                    filter: filter_text,
                    data,
                }) {
                    error!("Error sending ready status: {}", e);
                }
            });
        } else {
            panic!(
                "Wrong state, no provider present, current state is: {}",
                self.state
            );
        }
    }

    fn update(&mut self) {
        // We make sure we don't trigger two equal state changes consecutively
        // this is especially relevant for the filters, so if a filter function
        // takes a little, we don't want to stack filter changes if the text
        // of the filter changes faster than the actual results
        while let Ok(state) = self.rx.try_recv() {
            if let Some(queued_state) = self.deduped_message_bus.back() {
                if core::mem::discriminant(queued_state) != core::mem::discriminant(&state) {
                    self.deduped_message_bus.push_back(state);
                } else {
                    debug!(
                        "Duplicated state events so we are replacing the last one in the queue: {}",
                        state
                    );
                    self.deduped_message_bus.pop_back();
                    self.deduped_message_bus.push_back(state);
                }
            } else {
                self.deduped_message_bus.push_back(state);
            }
        }
        if let Some(state) = self.deduped_message_bus.pop_front() {
            info!("New Status received: {}", state);
            self.state = state;
            match &self.state {
                StateMessage::Initializing => {
                    info!("Status is clear, we initialize");
                    self.initialize()
                }
                StateMessage::Initialized { provider } => {
                    info!("Status initialized, we proceed to apply filter");
                    // Only place we need to clone the provider
                    self.provider = Some(Arc::new(provider.to_owned()));
                    self.trigger_filter();
                }
                StateMessage::Filtering => {
                    // We are filtering, waiting for results
                }
                StateMessage::Filtered { filter, data } => {
                    self.state_data.elements = data.to_owned();
                    self.state = StateMessage::Ready {
                        filter: filter.to_owned(),
                    };
                }
                StateMessage::Ready { filter: _ } => {}
            }
        }
        if let StateMessage::Ready { filter } = &self.state {
            // We are ready to show elements
            if filter != &self.state_data.filter_text {
                info!("Filter changed, we reapply the filter");
                self.trigger_filter();
            }
        }
    }
}

pub trait ListElement: Send + Sync + Clone {
    fn get_height_mult(&self) -> f32;
    fn get_icon(&self) -> impl Into<egui::WidgetText>;
    fn get_label(&self) -> impl Into<egui::WidgetText>;
}
