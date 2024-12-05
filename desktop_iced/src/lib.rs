mod editor;
mod settings;

use std::path::PathBuf;

use core_notes::error::DialogErrors;
use core_notes::nfs::{EntryData, NoteEntry, NotePath};
use core_notes::NoteVault;
use editor::Editor;
// use filtered_list::row::RowItem;
use iced::futures::channel::mpsc::Sender;
use iced::futures::{SinkExt, Stream};
use iced::keyboard::{key, Modifiers};
use iced::widget::{horizontal_space, row, text, text_editor};
use iced::{stream, Subscription};
use iced::{
    widget::{self},
    Element, Task, Theme,
};
use log::{debug, error};
use settings::Settings;

pub type IcedSender = iced::futures::channel::mpsc::Sender<Message>;

#[derive(Debug, Clone)]
pub enum Message {
    Ready(Sender<Message>),
    WorkspaceOpened(Result<PathBuf, DialogErrors>),
    EditorAction(text_editor::Action),
    NoteEntryChosen(usize, NoteEntry),
    Test,
}

pub trait AppScreen {
    fn get_view(&self) -> Element<Message>;
    fn update(&mut self, action: Message) -> Task<Message>;
    fn subscription(&self) {}
}

pub struct DesktopApp {
    async_sender: Option<Sender<Message>>,
    current_screen: Option<Box<dyn AppScreen>>,
    settings: Settings,
}

// impl RowItem for NoteEntry {
//     fn get_view(&self) -> Element<Message> {
//         row![text(self.to_string().clone())].padding(2).into()
//     }
//
//     fn get_sort_string(&self) -> String {
//         match &self.data {
//             EntryData::Note(_note_data) => format!("2{}", self.path_string),
//             EntryData::Directory(_directory_data) => {
//                 format!("1{}", self.path_string)
//             }
//             EntryData::Attachment => format!("3{}", self.path_string),
//         }
//     }
//
//     fn get_message(&self) -> filtered_list::row::RowMessage {
//         todo!()
//     }
// }
//
// impl RowItem for String {
//     fn get_view(&self) -> Element<Message> {
//         row![text(self.clone())].padding(2).into()
//     }
//
//     fn get_sort_string(&self) -> String {
//         self.to_owned()
//     }
//
//     fn get_message(&self) -> filtered_list::row::RowMessage {
//         todo!()
//     }
// }

impl DesktopApp {
    pub fn start() -> iced::Result {
        iced::application("Noters", DesktopApp::update, DesktopApp::view)
            .subscription(DesktopApp::subscription)
            .window_size((500.0, 500.0))
            .theme(DesktopApp::theme)
            .run_with(DesktopApp::new)
    }

    fn new() -> (Self, Task<Message>) {
        let settings = Settings::load().unwrap();
        // let vault = NoteVault::new(&settings.workspace_dir.unwrap());
        // if let Some(workspace_dir) = settings.workspace_dir {}

        let task = match &settings.workspace_dir {
            Some(path) => Task::done(Message::WorkspaceOpened(Ok(path.to_owned()))),
            // TODO: Replace this with opening the settings window
            None => Task::perform(pick_workspace(), Message::WorkspaceOpened),
        };
        (
            Self {
                async_sender: None,
                current_screen: None,
                settings,
            },
            task,
        )
    }

    fn init(&mut self, sender: Sender<Message>) {
        // self.filtered_list = Some(FilteredList::new(sender.clone()));
        // self.filtered_list
        //     .as_mut()
        //     .unwrap()
        //     .add_element("uno".to_string());
        // self.filtered_list
        //     .as_mut()
        //     .unwrap()
        //     .add_element("deux".to_string());
        // self.filtered_list
        //     .as_mut()
        //     .unwrap()
        //     .add_element("tre".to_string());
        // self.filtered_list
        //     .as_mut()
        //     .unwrap()
        //     .add_element("four".to_string());
        // self.filtered_list
        //     .as_mut()
        //     .unwrap()
        //     .add_element("ciinc".to_string());
        // self.filtered_list
        //     .as_mut()
        //     .unwrap()
        //     .add_element("seis".to_string());
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Ready(sender) => {
                debug!("App Ready");
                self.init(sender.clone());
                self.async_sender = Some(sender);
                Task::none()
            }
            Message::WorkspaceOpened(path_buf) => {
                if let Ok(Ok(vault)) = path_buf.map(NoteVault::new) {
                    debug!("Opening Editor");
                    let editor = Editor::new(vault, NotePath::root());
                    self.current_screen = Some(Box::new(editor));
                };
                Task::none()
            }
            _ => {
                if let Some(view) = self.current_screen.as_mut() {
                    view.update(message)
                } else {
                    Task::none()
                }
            }
        }
        // match message {
        //     Message::Ready(sender) => {}
        //
        //     Message::WorkspaceOpened(res) => match res {
        //         Ok(path) => {
        //             if let Err(e) = self.settings.set_workspace(path) {
        //                 println!("{}", e);
        //             }
        //             Task::none()
        //         }
        //         Err(_e) => Task::none(),
        //     },
        //     Message::EditorAction(action) => self.editor.update(action),
        //     Message::FilterAction(message) => {
        //         // if let Some(filtered_list) = self.filtered_list.as_mut() {
        //         //     filtered_list.update(message)
        //         // } else {
        //         //     Task::none()
        //         // }
        //         Task::none()
        //     }
        //     Message::Test => {
        //         debug!("Testing");
        //         Task::none()
        //     }
        // }
    }

    fn view(&self) -> Element<'_, Message> {
        if let Some(view) = self.current_screen.as_ref() {
            view.get_view()
        } else {
            widget::container(horizontal_space()).into()
        }
        // let editor = self.editor.get_view();
        // let row = if let Some(filter_view) = &self.filtered_list {
        //     let filter = filter_view.get_view();
        //     row![editor, filter]
        // } else {
        //     row![editor]
        // };

        // let row = row![editor];
        // let w = widget::container(row.spacing(10)).padding(10).into();
        // w
    }

    fn theme(&self) -> Theme {
        Theme::CatppuccinMocha
    }

    fn worker() -> impl Stream<Item = Message> {
        stream::channel(100, |mut output| async move {
            debug!("Worker Started");
            // let (sender, mut receiver) = mpsc::channel(100);
            if let Err(e) = output.send(Message::Ready(output.clone())).await {
                error!("{}", e);
            }
        })
    }

    fn subscription(&self) -> Subscription<Message> {
        let s1 = Subscription::run(Self::worker);
        let s2 = iced::keyboard::on_key_press(|key, modifier| match (key.as_ref(), modifier) {
            (key::Key::Character("o"), Modifiers::COMMAND) => {
                debug!("Pressed the O key");
                None
            }
            _ => None,
        });

        Subscription::batch(vec![s1, s2])
    }
}

async fn pick_workspace() -> Result<PathBuf, DialogErrors> {
    let handle = rfd::AsyncFileDialog::new()
        .set_title("Choose a workspace directory")
        .pick_folder()
        .await
        .ok_or(DialogErrors::DialogClosed)?;

    Ok(handle.path().to_owned())
}
