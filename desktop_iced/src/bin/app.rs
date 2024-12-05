use noters::DesktopApp;

fn main() -> iced::Result {
    env_logger::Builder::new()
        .filter(Some("noters"), log::LevelFilter::max())
        .init();

    DesktopApp::start()
}

// struct Editor {
//     path: Option<PathBuf>,
//     content: text_editor::Content,
//     error: Option<Error>,
// }
//
// #[derive(Debug, Clone)]
// enum Message {
//     Edit(text_editor::Action),
//     New,
//     Open,
//     Save,
//     FileOpened(Result<(PathBuf, Arc<String>), Error>),
//     FileSaved(Result<PathBuf, Error>),
// }
//
// impl Editor {
//     fn new() -> (Self, Task<Message>) {
//         (
//             Self {
//                 path: None,
//                 content: text_editor::Content::new(),
//                 error: None,
//             },
//             Task::perform(load_file(default_file()), Message::FileOpened),
//         )
//     }
//
//     fn title(&self) -> String {
//         String::from("An editor")
//     }
//
//     fn update(&mut self, message: Message) -> Task<Message> {
//         match message {
//             Message::Edit(action) => {
//                 self.content.perform(action);
//                 self.error = None;
//                 Task::none()
//             }
//             Message::New => {
//                 self.path = None;
//                 self.content = text_editor::Content::new();
//
//                 Task::none()
//             }
//             Message::Open => Task::perform(pick_file(), Message::FileOpened),
//             Message::Save => {
//                 let text = self.content.text();
//
//                 Task::perform(save_file(self.path.clone(), text), Message::FileSaved)
//             }
//             Message::FileOpened(Ok((path, content))) => {
//                 self.path = Some(path);
//                 self.content = text_editor::Content::with_text(&content);
//                 Task::none()
//             }
//             Message::FileSaved(Ok(path)) => {
//                 self.path = Some(path);
//
//                 Task::none()
//             }
//             Message::FileSaved(Err(err)) => {
//                 self.error = Some(err);
//
//                 Task::none()
//             }
//             Message::FileOpened(Err(error)) => {
//                 self.error = Some(error);
//                 Task::none()
//             }
//         }
//     }
//
//     fn view(&self) -> Element<'_, Message> {
//         let controls = row![
//             action(new_icon(), "New", Message::New),
//             action(open_icon(), "Open", Message::Open),
//             action(save_icon(), "Save", Message::Save)
//         ]
//         .spacing(10);
//         let input = text_editor(&self.content)
//             .on_action(Message::Edit)
//             .highlight_with::<Highlighter>(
//                 highlighter::Settings {
//                     theme: highlighter::Theme::SolarizedDark,
//                     token: self
//                         .path
//                         .as_ref()
//                         .and_then(|path| path.extension()?.to_str())
//                         .unwrap_or("rs")
//                         .to_string(),
//                 },
//                 |highlight, _theme| highlight.to_format(),
//             );
//
//         let status_bar = {
//             let status = if let Some(Error::IOFailed(error)) = self.error.as_ref() {
//                 text(error.to_string())
//             } else {
//                 match self.path.as_deref().and_then(Path::to_str) {
//                     Some(path) => text(path).size(14),
//                     None => text("New File"),
//                 }
//             };
//
//             let text_position = {
//                 let (line, column) = self.content.cursor_position();
//
//                 text(format!("{}:{}", line + 1, column + 1))
//             };
//
//             row![status, horizontal_space(), text_position]
//         };
//
//         container(column![controls, input, status_bar].spacing(10))
//             .padding(10)
//             .into()
//     }
//
//     fn theme(&self) -> Theme {
//         Theme::Dark
//     }
// }
//
// fn action<'a>(
//     content: Element<'a, Message>,
//     label: &'a str,
//     on_press: Message,
// ) -> Element<'a, Message> {
//     tooltip(
//         button(container(content).width(30).center_x(Length::Fill))
//             .on_press(on_press)
//             .padding([5, 10]),
//         label,
//         tooltip::Position::FollowCursor,
//     )
//     // .style(theme::Theme::palette::EXTENDED_DARK)
//     .into()
// }
//
// fn new_icon<'a>() -> Element<'a, Message> {
//     icon('\u{E800}')
// }
//
// fn save_icon<'a>() -> Element<'a, Message> {
//     icon('\u{E801}')
// }
//
// fn open_icon<'a>() -> Element<'a, Message> {
//     icon('\u{F115}')
// }
//
// fn icon<'a>(codepoint: char) -> Element<'a, Message> {
//     const ICON_FONT: Font = Font::with_name("noters");
//
//     text(codepoint).font(ICON_FONT).into()
// }
//
// fn default_file() -> PathBuf {
//     PathBuf::from(format!("{}/src/bin/app.rs", env!("CARGO_MANIFEST_DIR")))
// }
//
// async fn pick_file() -> Result<(PathBuf, Arc<String>), Error> {
//     let handle = rfd::AsyncFileDialog::new()
//         .set_title("Choose a text file")
//         .pick_file()
//         .await
//         .ok_or(Error::DialogClosed)?;
//     load_file(handle.path().to_owned()).await
// }
//
// async fn load_file(path: PathBuf) -> Result<(PathBuf, Arc<String>), Error> {
//     let content = tokio::fs::read_to_string(&path)
//         .await
//         .map(|w| Arc::new(w))
//         .map_err(|error| error.kind())
//         .map_err(Error::IOFailed)?;
//
//     Ok((path, content))
// }
//
// async fn save_file(path: Option<PathBuf>, text: String) -> Result<PathBuf, Error> {
//     let path = if let Some(path) = path {
//         path
//     } else {
//         rfd::AsyncFileDialog::new()
//             .set_title("Choose a file name...")
//             .save_file()
//             .await
//             .ok_or(Error::DialogClosed)
//             .map(|handle| handle.path().to_owned())?
//     };
//
//     tokio::fs::write(&path, text)
//         .await
//         .map_err(|error| Error::IOFailed(error.kind()))?;
//
//     Ok(path)
// }
//
// #[derive(Debug, Clone)]
// enum Error {
//     DialogClosed,
//     IOFailed(io::ErrorKind),
// }
