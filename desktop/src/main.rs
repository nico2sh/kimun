mod components;
mod editor;
mod fonts;
mod icons;
mod modals;
mod settings;

use std::{
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

use components::{filtered_list::ListViewMessage, list::RowSelection};
use editor::{Editor, EditorMessage};
use fonts::{FONT_CODE_BYTES, FONT_UI_BYTES};
use iced::{
    Color, Element, Subscription, Task,
    futures::{SinkExt, Stream, channel::mpsc::Sender},
    keyboard::{Key, Modifiers},
    time,
    widget::container,
};
use icons::ICON_BYTES;
use kimun_core::{NoteVault, nfs::VaultPath};
use log::{debug, error};
use modals::{ModalManager, Modals};
use settings::{
    Settings,
    page::{SettingsMessage, SettingsPage},
};

fn main() -> iced::Result {
    env_logger::Builder::new()
        .filter(Some("kimun_"), log::LevelFilter::max())
        .init();

    iced::application(DesktopApp::new, DesktopApp::update, DesktopApp::view)
        .subscription(DesktopApp::subscription)
        .title(DesktopApp::title)
        .theme(DesktopApp::theme)
        .window_size((800.0, 600.0))
        .font(FONT_UI_BYTES)
        .font(FONT_CODE_BYTES)
        .font(ICON_BYTES)
        .run()
    // .run_with(DesktopApp::new)
}

#[derive(Debug, Clone)]
enum AsyncMessage {
    Save,
}

#[derive(Debug, Clone)]
enum KimunState {
    Default,
}

#[derive(Debug, Clone)]
enum KimunMessage {
    Ready,
    Error(ErrorMessage),
    KeyPresses(Key, Modifiers),
    EditorMessage(EditorMessage),
    Select(RowSelection),
    ListViewMessage(ListViewMessage),
    CloseModal,
    ShowModal(Modals),
    OpenPage(KimunPage),
    SettingsChange(SettingsMessage),
    SettingsUpdated(Settings),
}

#[derive(Debug, Clone)]
enum ErrorMessage {
    Add(String),
    Remove(u32),
}

impl KimunMessage {
    pub fn add_error<S: AsRef<str>>(message: S) -> Self {
        KimunMessage::Error(ErrorMessage::Add(message.as_ref().to_string()))
    }
}

#[derive(Debug, Clone)]
enum KimunPage {
    Editor(NoteVault, VaultPath, Settings),
    NoNote(NoteVault),
    Settings,
    Error(String),
}

#[derive(Default)]
struct ErrorMessages {
    last_id: u32,
    messages: Vec<(u32, String)>,
}

impl ErrorMessages {
    fn add_message<S: AsRef<str>>(&mut self, message: S) -> Task<KimunMessage> {
        self.last_id += 1;
        self.messages
            .push((self.last_id, message.as_ref().to_string()));
        let id = self.last_id;
        Task::perform(
            async move {
                std::thread::sleep(std::time::Duration::from_secs(3));
                id
            },
            |id| KimunMessage::Error(ErrorMessage::Remove(id)),
        )
    }

    fn remove_message(&mut self, id: &u32) {
        if let Some(position) = self.messages.iter_mut().position(|(m_id, _)| m_id == id) {
            self.messages.remove(position);
        }
    }

    fn get_messages(&self) -> impl Iterator<Item = String> {
        self.messages
            .iter()
            .map(|(_id, message)| message.to_string())
    }
}

struct DesktopApp {
    current_page: Box<dyn KimunPageView>,
    modal_manager: ModalManager,
    settings: Settings,
    error_messages: ErrorMessages,
}

impl DesktopApp {
    fn new() -> (Self, Task<KimunMessage>) {
        let settings = Settings::load_from_disk().unwrap_or_default();
        let current_page = Box::new(EmptyPage {});
        let modal_manager = ModalManager::new();
        (
            Self {
                current_page,
                modal_manager,
                settings,
                error_messages: ErrorMessages::default(),
            },
            Task::none(),
        )
    }

    fn title(&self) -> String {
        "Kimun".to_string()
    }

    fn get_first_view(workspace_dir: &PathBuf, settings: &Settings) -> KimunPage {
        let last_note = settings.last_paths.last().and_then(|path| {
            if !path.is_note() {
                None
            } else {
                Some(path.to_owned())
            }
        });

        let vault_res = NoteVault::new(workspace_dir);

        match vault_res {
            Ok(vault) => {
                match last_note {
                    Some(path) => {
                        // An Editor view
                        KimunPage::Editor(vault, path, settings.to_owned())
                    }
                    None => KimunPage::NoNote(vault),
                }
            }
            Err(e) => KimunPage::Error(e.to_string()),
        }
    }

    fn initialize(&mut self) -> Task<KimunMessage> {
        let current_page = match &self.settings.workspace_dir {
            Some(workspace_dir) => Self::get_first_view(workspace_dir, &self.settings),
            None => KimunPage::Settings,
        };
        Task::done(KimunMessage::OpenPage(current_page))
    }

    fn update(&mut self, message: KimunMessage) -> Task<KimunMessage> {
        match &message {
            KimunMessage::Ready => self.initialize(),
            KimunMessage::SettingsUpdated(settings) => {
                self.settings = settings.clone();
                Task::none()
            }
            KimunMessage::KeyPresses(key, modifiers) => {
                if matches!(
                    (key.as_ref(), modifiers),
                    (Key::Character(","), &Modifiers::COMMAND)
                ) {
                    Task::batch([
                        self.modal_manager.close_modal(),
                        Task::done(KimunMessage::OpenPage(KimunPage::Settings)),
                    ])
                } else
                // We send key presses to the active view
                if let Some(modal) = &self.modal_manager.current_modal {
                    modal.key_press(key, modifiers)
                } else {
                    self.current_page.key_press(key, modifiers)
                }
            }
            KimunMessage::ShowModal(modal) => self.modal_manager.set_modal(modal.to_owned()),
            KimunMessage::CloseModal => self.modal_manager.close_modal(),
            KimunMessage::OpenPage(page) => {
                // We open a page
                match page {
                    KimunPage::Editor(vault, vault_path, settings) => {
                        let editor_res = Editor::new(vault, vault_path, settings);
                        match editor_res {
                            Ok(editor) => {
                                self.current_page = Box::new(editor);
                                Task::none()
                            }
                            Err(e) => {
                                let error_page = ErrorPage::new(e.to_string());
                                self.current_page = Box::new(error_page);

                                Task::none()
                            }
                        }
                    }
                    KimunPage::NoNote(vault) => {
                        let page = NoNotePage::new(vault);
                        self.current_page = Box::new(page);

                        Task::none()
                    }
                    KimunPage::Settings => {
                        let settings_page = SettingsPage::new();
                        self.current_page = Box::new(settings_page);

                        Task::none()
                    }
                    KimunPage::Error(e) => {
                        let error_page = ErrorPage::new(e);
                        self.current_page = Box::new(error_page);

                        Task::none()
                    }
                }
            }
            KimunMessage::Error(error_message) => {
                match error_message {
                    ErrorMessage::Add(message) => self.error_messages.add_message(message),
                    ErrorMessage::Remove(id) => {
                        self.error_messages.remove_message(id);
                        Task::none()
                    }
                }
                // We show the error
            }
            _ => {
                if let Some(modal) = self.modal_manager.current_modal.as_mut() {
                    let task1 = modal.update(message.clone());
                    let task2 = self.update_page(message);
                    Task::batch([task1, task2])
                } else {
                    self.update_page(message)
                }
            }
        }
    }

    fn update_page(&mut self, message: KimunMessage) -> Task<KimunMessage> {
        self.current_page.update(message)
    }

    fn view(&self) -> Element<KimunMessage> {
        let main_view = if self.error_messages.messages.is_empty() {
            self.current_page.view()
        } else {
            iced::widget::column![
                self.current_page.view(),
                iced::widget::container(iced::widget::text(
                    self.error_messages
                        .get_messages()
                        .collect::<Vec<String>>()
                        .join("\n")
                ))
            ]
            .into()
        };

        if let Some(modal_view) = &self.modal_manager.current_modal {
            let mv = container(modal_view.view())
                .width(modal_view.get_width())
                .height(modal_view.get_height())
                .padding(2)
                .style(container::rounded_box);
            iced::widget::stack![
                main_view,
                iced::widget::opaque(
                    iced::widget::mouse_area(iced::widget::center(iced::widget::opaque(mv)).style(
                        |_theme| {
                            container::Style {
                                background: Some(
                                    Color {
                                        a: 0.8,
                                        ..Color::BLACK
                                    }
                                    .into(),
                                ),
                                ..container::Style::default()
                            }
                        }
                    ))
                    .on_press(KimunMessage::CloseModal)
                )
            ]
            .into()
        } else {
            main_view.into()
        }
    }

    fn worker() -> impl Stream<Item = KimunMessage> {
        iced::stream::channel(100, |mut output: Sender<KimunMessage>| async move {
            debug!("Worker Started");
            // We execute whatever we need to initialize
            if let Err(e) = output.send(KimunMessage::Ready).await {
                error!("Error Initializing the app {}", e);
            }

            // loop {
            //     // Read next input sent from `Application`
            //     let input = receiver.select_next_some().await;
            //
            //     match input {
            //         AsyncMessage::Save => {
            //             // Do some async work...
            //
            //             // Finally, we can optionally produce a message to tell the
            //             // `Application` the work is done
            //             // output.send(Event::WorkFinished).await;
            //         }
            //     }
            // }
        })
    }

    fn subscription(&self) -> Subscription<KimunMessage> {
        let init = Subscription::run(Self::worker);
        let save_tick = time::every(std::time::Duration::from_secs(5))
            .map(|_time| KimunMessage::EditorMessage(EditorMessage::SaveTick));
        let key_capture = iced::keyboard::on_key_press(|key, modifier| {
            Some(KimunMessage::KeyPresses(key, modifier))
            // None

            //     match (key.as_ref(), modifier) {
            //     (key::Key::Character("o"), Modifiers::COMMAND) => {
            //         debug!("Pressed the O key");
            //         None
            //     }
            //     _ => None,
            // }
        });
        Subscription::batch(vec![init, save_tick, key_capture])
    }

    fn theme(&self) -> iced::Theme {
        self.settings.theme.to_owned()
    }
}

trait KimunPageView {
    fn update(&mut self, message: KimunMessage) -> Task<KimunMessage>;
    fn view(&self) -> Element<KimunMessage>;
    fn key_press(
        &self,
        key: &iced::keyboard::Key,
        modifiers: &iced::keyboard::Modifiers,
    ) -> Task<KimunMessage>;
}

struct NoNotePage {}

impl NoNotePage {
    fn new(vault: &NoteVault) -> Self {
        todo!()
    }
}

impl KimunPageView for NoNotePage {
    fn update(&mut self, _message: KimunMessage) -> Task<KimunMessage> {
        Task::none()
    }

    fn view(&self) -> Element<KimunMessage> {
        iced::widget::column![].into()
    }

    fn key_press(
        &self,
        key: &iced::keyboard::Key,
        modifiers: &iced::keyboard::Modifiers,
    ) -> Task<KimunMessage> {
        Task::none()
    }
}

struct ErrorPage {
    message: String,
}

impl ErrorPage {
    pub fn new<S: AsRef<str>>(message: S) -> Self {
        let message = message.as_ref().to_string();
        Self { message }
    }
}

impl KimunPageView for ErrorPage {
    fn update(&mut self, _message: KimunMessage) -> Task<KimunMessage> {
        Task::none()
    }

    fn view(&self) -> Element<KimunMessage> {
        iced::widget::column![
            iced::widget::text("There has been an error:")
                .align_x(iced::alignment::Horizontal::Center),
            iced::widget::text(&self.message).align_x(iced::alignment::Horizontal::Center)
        ]
        .spacing(20)
        .width(iced::Length::Fill)
        .into()
    }

    fn key_press(
        &self,
        key: &iced::keyboard::Key,
        modifiers: &iced::keyboard::Modifiers,
    ) -> Task<KimunMessage> {
        Task::none()
    }
}

struct EmptyPage {}

impl KimunPageView for EmptyPage {
    fn update(&mut self, message: KimunMessage) -> Task<KimunMessage> {
        Task::none()
    }

    fn view(&self) -> Element<KimunMessage> {
        iced::widget::column![].into()
    }

    fn key_press(
        &self,
        key: &iced::keyboard::Key,
        modifiers: &iced::keyboard::Modifiers,
    ) -> Task<KimunMessage> {
        todo!()
    }
}
