use std::collections::HashMap;

use dioxus::prelude::Callback;
use kimun_core::nfs::VaultPath;

pub struct Subscription {
    callback: Callback<GlobalEvent>,
}

impl Subscription {
    fn new(callback: Callback<GlobalEvent>) -> Self {
        Self { callback }
    }
}

pub struct PubSub {
    subscribers: HashMap<String, Subscription>,
}

impl PubSub {
    pub fn new() -> Self {
        Self {
            subscribers: HashMap::default(),
        }
    }

    pub fn subscribe<S: AsRef<str>>(&mut self, id: S, callback: Callback<GlobalEvent>) {
        self.subscribers
            .insert(id.as_ref().to_string(), Subscription::new(callback));
    }
    pub fn unsubscribe<S: AsRef<str>>(&mut self, id: S) {
        self.subscribers.remove(id.as_ref());
    }
    pub fn publish(&self, event: GlobalEvent) {
        for (_entry, subscription) in &self.subscribers {
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
}
