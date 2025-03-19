use std::sync::{atomic::AtomicBool, Arc, Mutex};

use kimun_core::{nfs::VaultPath, note::NoteDetails, NoteVault};
use log::{error, info};

const AUTOSAVE_SECS: u64 = 5;

pub struct SaveManager {
    note: Arc<Mutex<NoteDetails>>,
    active_loop: Arc<AtomicBool>,
    is_saved: Arc<AtomicBool>,
    vault: NoteVault,
}

impl SaveManager {
    pub fn new<S: AsRef<str>>(text: S, path: &VaultPath, vault: &NoteVault) -> Self {
        let note_details = NoteDetails::new(path, text);
        Self {
            note: Arc::new(Mutex::new(note_details)),
            active_loop: Arc::new(AtomicBool::new(true)),
            is_saved: Arc::new(AtomicBool::new(true)),
            vault: vault.to_owned(),
        }
    }

    pub fn init_loop(&self) {
        let note = self.note.clone();
        let is_saved = self.is_saved.clone();
        let vault = self.vault.clone();
        let active_loop = self.active_loop.clone();
        std::thread::spawn(move || {
            while active_loop.load(std::sync::atomic::Ordering::Relaxed) {
                std::thread::sleep(std::time::Duration::from_secs(AUTOSAVE_SECS));
                info!("Should I save...");
                if !is_saved.load(std::sync::atomic::Ordering::Relaxed) {
                    info!("Saving...");
                    let note_lock = note.lock().unwrap();
                    let path = &note_lock.path;
                    let text = &note_lock.raw_text;
                    if let Err(e) = vault.save_note(path, text) {
                        error!("Error saving Note at {}: {}", path, e);
                    } else {
                        is_saved.store(true, std::sync::atomic::Ordering::Relaxed);
                    }
                }
            }
        });
    }

    pub fn update_text<S: AsRef<str>>(&mut self, text: S) {
        let text = text.as_ref().to_string();
        let current_note = self.note.clone();
        let is_saved = self.is_saved.clone();
        std::thread::spawn(move || {
            let mut note_lock = current_note.lock().unwrap();
            let path = &note_lock.path;
            *note_lock = NoteDetails::new(path, text);
            is_saved.store(false, std::sync::atomic::Ordering::Relaxed);
        });
    }

    pub fn load<S: AsRef<str>>(&self, text: S, path: &VaultPath) {
        *self.note.lock().unwrap() = NoteDetails::new(path, text);
        self.is_saved
            .store(true, std::sync::atomic::Ordering::Relaxed);
    }

    pub fn save(&self) -> anyhow::Result<()> {
        let note_lock = self.note.lock().unwrap();
        let path = &note_lock.path;
        let text = &note_lock.raw_text;
        self.vault.save_note(path, text)?;
        self.is_saved
            .store(false, std::sync::atomic::Ordering::Relaxed);
        Ok(())
    }

    pub fn get_path(&self) -> VaultPath {
        self.note.lock().unwrap().path.to_owned()
    }
}

impl Drop for SaveManager {
    fn drop(&mut self) {
        self.active_loop
            .store(false, std::sync::atomic::Ordering::Relaxed);
    }
}
