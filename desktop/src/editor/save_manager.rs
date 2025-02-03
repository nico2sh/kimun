use std::sync::{atomic::AtomicBool, Arc, Mutex};

use kimun_core::{nfs::VaultPath, NoteVault};
use log::{error, info};

const AUTOSAVE_SECS: u64 = 5;

pub struct SaveManager {
    text: Arc<Mutex<String>>,
    active_loop: Arc<AtomicBool>,
    is_saved: Arc<AtomicBool>,
    path: Arc<Mutex<Option<VaultPath>>>,
    vault: NoteVault,
}

impl SaveManager {
    pub fn new<S: AsRef<str>>(text: S, path: &Option<VaultPath>, vault: &NoteVault) -> Self {
        Self {
            text: Arc::new(Mutex::new(text.as_ref().to_string())),
            active_loop: Arc::new(AtomicBool::new(true)),
            is_saved: Arc::new(AtomicBool::new(true)),
            path: Arc::new(Mutex::new(path.to_owned())),
            vault: vault.to_owned(),
        }
    }

    pub fn init_loop(&self) {
        let text = self.text.clone();
        let is_saved = self.is_saved.clone();
        let vault = self.vault.clone();
        let path = self.path.clone();
        let active_loop = self.active_loop.clone();
        std::thread::spawn(move || {
            while active_loop.load(std::sync::atomic::Ordering::Relaxed) {
                std::thread::sleep(std::time::Duration::from_secs(AUTOSAVE_SECS));
                info!("Should I save...");
                if !is_saved.load(std::sync::atomic::Ordering::Relaxed) {
                    info!("Saving...");
                    let path_guard = path.lock().unwrap();
                    if let Some(path) = &*path_guard {
                        if let Err(e) = vault.save_note(path, &*text.lock().unwrap()) {
                            error!("Error saving Note at {}: {}", path, e);
                        } else {
                            is_saved.store(true, std::sync::atomic::Ordering::Relaxed);
                        }
                    }
                }
            }
        });
    }

    pub fn update_text<S: AsRef<str>>(&mut self, text: S) {
        let text = text.as_ref().to_string();
        let current_text = self.text.clone();
        let is_saved = self.is_saved.clone();
        std::thread::spawn(move || {
            *current_text.lock().unwrap() = text;
            is_saved.store(false, std::sync::atomic::Ordering::Relaxed);
        });
    }

    pub fn load<S: AsRef<str>>(&self, text: S, path: &VaultPath) {
        *self.text.lock().unwrap() = text.as_ref().to_string();
        *self.path.lock().unwrap() = Some(path.to_owned());
        self.is_saved
            .store(true, std::sync::atomic::Ordering::Relaxed);
    }

    pub fn save(&self) -> anyhow::Result<()> {
        if let Some(path) = &*self.path.lock().unwrap() {
            self.vault.save_note(path, &*self.text.lock().unwrap())?;
        }
        Ok(())
    }

    pub fn get_path(&self) -> Option<VaultPath> {
        self.path.lock().unwrap().to_owned()
    }
}

impl Drop for SaveManager {
    fn drop(&mut self) {
        self.active_loop
            .store(false, std::sync::atomic::Ordering::Relaxed);
    }
}
