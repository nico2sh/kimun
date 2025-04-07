use iced::advanced::graphics::image::image_rs::ImageError;
use iced::advanced::graphics::image::image_rs::ImageReader;
use iced::animation;
use iced::clipboard;
use iced::highlighter;
use iced::time::{self, Instant, milliseconds};
use iced::widget::{
    self, button, center_x, container, horizontal_space, hover, image, markdown, pop, right, row,
    scrollable, text_editor, toggler,
};
use iced::window;
use iced::{Animation, Element, Fill, Font, Function, Subscription, Task, Theme};
use reqwest::Url;

use std::collections::HashMap;
use std::io;
use std::sync::Arc;

use super::PreviewMessage;

enum Image {
    Loading,
    Ready {
        handle: image::Handle,
        fade_in: Animation<bool>,
    },
    #[allow(dead_code)]
    Errored(Error),
}

struct CustomViewer<'a> {
    images: &'a HashMap<markdown::Url, Image>,
    now: Instant,
}

impl<'a> markdown::Viewer<'a, PreviewMessage> for CustomViewer<'a> {
    fn on_link_click(url: markdown::Url) -> PreviewMessage {
        PreviewMessage::LinkClicked(url)
    }

    fn image(
        &self,
        _settings: markdown::Settings,
        url: &'a markdown::Url,
        _title: &'a str,
        _alt: &markdown::Text,
    ) -> Element<'a, PreviewMessage> {
        if let Some(Image::Ready { handle, fade_in }) = self.images.get(url) {
            center_x(
                image(handle)
                    .opacity(fade_in.interpolate(0.0, 1.0, self.now))
                    .scale(fade_in.interpolate(1.2, 1.0, self.now)),
            )
            .into()
        } else {
            pop(horizontal_space())
                .key(url.as_str())
                .delay(milliseconds(500))
                .on_show(|_size| PreviewMessage::ImageShown(url.clone()))
                .into()
        }
    }

    fn code_block(
        &self,
        settings: markdown::Settings,
        _language: Option<&'a str>,
        code: &'a str,
        lines: &'a [markdown::Text],
    ) -> Element<'a, PreviewMessage> {
        let code_block = markdown::code_block(settings, lines, PreviewMessage::LinkClicked);

        // icon::copy().size(12)
        let copy = button("copy")
            .padding(2)
            .on_press_with(|| PreviewMessage::Copy(code.to_owned()))
            .style(button::text);

        hover(
            code_block,
            right(container(copy).style(container::dark)).padding(settings.spacing / 2),
        )
    }
}

async fn download_image(url: markdown::Url) -> Result<image::Handle, Error> {
    use std::io;
    use tokio::task;

    println!("Trying to download image: {url}");

    let client = reqwest::Client::new();

    let bytes = match Url::parse(&url) {
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
        Ok::<_, Error>(
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

#[derive(Debug, Clone)]
pub enum Error {
    Request(Arc<reqwest::Error>),
    IO(Arc<io::Error>),
    Join(Arc<tokio::task::JoinError>),
    ImageDecoding(Arc<ImageError>),
}

impl From<reqwest::Error> for Error {
    fn from(error: reqwest::Error) -> Self {
        Self::Request(Arc::new(error))
    }
}

impl From<io::Error> for Error {
    fn from(error: io::Error) -> Self {
        Self::IO(Arc::new(error))
    }
}

impl From<tokio::task::JoinError> for Error {
    fn from(error: tokio::task::JoinError) -> Self {
        Self::Join(Arc::new(error))
    }
}

impl From<ImageError> for Error {
    fn from(error: ImageError) -> Self {
        Self::ImageDecoding(Arc::new(error))
    }
}
