mod components;
mod editor;
mod fonts;
mod icons;
mod modals;
mod settings;

use std::path::PathBuf;

use components::filtered_list::VaultListMessage;
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
        .font(FONT_UI_BYTES)
        .font(FONT_CODE_BYTES)
        .font(ICON_BYTES)
        .run_with(DesktopApp::new)
}

#[derive(Debug, Clone)]
enum AsyncMessage {
    Save,
}

#[derive(Debug, Clone)]
enum KimunMessage {
    Ready,
    Error(String),
    KeyPresses(Key, Modifiers),
    EditorMessage(EditorMessage),
    ListViewMessage(VaultListMessage),
    CloseModal,
    ShowModal(Modals),
}

struct DesktopApp {
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

    fn initialize(&mut self) {
        let current_page = match &self.settings.workspace_dir {
            Some(workspace_dir) => Self::get_first_view(workspace_dir, &self.settings)
                .unwrap_or_else(|_e| Box::new(ErrorPage::new())),
            None => Box::new(SettingsPage::new()),
        };
        self.current_page = current_page;
    }

    fn update(&mut self, message: KimunMessage) -> Task<KimunMessage> {
        match &message {
            KimunMessage::Ready => {
                self.initialize();
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
                if let Some(modal) = self.modal_manager.current_modal.as_mut() {
                    let task1 = modal.update(message.clone());
                    let task2 = self.update_page(message);
                    Task::batch([task1, task2])
                } else {
                    self.update_page(message)
                }

                // update modal or view
                // if let Some(modal) = self.modal_manager.current_modal.as_mut() {
                //     modal.update(message)
                // } else {
                //     self.current_page.update(message).unwrap_or_else(|e| {
                //         error!("Error updating the page {}", e);
                //         Task::none()
                //     })
                // }
            }
        }
    }

    fn update_page(&mut self, message: KimunMessage) -> Task<KimunMessage> {
        self.current_page.update(message).unwrap_or_else(|e| {
            error!("Error updating the page {}", e);
            Task::none()
        })
    }

    fn view(&self) -> Element<KimunMessage> {
        if let Some(modal_view) = &self.modal_manager.current_modal {
            let mv = container(modal_view.view())
                .width(modal_view.get_width())
                .height(modal_view.get_height())
                .padding(2)
                .style(container::rounded_box);
            iced::widget::stack![
                self.current_page.view(),
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
            self.current_page.view()
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
            .map(|time| KimunMessage::EditorMessage(EditorMessage::SaveTick));
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

struct EmptyPage {}

impl KimunPage for EmptyPage {
    fn update(&mut self, message: KimunMessage) -> anyhow::Result<Task<KimunMessage>> {
        Ok(Task::none())
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
