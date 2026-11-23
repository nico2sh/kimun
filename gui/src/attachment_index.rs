//! One list of a vault's attachments, shared by every read-only view that
//! draws **Inline image**s (CONTEXT.md). A facet view can show 50 chunks and
//! the preview loads its own note: each walking the vault would be far too
//! much, so the first one to ask walks (on a background task, through
//! `NoteVault::list_attachments`) and the rest wait for that one walk.
//!
//! The editor keeps its own per-editor listing and does not use this.
//!
//! A GPUI global with a cheap `clone()` (like `ImageStore`): take a clone
//! from `cx.default_global` before calling [`AttachmentIndex::request`], so
//! the global is not borrowed while the task starts.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::{Arc, Weak};

use gpui_kit::{App, AppContext as _, Global};
use kimun_core::NoteVault;
use kimun_core::nfs::VaultPath;

type Callback = Box<dyn FnOnce(&mut App)>;

/// What is known about one vault.
struct Entry {
    /// Tells a dead vault whose address was reused from the one we know.
    vault: Weak<NoteVault>,
    /// The last list found; stays visible while a newer one is walked.
    list: Option<Arc<Vec<VaultPath>>>,
    /// `list` may be out of date: the next `request` walks again.
    stale: bool,
    /// Bumped by `refresh`, so a walk that began before it is known.
    generation: u64,
    /// The generation of the walk in flight, if one is.
    walking: Option<u64>,
    /// Callbacks to run when the list lands.
    waiting: Vec<Callback>,
}

#[derive(Default)]
struct Inner {
    entries: HashMap<usize, Entry>,
    /// How many walks were started.
    #[cfg(test)]
    walks: usize,
}

/// The attachments of each vault, listed once until refreshed.
#[derive(Clone, Default)]
pub(crate) struct AttachmentIndex {
    inner: Rc<RefCell<Inner>>,
}

impl Global for AttachmentIndex {}

fn key(vault: &Arc<NoteVault>) -> usize {
    Arc::as_ptr(vault) as usize
}

impl Inner {
    /// The live entry for `vault`, made when there is none (or the one there
    /// belonged to a vault since dropped).
    fn entry(&mut self, vault: &Arc<NoteVault>) -> &mut Entry {
        let entry = self.entries.entry(key(vault)).or_insert_with(|| Entry {
            vault: Arc::downgrade(vault),
            list: None,
            stale: true,
            generation: 0,
            walking: None,
            waiting: Vec::new(),
        });
        if entry.vault.upgrade().is_none() {
            *entry = Entry {
                vault: Arc::downgrade(vault),
                list: None,
                stale: true,
                generation: 0,
                walking: None,
                waiting: Vec::new(),
            };
        }
        entry
    }
}

impl AttachmentIndex {
    /// The list as last found, sorted; `None` before the first one lands.
    /// After `refresh` it is still the old list until the new one lands.
    #[cfg_attr(not(test), expect(dead_code, reason = "used by the read-only views"))]
    pub(crate) fn get(&self, vault: &Arc<NoteVault>) -> Option<Arc<Vec<VaultPath>>> {
        self.inner
            .borrow()
            .entries
            .get(&key(vault))
            .filter(|e| e.vault.upgrade().is_some())
            .and_then(|e| e.list.clone())
    }

    /// Ask for the list. `on_done` runs on the main thread once the list is
    /// there: on the next tick when it already is (never inside this call),
    /// otherwise when the walk lands. Requests made while a walk is in
    /// flight share it and all their callbacks run. A callback should hold a
    /// `WeakEntity`: it runs even when its requester is gone.
    #[cfg_attr(not(test), expect(dead_code, reason = "used by the read-only views"))]
    pub(crate) fn request(
        &mut self,
        vault: &Arc<NoteVault>,
        cx: &mut App,
        on_done: impl FnOnce(&mut App) + 'static,
    ) {
        let start = {
            let mut inner = self.inner.borrow_mut();
            let entry = inner.entry(vault);
            if entry.list.is_some() && !entry.stale {
                drop(inner);
                cx.defer(on_done);
                return;
            }
            entry.waiting.push(Box::new(on_done));
            entry.walking != Some(entry.generation)
        };
        if start {
            self.walk(vault, cx);
        }
    }

    /// Forget the list: the next `request` walks again. The old list stays
    /// visible through `get` until the new one lands.
    pub(crate) fn refresh(&mut self, vault: &Arc<NoteVault>) {
        let mut inner = self.inner.borrow_mut();
        if let Some(entry) = inner.entries.get_mut(&key(vault)) {
            entry.stale = true;
            entry.generation += 1;
        }
    }

    /// Start a walk for the vault's current generation.
    fn walk(&self, vault: &Arc<NoteVault>, cx: &mut App) {
        let generation = {
            let mut inner = self.inner.borrow_mut();
            #[cfg(test)]
            {
                inner.walks += 1;
            }
            let entry = inner.entry(vault);
            entry.walking = Some(entry.generation);
            entry.generation
        };
        let listing = {
            let vault = vault.clone();
            cx.background_spawn(async move {
                let mut found = vault.list_attachments();
                found.sort();
                found
            })
        };
        let this = self.clone();
        let weak = Arc::downgrade(vault);
        cx.spawn(async move |cx| {
            let found = listing.await;
            cx.update(|cx| this.landed(&weak, generation, found, cx));
        })
        .detach();
    }

    fn landed(
        &self,
        vault: &Weak<NoteVault>,
        generation: u64,
        found: Vec<VaultPath>,
        cx: &mut App,
    ) {
        let Some(vault) = vault.upgrade() else {
            return;
        };
        let (callbacks, again) = {
            let mut inner = self.inner.borrow_mut();
            let entry = inner.entry(&vault);
            // The same list again keeps the same `Arc`, so users do not
            // recompute.
            if entry.list.as_deref() != Some(&found) {
                entry.list = Some(Arc::new(found));
            }
            if entry.walking == Some(generation) {
                entry.walking = None;
            }
            if generation == entry.generation {
                entry.stale = false;
                (std::mem::take(&mut entry.waiting), false)
            } else {
                // A refresh came while this walk ran: what it found may be
                // old. The waiting callbacks wait for a walk that began
                // after it.
                (
                    Vec::new(),
                    entry.walking.is_none() && !entry.waiting.is_empty(),
                )
            }
        };
        if again {
            self.walk(&vault, cx);
        }
        for callback in callbacks {
            callback(cx);
        }
    }

    /// How many walks were started.
    #[cfg(test)]
    pub(crate) fn walks(&self) -> usize {
        self.inner.borrow().walks
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use gpui_kit::TestAppContext;
    use tokio::runtime::Runtime;

    use super::*;
    use crate::test_vault::test_vault;

    fn runtime() -> Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime")
    }

    fn png(vault: &Arc<NoteVault>, name: &str) {
        let path = vault.path_to_pathbuf(&VaultPath::new(name));
        std::fs::write(path, b"x").unwrap();
    }

    fn names(list: &Arc<Vec<VaultPath>>) -> Vec<String> {
        list.iter().map(|p| p.to_string()).collect()
    }

    /// Request and let the walk land; the callback ran.
    fn ask(cx: &mut TestAppContext, index: &AttachmentIndex, vault: &Arc<NoteVault>) {
        let ran = Rc::new(Cell::new(false));
        let r = ran.clone();
        let mut index = index.clone();
        cx.update(|cx| index.request(vault, cx, move |_| r.set(true)));
        cx.run_until_parked();
        assert!(ran.get());
    }

    #[gpui_kit::test]
    fn two_requests_share_one_walk(cx: &mut TestAppContext) {
        let rt = runtime();
        let tv = rt.block_on(test_vault(&[], &[]));
        let vault = tv.vault.clone();
        png(&vault, "a.png");
        let index = AttachmentIndex::default();
        let ran = Rc::new(Cell::new(0));
        let mut i = index.clone();
        cx.update(|cx| {
            for _ in 0..2 {
                let r = ran.clone();
                i.request(&vault, cx, move |_| r.set(r.get() + 1));
            }
        });
        cx.run_until_parked();
        assert_eq!(index.walks(), 1);
        assert_eq!(ran.get(), 2);
        assert_eq!(names(&index.get(&vault).unwrap()), vec!["/a.png"]);
    }

    #[gpui_kit::test]
    fn a_request_after_the_list_is_there_runs_on_the_next_tick(cx: &mut TestAppContext) {
        let rt = runtime();
        let tv = rt.block_on(test_vault(&[], &[]));
        let vault = tv.vault.clone();
        let index = AttachmentIndex::default();
        ask(cx, &index, &vault);
        let ran = Rc::new(Cell::new(false));
        let r = ran.clone();
        let mut i = index.clone();
        cx.update(|cx| {
            i.request(&vault, cx, move |_| r.set(true));
            assert!(!ran.get(), "not inside the call");
        });
        cx.run_until_parked();
        assert!(ran.get());
        assert_eq!(index.walks(), 1);
    }

    #[gpui_kit::test]
    fn refresh_lists_again(cx: &mut TestAppContext) {
        let rt = runtime();
        let tv = rt.block_on(test_vault(&[], &[]));
        let vault = tv.vault.clone();
        png(&vault, "a.png");
        let index = AttachmentIndex::default();
        ask(cx, &index, &vault);
        png(&vault, "b.png");
        let mut i = index.clone();
        i.refresh(&vault);
        // The old list stays until the new one lands.
        assert_eq!(names(&index.get(&vault).unwrap()), vec!["/a.png"]);
        ask(cx, &index, &vault);
        assert_eq!(index.walks(), 2);
        assert_eq!(names(&index.get(&vault).unwrap()), vec!["/a.png", "/b.png"]);
    }

    #[gpui_kit::test]
    fn a_second_vault_has_its_own_list(cx: &mut TestAppContext) {
        let rt = runtime();
        let (tv1, tv2) = (
            rt.block_on(test_vault(&[], &[])),
            rt.block_on(test_vault(&[], &[])),
        );
        let (one, two) = (tv1.vault.clone(), tv2.vault.clone());
        png(&one, "one.png");
        png(&two, "two.png");
        let index = AttachmentIndex::default();
        ask(cx, &index, &one);
        assert!(index.get(&two).is_none());
        ask(cx, &index, &two);
        assert_eq!(names(&index.get(&one).unwrap()), vec!["/one.png"]);
        assert_eq!(names(&index.get(&two).unwrap()), vec!["/two.png"]);
        assert_eq!(index.walks(), 2);
    }

    #[gpui_kit::test]
    fn an_unchanged_list_does_not_replace_the_arc(cx: &mut TestAppContext) {
        let rt = runtime();
        let tv = rt.block_on(test_vault(&[], &[]));
        let vault = tv.vault.clone();
        png(&vault, "a.png");
        let index = AttachmentIndex::default();
        ask(cx, &index, &vault);
        let before = index.get(&vault).unwrap();
        index.clone().refresh(&vault);
        ask(cx, &index, &vault);
        assert_eq!(index.walks(), 2);
        assert!(Arc::ptr_eq(&before, &index.get(&vault).unwrap()));
    }
}
