use std::{collections::VecDeque, sync::Arc};

use crossbeam_channel::{Receiver, Sender};
use eframe::egui;
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

pub(super) struct FilteredList<F, P, D>
where
    F: FilteredListFunctions<P, D> + 'static,
    P: Send + Sync + Clone + 'static,
    D: ListElement + 'static,
{
    state_manager: SelectorStateManager<F, P, D>,
    message_sender: Sender<EditorMessage>,
    requested_focus: bool,
    requested_scroll: bool,
}

impl<F, P, D> FilteredList<F, P, D>
where
    F: FilteredListFunctions<P, D> + 'static,
    P: Send + Sync + Clone + 'static,
    D: ListElement + 'static,
{
    pub fn new(functions: F, message_sender: Sender<EditorMessage>) -> Self {
        let mut state_manager = SelectorStateManager::new(functions);
        state_manager.initialize();
        Self {
            state_manager,
            message_sender,
            requested_focus: true,
            requested_scroll: false,
        }
    }

    pub fn request_focus(&mut self) {
        self.requested_focus = true;
    }

    fn select(&mut self, selected: &D) {
        if let Some(message) = self.state_manager.functions.on_entry(selected) {
            match message {
                FilteredListFunctionMessage::ToEditor(editor_message) => {
                    if let Err(e) = self.message_sender.send(editor_message) {
                        error!("Can't send the message to editor, Err: {}", e)
                    }
                }
                FilteredListFunctionMessage::ResetState(functions) => {
                    self.state_manager.functions = Arc::new(functions);
                    self.request_focus();
                    if let Err(e) = self.state_manager.tx.send(StateMessage::Initializing) {
                        error!("Can't reset the state, Err: {}", e)
                    }
                }
            }
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
}

impl<F, P, D> EditorModal for FilteredList<F, P, D>
where
    F: FilteredListFunctions<P, D>,
    P: Send + Sync + Clone + 'static,
    D: ListElement + 'static,
{
    fn update(&mut self, ui: &mut egui::Ui) {
        self.state_manager.update();
        let mut selected_element = None;

        let header = self.get_header();
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
                let _filter_response = ui.add(
                    egui::TextEdit::singleline(&mut self.state_manager.state_data.filter_text)
                        .desired_width(f32::INFINITY)
                        .id(ID_SEARCH.into()),
                );

                ui.separator();

                let mut selected = self.state_manager.state_data.get_selected();
                let scroll_area = egui::scroll_area::ScrollArea::vertical()
                    .max_height(400.0)
                    .auto_shrink(false);
                scroll_area.show(ui, |ui| {
                    ui.vertical(|ui| {
                        if let Some(element) = &header {
                            let header_response = element.draw_element(ui);
                            if header_response.clicked() {
                                selected_element = Some(element.clone());
                            }
                            if header_response.hovered() {
                                selected = None;
                                header_response.highlight();
                            }
                            ui.separator();
                        }
                        let elements = self.state_manager.state_data.get_elements();
                        for (pos, element) in elements.iter().enumerate() {
                            let response = element.draw_element(ui);
                            if response.clicked() {
                                selected_element = Some(element.clone());
                            }
                            if response.hovered() {
                                selected = Some(pos);
                            }
                            if Some(pos) == selected {
                                if self.requested_scroll {
                                    response.scroll_to_me(Some(egui::Align::Center));
                                    self.requested_scroll = false;
                                }
                                response.highlight();
                            }
                        }
                    });
                });
                self.state_manager.state_data.set_selected(selected);
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
            if let Some(header) = header {
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
            self.select(&se);
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
    fn draw_element(&self, ui: &mut egui::Ui) -> egui::Response;
}
