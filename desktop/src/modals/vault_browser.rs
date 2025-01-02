use std::sync::{mpsc, Arc, Mutex};

use eframe::egui;
use log::{debug, error};
use notes_core::{nfs::NotePath, NoteVault, SearchResult, VaultBrowseOptionsBuilder};
use rayon::slice::ParallelSliceMut;

use crate::icons;

use super::EditorModal;

pub const ID_SEARCH: &str = "Search Popup";

pub(super) struct VaultBrowser {
    filter_text: String,
    selector: Selector,
    rx: Option<mpsc::Receiver<SearchResult>>,
    to_clear: bool,
    requested_focus: bool,
    vault: Arc<NoteVault>,
}

impl VaultBrowser {
    pub fn new(vault: NoteVault) -> Self {
        let selector = Selector::new();

        Self {
            filter_text: String::new(),
            selector,
            rx: None,
            to_clear: false,
            requested_focus: true,
            vault: Arc::new(vault),
        }
    }

    pub fn browse_path(&mut self, path: &NotePath) {
        let search_path = if path.is_note() {
            path.get_parent_path().0
        } else {
            path.to_owned()
        };
        let (browse_options, receiver) = VaultBrowseOptionsBuilder::new(&search_path).build();
        let vault = Arc::clone(&self.vault);
        self.rx = Some(receiver);

        std::thread::spawn(move || {
            debug!("Retreiving notes for dialog");
            vault
                .browse_vault(browse_options)
                .expect("Error getting notes");
        });
    }

    pub fn _clear(&mut self) {
        self.to_clear = true;
    }

    pub fn _request_focus(&mut self) {
        self.requested_focus = true;
    }

    fn update_filter(&mut self) {
        if let Some(rx) = &self.rx {
            let trigger_filter = if let Ok(row) = rx.try_recv() {
                // info!("adding to list {}", row.as_ref());
                self.selector.add_element(row.into());
                true
            } else {
                false
            };
            while let Ok(row) = rx.try_recv() {
                self.selector.add_element(row.into());
            }
            if trigger_filter {
                self.selector.filter_content(&self.filter_text);
            }
        }
    }
}

impl EditorModal for VaultBrowser {
    fn update(&mut self, ui: &mut egui::Ui) {
        if self.to_clear {
            self.selector.clear();
            self.to_clear = false;
        }

        self.update_filter();

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

        if self.requested_focus {
            ui.ctx()
                .memory_mut(|mem| mem.request_focus(ID_SEARCH.into()));
            self.requested_focus = false;
        }
    }
}

pub struct Selector {
    elements: Arc<Mutex<Vec<SelectorEntry>>>,
    selected: Option<usize>,
    filtered_elements: Vec<SelectorEntry>,
    tx: mpsc::Sender<Vec<SelectorEntry>>,
    rx: mpsc::Receiver<Vec<SelectorEntry>>,
}

impl Selector {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            elements: Arc::new(Mutex::new(vec![])),
            selected: None,
            filtered_elements: vec![],
            tx,
            rx,
        }
    }

    pub fn get_selected(&self) -> Option<usize> {
        self.selected
    }

    pub fn get_selection(&self) -> Option<&SelectorEntry> {
        if let Some(selected) = self.selected {
            self.filtered_elements.get(selected)
        } else {
            None
        }
    }

    pub fn set_selected(&mut self, number: Option<usize>) {
        if self.filtered_elements.is_empty() {
            self.selected = None;
        } else {
            self.selected = number.map(|n| std::cmp::min(self.filtered_elements.len() - 1, n));
        }
    }

    pub fn select_next(&mut self) {
        if self.filtered_elements.is_empty() {
            self.selected = None;
        } else {
            self.selected = Some(if let Some(mut selected) = self.selected {
                selected += 1;
                if selected > self.filtered_elements.len() - 1 {
                    selected - self.filtered_elements.len()
                } else {
                    selected
                }
            } else {
                0
            });
        }
    }

    pub fn select_prev(&mut self) {
        if self.filtered_elements.is_empty() {
            self.selected = None;
        } else {
            self.selected = Some(if let Some(mut selected) = self.selected {
                if selected == 0 {
                    selected = self.filtered_elements.len() - 1;
                } else {
                    selected -= 1;
                }
                selected
            } else {
                0
            });
        }
    }

    fn filter_elements(elements: &Vec<SelectorEntry>, filter: &str) -> Vec<SelectorEntry> {
        let mut matcher = nucleo::Matcher::new(nucleo::Config::DEFAULT);
        let res = nucleo::pattern::Pattern::parse(
            filter,
            nucleo::pattern::CaseMatching::Ignore,
            nucleo::pattern::Normalization::Smart,
        )
        .match_list(elements, &mut matcher)
        .iter()
        .map(|e| e.0.to_owned())
        .collect::<Vec<SelectorEntry>>();
        res
    }

    pub fn clear(&mut self) {
        self.elements.lock().unwrap().clear();
        self.filtered_elements.clear();
    }

    pub fn add_element(&mut self, element: SelectorEntry) {
        self.elements.lock().unwrap().push(element);
    }

    pub fn filter_content(&mut self, filter_text: &String) {
        let tx = self.tx.clone();
        let elements = Arc::clone(&self.elements);
        let filter_text = filter_text.to_owned();
        std::thread::spawn(move || {
            let filtered = Self::filter_elements(&elements.lock().unwrap(), &filter_text);
            if let Err(e) = tx.send(filtered) {
                error!("Error sending filtered results: {}", e)
            }
        });
    }

    pub fn get_elements(&mut self) -> &Vec<SelectorEntry> {
        if let Some(elements) = self.rx.try_iter().last() {
            self.filtered_elements = elements;
        }
        self.filtered_elements
            .par_sort_by(|a, b| a.get_sort_string().cmp(&b.get_sort_string()));
        &self.filtered_elements
    }
}

#[derive(Clone)]
pub struct SelectorEntry {
    path: NotePath,
    path_str: String,
    entry_type: SelectorEntryType,
}

#[derive(Clone)]
enum SelectorEntryType {
    Note,
    Directory,
    Attachment,
}

impl From<SearchResult> for SelectorEntry {
    fn from(value: SearchResult) -> Self {
        match value {
            SearchResult::Note(note_details) => SelectorEntry {
                path: note_details.path.clone(),
                path_str: note_details.path.to_string(),
                entry_type: SelectorEntryType::Note,
            },
            SearchResult::Directory(directory_details) => SelectorEntry {
                path: directory_details.path.clone(),
                path_str: directory_details.path.to_string(),
                entry_type: SelectorEntryType::Directory,
            },
            SearchResult::Attachment(path) => SelectorEntry {
                path: path.clone(),
                path_str: path.to_string(),
                entry_type: SelectorEntryType::Attachment,
            },
        }
    }
}

impl SelectorEntry {
    fn get_label(&self, ui: &mut egui::Ui) -> egui::Response {
        let icon = match &self.entry_type {
            SelectorEntryType::Note => icons::NOTE,
            SelectorEntryType::Directory => icons::DIRECTORY,
            SelectorEntryType::Attachment => icons::ATTACHMENT,
        };
        ui.label(format!("{}   {}", icon, self.path_str))
    }

    fn get_sort_string(&self) -> String {
        match &self.entry_type {
            SelectorEntryType::Note => format!("2{}", self.path_str),
            SelectorEntryType::Directory => format!("1{}", self.path_str),
            SelectorEntryType::Attachment => format!("3{}", self.path_str),
        }
    }
}

impl AsRef<str> for SelectorEntry {
    fn as_ref(&self) -> &str {
        &self.path_str
    }
}
