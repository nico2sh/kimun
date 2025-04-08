mod markdown_viewer;

use std::{collections::HashMap, time::Instant};

use iced::{
    Animation,
    Length::Fill,
    Task,
    advanced::graphics::image::image_rs::ImageReader,
    animation, clipboard,
    widget::{container, image, markdown, scrollable},
};
use kimun_core::{
    NoteVault,
    nfs::VaultPath,
    note::{Link, LinkType, NoteDetails},
};
use log::{debug, error};
use markdown_viewer::{CustomViewer, Image, MDError};

use crate::KimunMessage;

use super::EditorMessage;

#[derive(Debug, Clone)]
pub enum PreviewMessage {
    LinkClicked(markdown::Url),
    ImageShown(String),
    ImageDownloaded(markdown::Url, Result<image::Handle, MDError>),
    Copy(String),
}

impl From<PreviewMessage> for KimunMessage {
    fn from(value: PreviewMessage) -> Self {
        KimunMessage::EditorMessage(EditorMessage::PreviewMessage(value))
    }
}

pub struct PreviewPage {
    vault: NoteVault,
    path: VaultPath,
    content: Vec<markdown::Item>,
    md_settings: markdown::Settings,
    note_links: Vec<Link>,
    images: HashMap<markdown::Url, Image>,
    now: Instant,
}

impl PreviewPage {
    pub fn new(text: String, vault: NoteVault, path: VaultPath) -> Self {
        let md = NoteDetails::new(&path, &text).get_markdown_and_links();
        let content = markdown::parse(&md.text).collect();
        let note_links = md.links;
        let md_style = markdown::Style::from_palette(iced::Theme::TokyoNightStorm.palette());
        let md_settings = markdown::Settings::with_style(md_style);
        let images = HashMap::new();
        let now = Instant::now();
        Self {
            vault,
            path,
            content,
            md_settings,
            note_links,
            images,
            now,
        }
    }

    pub fn load_note(&mut self, details: NoteDetails) {
        self.content = markdown::parse(&details.get_markdown_and_links().text).collect();
        self.path = details.path;
    }

    pub fn view(&self) -> iced::Element<crate::KimunMessage> {
        container(
            scrollable(
                markdown::view_with(
                    &self.content,
                    self.md_settings,
                    &CustomViewer {
                        images: &self.images,
                        now: self.now,
                    },
                )
                .map(|url| KimunMessage::EditorMessage(EditorMessage::PreviewMessage(url))),
            )
            .spacing(10)
            .width(Fill)
            .height(Fill),
        )
        .padding(10)
        .into()
    }

    pub fn key_press(
        &self,
        key: &iced::keyboard::Key,
        modifiers: &iced::keyboard::Modifiers,
    ) -> iced::Task<crate::KimunMessage> {
        if let Some(message) = super::manage_editor_hotkeys(key, modifiers, &self.vault, &self.path)
        {
            Task::done(message)
        } else {
            Task::none()
        }
    }

    pub fn update(&mut self, pmessage: PreviewMessage) -> iced::Task<KimunMessage> {
        match pmessage {
            PreviewMessage::LinkClicked(url) => {
                let link = self.note_links.iter().find(|item| item.raw_link.eq(&url));
                if let Some(l) = link {
                    debug!("Link found clicked: {}", url);
                    if let LinkType::Note(path) = &l.ltype {
                        match self.vault.open_or_search(path) {
                            Ok(result) => {
                                debug!("Got {} results", result.len());
                                let message = match result.len() {
                                    0 => EditorMessage::NewNote(path.to_owned()),
                                    1 => {
                                        let path = result.first().unwrap().0.path.clone();
                                        EditorMessage::OpenNote(path)
                                    }
                                    _ => EditorMessage::SelectNote(result),
                                };
                                Task::done(message.into())
                            }
                            Err(e) => {
                                error!("Error: {}", e);
                                Task::none()
                            }
                        }
                    } else {
                        Task::none()
                    }
                } else {
                    debug!("Link clicked: {}", url);
                    Task::none()
                }
            }
            PreviewMessage::ImageShown(url) => {
                if self.images.contains_key(&url) {
                    return Task::none();
                }

                let _ = self.images.insert(url.clone(), Image::Loading);

                Task::perform(download_image(url.clone()), |r| {
                    KimunMessage::EditorMessage(EditorMessage::PreviewMessage(
                        PreviewMessage::ImageDownloaded(url, r),
                    ))
                })
            }
            PreviewMessage::ImageDownloaded(url, result) => {
                let _ = self.images.insert(
                    url,
                    result
                        .map(|handle| Image::Ready {
                            handle,
                            fade_in: Animation::new(false)
                                .quick()
                                .easing(animation::Easing::EaseInOut)
                                .go(true),
                        })
                        .unwrap_or_else(Image::Errored),
                );

                Task::none()
            }
            PreviewMessage::Copy(text) => clipboard::write(text),
        }
    }
}

async fn download_image(url: markdown::Url) -> Result<image::Handle, MDError> {
    use std::io;
    use tokio::task;

    println!("Trying to download image: {url}");

    let client = reqwest::Client::new();

    let bytes = match reqwest::Url::parse(&url) {
        Ok(url) => client
            .get(url)
            .send()
            .await?
            .error_for_status()?
            .bytes()
            .await?
            .to_vec(),
        Err(_e) => {
            // We try to get from a local path
            std::fs::read(url)?
        }
    };

    let image = task::spawn_blocking(move || {
        Ok::<_, MDError>(
            ImageReader::new(io::Cursor::new(bytes))
                .with_guessed_format()?
                .decode()?
                .to_rgba8(),
        )
    })
    .await??;

    Ok(image::Handle::from_rgba(
        image.width(),
        image.height(),
        image.into_raw(),
    ))
}
