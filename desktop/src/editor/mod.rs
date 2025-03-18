use anyhow::bail;
use iced::{
    Font,
    Length::Fill,
    Task, highlighter,
    keyboard::{Key, Modifiers, key::Named},
    widget::{row, text_editor},
};
use kimun_core::{NoteVault, nfs::VaultPath, note::NoteDetails};
use log::{debug, error};

use crate::{KimunMessage, KimunPage, settings::Settings};

#[derive(Clone, Debug)]
pub enum EditorMessage {
    Edit(text_editor::Action),
    OpenCreateOrSearchNote(String),
    OpenNote(VaultPath),
    NewNote(VaultPath),
    NewJournal,
    ShowPreview(bool),
    SaveTick,
    Save(NoteVault, VaultPath, String),
    Saved(VaultPath),
    OpenSettings,
}

#[derive(PartialEq, Eq)]
enum SavingStatus {
    Saved,
    Saving,
    NotSaved,
}

impl From<EditorMessage> for KimunMessage {
    fn from(value: EditorMessage) -> Self {
        KimunMessage::EditorMessage(value)
    }
}

pub struct Editor {
    note_details: NoteDetails,
    content: text_editor::Content,
    saved: SavingStatus,
    vault: NoteVault,
    path: VaultPath,
    settings: Settings,
}

impl Drop for Editor {
    fn drop(&mut self) {
        if let Err(e) = self.vault.save_note(&self.path, self.content.text()) {
            error!("Error saving note: {}", e);
        }
    }
}

impl Editor {
    pub(crate) fn new(
        vault: &kimun_core::NoteVault,
        path: &kimun_core::nfs::VaultPath,
        settings: &Settings,
    ) -> anyhow::Result<Self> {
        let note_details = vault.load_note(path)?;
        let content = text_editor::Content::with_text(&note_details.raw_text);
        Ok(Self {
            note_details,
            content,
            saved: SavingStatus::Saved,
            vault: vault.to_owned(),
            path: path.to_owned(),
            settings: settings.to_owned(),
        })
    }

    /// Loads a note from the path
    /// if the path is a note, then we load the note in the current view
    /// if not, we return an error
    fn load_note_path(&mut self, note_path: &VaultPath) -> anyhow::Result<Task<KimunMessage>> {
        // We will save the current note
        let task = Task::done(
            EditorMessage::Save(self.vault.clone(), self.path.clone(), self.content.text()).into(),
        );
        if note_path.is_note() && self.vault.exists(note_path).is_some() {
            let note_details = self.vault.load_note(note_path)?;
            self.settings.add_path_history(note_path);
            self.settings.save_to_disk()?;
            self.set_content(&note_details);
            self.path = note_path.to_owned();
            self.saved = SavingStatus::Saved;
        } else {
            bail!(
                "Note path is not a note or vault path doesn't exist: {}",
                note_path
            )
        };

        Ok(task)
    }

    /// Creates a note from the path
    fn new_note_at_path(&mut self, note_path: &VaultPath) -> anyhow::Result<Task<KimunMessage>> {
        // We will save the current note
        let task = Task::done(
            EditorMessage::Save(self.vault.clone(), self.path.clone(), self.content.text()).into(),
        );

        let mut np = note_path.clone();
        loop {
            if self.vault.exists(&np).is_none() {
                break;
            } else {
                np = np.get_name_on_conflict();
            }
        }
        if np.is_note() {
            let details = NoteDetails::new(&np, String::new());
            self.saved = SavingStatus::Saved;
            self.set_content(&details);
        } else {
            bail!("Note path is not a note: {}", np)
        };

        Ok(task)
    }

    fn set_content(&mut self, details: &NoteDetails) {
        self.content = text_editor::Content::with_text(&details.raw_text);
        self.note_details = details.to_owned();
    }

    fn manage_editor_keys(
        &self,
        kp: &text_editor::KeyPress,
    ) -> Option<text_editor::Binding<KimunMessage>> {
        match (kp.key.as_ref(), kp.modifiers, kp.status) {
            (Key::Named(Named::Tab), _, text_editor::Status::Focused) => {
                // We insert spaces instead of tabs
                // TODO: Manage indenting
                let _tab: Option<text_editor::Binding<KimunMessage>> =
                    Some(text_editor::Binding::Insert('\t'));
                let spaces: Option<text_editor::Binding<KimunMessage>> = Some(
                    text_editor::Binding::Sequence(vec![text_editor::Binding::Insert(' '); 4]),
                );
                spaces
            }
            (Key::Character("k"), Modifiers::COMMAND, _) => Some(text_editor::Binding::Custom(
                KimunMessage::ShowModal(crate::modals::Modals::VaultSearch(self.vault.clone())),
            )),
            (Key::Character("o"), Modifiers::COMMAND, _) => {
                let current_path = &self.path.get_parent_path().0;
                Some(text_editor::Binding::Custom(KimunMessage::ShowModal(
                    crate::modals::Modals::VaultBrowse(self.vault.clone(), current_path.to_owned()),
                )))
            }
            (Key::Character("j"), Modifiers::COMMAND, _) => Some(text_editor::Binding::Custom(
                EditorMessage::NewJournal.into(),
            )),
            _ => None,
        }
    }
}

impl KimunPage for Editor {
    fn update(&mut self, message: crate::KimunMessage) -> anyhow::Result<Task<KimunMessage>> {
        let task = if let KimunMessage::EditorMessage(message) = message {
            match message {
                EditorMessage::Edit(action) => {
                    if self.saved != SavingStatus::NotSaved && action.is_edit() {
                        self.saved = SavingStatus::NotSaved;
                    }

                    self.content.perform(action);

                    Task::none()
                }
                EditorMessage::OpenCreateOrSearchNote(path) => {
                    let path = VaultPath::note_path_from(path);
                    // let path = VaultPath::new(path);
                    let result = self.vault.open_or_search(&path)?;
                    debug!("Got {} results", result.len());
                    match result.len() {
                        0 => {
                            Task::done(EditorMessage::NewNote(path).into())
                            // if let Err(e) = self
                            //     .message_sender
                            //     .send(EditorMessage::NewNote(path).into())
                            // {
                            //     error!("Error sending an editor message: {}", e);
                            // }
                        }
                        1 => {
                            let path = result.first().unwrap().0.path.clone();
                            Task::done(EditorMessage::OpenNote(path).into())
                            // if let Err(e) = self.message_sender.send(EditorMessage::OpenNote(path))
                            // {
                            //     error!("Error sending an editor message: {}", e);
                            // }
                        }
                        _ => {
                            // Task::from(EditorMessage::NoteSelect(path).into())
                            // let _e = self.modal_manager.set_modal(Modals::NoteSelect(
                            //     self.vault.clone(),
                            //     result,
                            //     self.message_sender.clone(),
                            // ));
                            Task::none()
                        }
                    }
                }
                EditorMessage::OpenNote(note_path) => {
                    debug!("Loading note at path {}", note_path);
                    self.load_note_path(&note_path)?
                }
                EditorMessage::NewJournal => {
                    let (data, _content) = self.vault.journal_entry()?;
                    self.load_note_path(&data.path)?
                }
                EditorMessage::NewNote(note_path) => {
                    debug!("New note at: {}", note_path);
                    self.new_note_at_path(&note_path)?
                }
                EditorMessage::SaveTick => {
                    debug!("Received Save Signal");
                    if self.saved == SavingStatus::NotSaved {
                        let vault = self.vault.clone();
                        let path = self.path.clone();
                        let content = self.content.text();
                        Task::done(EditorMessage::Save(vault, path, content).into())
                    } else {
                        Task::none()
                    }
                }
                EditorMessage::Save(vault, path, content) => {
                    self.saved = SavingStatus::Saving;
                    Task::perform(
                        async move { vault.save_note(&path, content) },
                        |res| match res {
                            Ok((entry, _content)) => EditorMessage::Saved(entry.path).into(),
                            Err(e) => KimunMessage::Error(format!("Error saving note: {}", e)),
                        },
                    )
                }
                EditorMessage::Saved(path) => {
                    // Since saving happens asynchronously, we may have saved a note before opening
                    // another one in a new path, so we check we are marking as saved the current
                    if self.path.eq(&path) && self.saved == SavingStatus::Saving {
                        self.saved = SavingStatus::Saved;
                    }
                    Task::none()
                }
                EditorMessage::ShowPreview(visible) => {
                    // self.change_viewer(viewer_type)?;
                    // self.request_focus = true;
                    Task::none()
                }
                EditorMessage::OpenSettings => {
                    // self.request_windows_switch = Some(WindowAction::Settings)
                    Task::none()
                }
            }
        } else {
            Task::none()
        };
        Ok(task)
    }

    fn view(&self) -> iced::Element<KimunMessage> {
        let editor = text_editor(&self.content)
            .placeholder("Type your Markdown here...")
            .on_action(|a| EditorMessage::Edit(a).into())
            .height(Fill)
            .padding(10)
            .font(Font::MONOSPACE)
            .key_binding(move |kp| {
                self.manage_editor_keys(&kp)
                    .or_else(|| text_editor::Binding::from_key_press(kp))
            })
            .highlight("markdown", highlighter::Theme::Base16Ocean);
        row![editor,].spacing(5).padding(10).into()
    }

    fn key_press(
        &self,
        _key: &iced::keyboard::Key,
        _modifiers: &iced::keyboard::Modifiers,
    ) -> Task<KimunMessage> {
        Task::none()
    }
}
