use std::{cell::RefCell, collections::HashMap, rc::Rc};

use dioxus::prelude::Callback;
use kimun_core::nfs::VaultPath;

#[derive(Clone)]
struct Subscription<E>
where
    E: Clone + 'static,
{
    callback: Callback<E>,
}

impl<E> Subscription<E>
where
    E: Clone + 'static,
{
    fn new(callback: Callback<E>) -> Self {
        Self { callback }
    }
}

/// The publisher/subscriber channel to send events across different components
#[derive(Clone)]
pub struct PubSub<E>
where
    E: Clone + 'static,
{
    subscribers: Rc<RefCell<HashMap<String, Subscription<E>>>>,
}

impl<E> PubSub<E>
where
    E: Clone + 'static,
{
    pub fn new() -> Self {
        Self {
            subscribers: Rc::new(RefCell::new(HashMap::default())),
        }
    }

    pub fn subscribe<S: AsRef<str>>(&self, id: S, callback: Callback<E>) {
        self.subscribers
            .borrow_mut()
            .insert(id.as_ref().to_string(), Subscription::new(callback));
    }
    pub fn unsubscribe<S: AsRef<str>>(&self, id: S) {
        self.subscribers.borrow_mut().remove(id.as_ref());
    }
    pub fn publish(&self, event: E) {
        for subscription in self.subscribers.borrow().values() {
            subscription.callback.call(event.clone());
        }
    }
}

/// Broadcast info when something happens
#[derive(Debug, Clone, PartialEq, Hash)]
pub enum GlobalEvent {
    SaveCurrentNote,
    MarkNoteClean,
    Deleted(VaultPath),
    Moved {
        from: VaultPath,
        to: VaultPath,
    },
    Renamed {
        old_name: VaultPath,
        new_name: VaultPath,
    },
    NewNoteCreated(VaultPath),
    NewDirectoryCreated(VaultPath),
}
