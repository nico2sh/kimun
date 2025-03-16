mod components;
mod editor;
mod icons;
mod modals;
mod settings;

use std::path::PathBuf;

use components::filtered_list::VaultListMessage;
use editor::{Editor, EditorMessage};
use iced::{
    Color, Element, Subscription, Task,
    futures::{SinkExt, Stream, channel::mpsc::Sender, stream},
    keyboard::{Key, Modifiers, key},
    widget::{center, column, container, mouse_area, opaque, stack, text_editor},
};
use icons::ICON_BYTES;
use kimun_core::NoteVault;
use log::{debug, error};
use modals::{ModalManager, Modals};
use settings::{Settings, page::SettingsPage};

fn main() -> iced::Result {
    env_logger::Builder::new()
        .filter(Some("kimun_"), log::LevelFilter::max())
        .init();

    iced::application("Kimun Editor", DesktopApp::update, DesktopApp::view)
        .subscription(DesktopApp::subscription)
        .window_size((800.0, 600.0))
        .theme(DesktopApp::theme)
        .font(include_bytes!("../res/fonts/InterVariable.ttf").as_slice())
        .font(include_bytes!("../res/fonts/FiraCode-Regular.ttf").as_slice())
        .font(ICON_BYTES)
        .run_with(DesktopApp::new)
}

#[derive(Debug, Clone)]
enum KimunMessage {
    Ready(Sender<KimunMessage>),
    KeyPresses(Key, Modifiers),
    EditorMessage(EditorMessage),
    ListViewMessage(VaultListMessage),
    CloseModal,
    ShowModal(Modals),
}

struct DesktopApp {
    sender: Option<Sender<KimunMessage>>,
    current_page: Box<dyn KimunPage>,
    modal_manager: ModalManager,
    settings: Settings,
}

impl DesktopApp {
    fn new() -> (Self, Task<KimunMessage>) {
        let settings = Settings::load_from_disk().unwrap_or_default();
        let current_page = Box::new(EmptyPage {});
        let modal_manager = ModalManager::new();
        (
            Self {
                sender: None,
                current_page,
                modal_manager,
                settings,
            },
            Task::none(),
        )
    }

    fn get_first_view(
        workspace_dir: &PathBuf,
        settings: &Settings,
    ) -> anyhow::Result<Box<dyn KimunPage>> {
        let last_note = settings.last_paths.last().and_then(|path| {
            if !path.is_note() {
                None
            } else {
                Some(path.to_owned())
            }
        });

        let vault = NoteVault::new(workspace_dir)?;
        let view: Box<dyn KimunPage> = match last_note {
            Some(path) => Box::new(Editor::new(&vault, &path, settings)?),
            None => Box::new(NoNotePage::new(&vault)),
        };
        Ok(view)
    }

    fn initialize(&mut self, sender: Sender<KimunMessage>) {
        self.sender = Some(sender);
        let current_page = match &self.settings.workspace_dir {
            Some(workspace_dir) => Self::get_first_view(workspace_dir, &self.settings)
                .unwrap_or_else(|_e| Box::new(ErrorPage::new())),
            None => Box::new(SettingsPage::new()),
        };
        self.current_page = current_page;
    }

    fn update(&mut self, message: KimunMessage) -> Task<KimunMessage> {
        match &message {
            KimunMessage::Ready(sender) => {
                self.initialize(sender.to_owned());
                Task::none()
            }
            KimunMessage::KeyPresses(key, modifiers) => {
                // We send key presses to the active view
                if let Some(modal) = &self.modal_manager.current_modal {
                    modal.key_press(key, modifiers)
                } else {
                    self.current_page.key_press(key, modifiers)
                }
            }
            KimunMessage::ShowModal(modal) => self.modal_manager.set_modal(modal.to_owned()),
            KimunMessage::CloseModal => self.modal_manager.close_modal(),
            _ => {
                // update modal or view
                if let Some(modal) = self.modal_manager.current_modal.as_mut() {
                    modal.update(message).unwrap_or_else(|e| {
                        error!("Error updating the modal {}", e);
                        Task::none()
                    })
                } else {
                    self.current_page.update(message).unwrap_or_else(|e| {
                        error!("Error updating the page {}", e);
                        Task::none()
                    })
                }
            }
        }
    }

    fn view(&self) -> Element<KimunMessage> {
        if let Some(modal_view) = &self.modal_manager.current_modal {
            let mv = container(modal_view.view())
                .width(600)
                .height(800)
                .padding(2)
                .style(container::rounded_box);
            stack![
                self.current_page.view(),
                opaque(
                    mouse_area(center(opaque(mv)).style(|_theme| {
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
                    }))
                    .on_press(KimunMessage::CloseModal)
                )
            ]
            .into()
        } else {
            self.current_page.view()
        }
    }

    fn worker() -> impl Stream<Item = KimunMessage> {
        iced::stream::channel(100, |mut output| async move {
            debug!("Worker Started");
            // let (sender, mut receiver) = mpsc::channel(100);
            if let Err(e) = output.send(KimunMessage::Ready(output.clone())).await {
                error!("Error Initializing the app {}", e);
            }
        })
    }

    fn subscription(&self) -> Subscription<KimunMessage> {
        let s1 = Subscription::run(Self::worker);
        let s2 = iced::keyboard::on_key_press(|key, modifier| {
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
        Subscription::batch(vec![s1, s2])
    }

    fn theme(&self) -> iced::Theme {
        iced::Theme::GruvboxDark
    }
}

trait KimunPage {
    fn update(&mut self, message: KimunMessage) -> anyhow::Result<Task<KimunMessage>>;
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

impl KimunPage for NoNotePage {
    fn update(&mut self, message: KimunMessage) -> anyhow::Result<Task<KimunMessage>> {
        Ok(Task::none())
    }

    fn view(&self) -> Element<KimunMessage> {
        column![].into()
    }

    fn key_press(
        &self,
        key: &iced::keyboard::Key,
        modifiers: &iced::keyboard::Modifiers,
    ) -> Task<KimunMessage> {
        Task::none()
    }
}

struct ErrorPage {}

impl ErrorPage {
    pub fn new() -> Self {
        Self {}
    }
}

impl KimunPage for ErrorPage {
    fn update(&mut self, message: KimunMessage) -> anyhow::Result<Task<KimunMessage>> {
        Ok(Task::none())
    }

    fn view(&self) -> Element<KimunMessage> {
        column![].into()
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

impl KimunPage for EmptyPage {
    fn update(&mut self, message: KimunMessage) -> anyhow::Result<Task<KimunMessage>> {
        Ok(Task::none())
    }

    fn view(&self) -> Element<KimunMessage> {
        column![].into()
    }

    fn key_press(
        &self,
        key: &iced::keyboard::Key,
        modifiers: &iced::keyboard::Modifiers,
    ) -> Task<KimunMessage> {
        todo!()
    }
}
