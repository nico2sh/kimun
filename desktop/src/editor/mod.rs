mod preview;

use std::{cell::RefCell, rc::Rc};

use anyhow::bail;
use iced::{
    Font,
    Length::Fill,
    Padding, Task, highlighter,
    keyboard::{self, Key, Modifiers, key::Named},
    widget::{row, text_editor},
};
use kimun_core::{
    NoteVault,
    nfs::{NoteEntryData, VaultPath},
    note::{NoteContentData, NoteDetails},
};
use log::{debug, error};
use preview::{PreviewMessage, PreviewPage};

use crate::{
    KimunMessage, KimunPageView,
    modals::Modals,
    settings::Settings,
    style_units::{SMALL_PADDING, SMALL_SPACING},
};

#[derive(Clone, Debug)]
pub enum EditorMsg {
    Edit(text_editor::Action),
    SelectNote(Vec<(NoteEntryData, NoteContentData)>),
    OpenNote(VaultPath),
    NewNote(VaultPath),
    NewJournal,
    SaveTick,
    Save,
    Saved(VaultPath),
    PreviewMessage(PreviewMessage),
    ToggleView,
    Undo,
    Redo,
}

#[derive(PartialEq, Eq)]
enum SaveStatus {
    Saved,
    Saving,
    NotSaved,
}

impl From<EditorMsg> for KimunMessage {
    fn from(value: EditorMsg) -> Self {
        KimunMessage::EditorMessage(value)
    }
}

pub struct Editor {
    note_details: NoteDetails,
    content: text_editor::Content,
    preview_page: Option<PreviewPage>,
    save_status: SaveStatus,
    vault: NoteVault,
    path: VaultPath,
    settings: Rc<RefCell<Settings>>,
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
        settings: Rc<RefCell<Settings>>,
    ) -> anyhow::Result<Self> {
        let note_details = vault.load_note(path)?;
        let content = text_editor::Content::with_text(&note_details.raw_text);
        settings.borrow_mut().add_path_history(path);
        if let Err(e) = settings.borrow().save_to_disk() {
            error!("Failed updating the last paths in settings: {}", e);
        }
        Ok(Self {
            note_details,
            content,
            preview_page: None,
            save_status: SaveStatus::Saved,
            vault: vault.to_owned(),
            path: path.to_owned(),
            settings: settings.to_owned(),
        })
    }

    /// Loads a note from the path
    /// if the path is a note, then we load the note in the current view
    /// if not, we return an error
    fn load_note_path(&mut self, note_path: &VaultPath) -> anyhow::Result<()> {
        // We will save the current note
        if note_path.is_note() && self.vault.exists(note_path).is_some() {
            // TODO: send the save message no matter if loading fails
            let note_details = self.vault.load_note(note_path)?;
            self.settings.borrow_mut().add_path_history(note_path);
            // We save but we don't throw the error, just log it
            if let Err(e) = self.settings.borrow().save_to_disk() {
                error!("Error saving settings to disk: {}", e);
            }
            self.set_content(&note_details);
            self.path = note_path.to_owned();
            self.save_status = SaveStatus::Saved;
            if let Some(preview) = self.preview_page.as_mut() {
                preview.load_note(note_details);
            }
        } else {
            bail!(
                "Note path is not a note or vault path doesn't exist: {}",
                note_path
            )
        };
        Ok(())
    }

    fn save_task(&mut self) -> Task<KimunMessage> {
        self.save_status = SaveStatus::Saving;
        let vault = self.vault.clone();
        let path = self.path.clone();
        let content = self.content.text();
        Task::perform(
            async move { vault.save_note(&path, content) },
            |res| match res {
                Ok((entry, _content)) => EditorMsg::Saved(entry.path).into(),
                Err(e) => KimunMessage::add_error(format!("Error saving note: {}", e)),
            },
        )
    }

    /// Creates a note from the path
    fn new_note_at_path(&mut self, note_path: &VaultPath) -> anyhow::Result<()> {
        // We will save the current note
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
            self.save_status = SaveStatus::Saved;
            self.set_content(&details);
        } else {
            bail!("Note path is not a note: {}", np)
        };

        Ok(())
    }

    fn set_content(&mut self, details: &NoteDetails) {
        self.content = text_editor::Content::with_text(&details.raw_text);
        self.note_details = details.to_owned();
    }

    // fn chain_preview_message(&mut self, message: KimunMessage) -> Task<KimunMessage> {
    //
    // }
}

pub fn manage_editor_hotkeys(
    key: &keyboard::Key,
    modifiers: &keyboard::Modifiers,
    vault: &NoteVault,
    path: &VaultPath,
) -> Option<KimunMessage> {
    match (key.as_ref(), modifiers) {
        (Key::Character("k"), &Modifiers::COMMAND) => Some(KimunMessage::ShowModal(
            Modals::VaultSearch(vault.to_owned()),
        )),
        (Key::Character("o"), &Modifiers::COMMAND) => {
            let current_path = path.get_parent_path().0;
            Some(KimunMessage::ShowModal(Modals::VaultBrowse(
                vault.to_owned(),
                current_path.to_owned(),
            )))
        }
        (Key::Character("j"), &Modifiers::COMMAND) => Some(EditorMsg::NewJournal.into()),
        (Key::Character("p"), &Modifiers::COMMAND) => Some(EditorMsg::ToggleView.into()),
        _ => None,
    }
}

pub fn manage_editor_keystrokes(
    key: &keyboard::Key,
    modifiers: &keyboard::Modifiers,
) -> Option<text_editor::Binding<KimunMessage>> {
    if modifiers.command() {
        // Command/Control pressed
        if key.as_ref().eq(&Key::Character("z")) {
            if modifiers.shift() {
                Some(text_editor::Binding::Custom(EditorMsg::Redo.into()))
            } else {
                Some(text_editor::Binding::Custom(EditorMsg::Undo.into()))
            }
        } else {
            None
        }
    } else if key.as_ref().eq(&Key::Named(Named::Tab)) {
        // NO Command/Control pressed
        // TODO: Manage indenting
        // We insert spaces instead of tabs
        let _tab: Option<text_editor::Binding<KimunMessage>> =
            Some(text_editor::Binding::Insert('\t'));
        let spaces: Option<text_editor::Binding<KimunMessage>> = Some(
            text_editor::Binding::Sequence(vec![text_editor::Binding::Insert(' '); 4]),
        );
        spaces
    } else {
        None
    }
}

impl KimunPageView for Editor {
    fn update(&mut self, message: crate::KimunMessage) -> Task<KimunMessage> {
        let task = if let KimunMessage::EditorMessage(message) = message {
            match message {
                EditorMsg::Edit(action) => {
                    // debug!("Action: {:?}", action);
                    if self.save_status != SaveStatus::NotSaved && action.is_edit() {
                        self.save_status = SaveStatus::NotSaved;
                    }

                    self.content.perform(action);

                    Task::none()
                }
                EditorMsg::Undo => {
                    // Implement Undo
                    Task::none()
                }
                EditorMsg::Redo => {
                    // Implement Redo
                    Task::none()
                }
                EditorMsg::SelectNote(notes) => {
                    // We select notes
                    Task::done(KimunMessage::ShowModal(Modals::NoteSelect(notes)))
                }
                EditorMsg::OpenNote(note_path) => {
                    debug!("Loading note at path {}", note_path);
                    let save_task = self.save_task();

                    match self.load_note_path(&note_path) {
                        Ok(_) => save_task,
                        Err(e) => Task::batch([
                            save_task,
                            Task::done(KimunMessage::add_error(e.to_string())),
                        ]),
                    }
                }
                EditorMsg::NewJournal => {
                    debug!("New journal entry");
                    let save_task = self.save_task();

                    match self
                        .vault
                        .journal_entry()
                        .map(|(details, _)| self.load_note_path(&details.path))
                    {
                        Ok(_) => save_task,
                        Err(e) => Task::batch([
                            save_task,
                            Task::done(KimunMessage::add_error(e.to_string())),
                        ]),
                    }
                }
                EditorMsg::NewNote(note_path) => {
                    debug!("New note at: {}", note_path);
                    let save_task = self.save_task();
                    self.preview_page = None;

                    match self.new_note_at_path(&note_path) {
                        Ok(_) => save_task,
                        Err(e) => Task::batch([
                            save_task,
                            Task::done(KimunMessage::add_error(e.to_string())),
                        ]),
                    }
                }
                EditorMsg::SaveTick => {
                    if self.save_status == SaveStatus::NotSaved {
                        Task::done(EditorMsg::Save.into())
                    } else {
                        Task::none()
                    }
                }
                EditorMsg::Save => self.save_task(),
                EditorMsg::Saved(path) => {
                    // Since saving happens asynchronously, we may have saved a note before opening
                    // another one in a new path, so we check we are marking as saved the current
                    debug!("Just saved: {}", path);
                    if self.path.eq(&path) && self.save_status == SaveStatus::Saving {
                        self.save_status = SaveStatus::Saved;
                    }
                    Task::none()
                }
                EditorMsg::ToggleView => {
                    if self.preview_page.is_none() {
                        self.preview_page = Some(PreviewPage::new(
                            self.content.text(),
                            self.vault.clone(),
                            self.path.clone(),
                        ));
                    } else {
                        self.preview_page = None;
                    }
                    Task::none()
                }
                EditorMsg::PreviewMessage(pmessage) => {
                    // self.change_viewer(viewer_type)?;
                    // self.request_focus = true;
                    if let Some(preview) = self.preview_page.as_mut() {
                        preview.update(pmessage)
                    } else {
                        Task::none()
                    }
                }
            }
        } else {
            Task::none()
        };

        task
    }

    fn view(&self) -> iced::Element<KimunMessage> {
        if let Some(preview_page) = &self.preview_page {
            preview_page.view()
        } else {
            let path = iced::widget::text(self.path.to_string());
            let path_label = match self.save_status {
                SaveStatus::Saved => row![path],
                SaveStatus::Saving => row![path, iced::widget::text("+")],
                SaveStatus::NotSaved => row![path, iced::widget::text("*")],
            }
            .padding(Padding {
                top: 0.0,
                right: 0.0,
                bottom: 0.0,
                left: SMALL_PADDING as f32,
            });
            let editor = text_editor(&self.content)
                .placeholder("Type your Markdown here...")
                .on_action(|a| EditorMsg::Edit(a).into())
                .height(Fill)
                .font(Font::MONOSPACE)
                .key_binding(move |kp| {
                    if matches![kp.status, text_editor::Status::Focused { is_hovered: _ }] {
                        manage_editor_keystrokes(&kp.key, &kp.modifiers)
                    } else {
                        None
                    }
                    .or(manage_editor_hotkeys(
                        &kp.key,
                        &kp.modifiers,
                        &self.vault,
                        &self.path,
                    )
                    .map(text_editor::Binding::Custom)
                    .or_else(|| text_editor::Binding::from_key_press(kp)))
                })
                .highlight("markdown", highlighter::Theme::Base16Ocean);
            iced::widget::column![editor, path_label]
                .spacing(SMALL_SPACING)
                .into()
        }
    }

    fn key_press(
        &self,
        key: &iced::keyboard::Key,
        modifiers: &iced::keyboard::Modifiers,
    ) -> Task<KimunMessage> {
        if let Some(preview_page) = &self.preview_page {
            preview_page.key_press(key, modifiers)
        } else {
            Task::none()
        }
    }
}
