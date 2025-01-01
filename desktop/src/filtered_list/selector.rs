use std::sync::{
    mpsc::{self, Receiver, Sender},
    Arc, Mutex,
};

use log::error;
use nucleo::{
    pattern::{CaseMatching, Normalization, Pattern},
    Config, Matcher,
};
use rayon::slice::ParallelSliceMut;

use super::row::RowItem;

pub struct Selector<R>
where
    R: RowItem + 'static,
{
    elements: Arc<Mutex<Vec<R>>>,
    selected: Option<usize>,
    filtered_elements: Vec<R>,
    tx: Sender<Vec<R>>,
    rx: Receiver<Vec<R>>,
}

impl<R> Selector<R>
where
    R: RowItem + 'static,
{
    pub fn new(elements: Vec<R>) -> Self {
        let filtered_elements = elements.clone();
        let (tx, rx) = mpsc::channel();
        Self {
            elements: Arc::new(Mutex::new(elements)),
            selected: None,
            filtered_elements,
            tx,
            rx,
        }
    }

    pub fn get_selected(&self) -> Option<usize> {
        self.selected
    }

    pub fn get_selection(&self) -> Option<&R> {
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

    fn filter_elements(elements: &Vec<R>, filter: &str) -> Vec<R> {
        let mut matcher = Matcher::new(Config::DEFAULT);
        let res = Pattern::parse(filter, CaseMatching::Ignore, Normalization::Smart)
            .match_list(elements, &mut matcher)
            .iter()
            .map(|e| e.0.to_owned())
            .collect::<Vec<R>>();
        res
    }

    pub fn clear(&mut self) {
        self.elements.lock().unwrap().clear();
        self.filtered_elements.clear();
    }

    pub fn add_element(&mut self, element: R) {
        self.elements.lock().unwrap().push(element);
    }

    pub fn add_elements(&mut self, element: Vec<R>) {
        self.elements.lock().unwrap().append(&mut element.clone());
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

    pub fn get_elements(&mut self) -> &Vec<R> {
        if let Some(elements) = self.rx.try_iter().last() {
            self.filtered_elements = elements;
        }
        self.filtered_elements
            .par_sort_by(|a, b| a.get_sort_string().cmp(&b.get_sort_string()));
        &self.filtered_elements
    }
}
