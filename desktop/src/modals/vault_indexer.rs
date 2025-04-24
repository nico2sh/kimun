use std::time::{Duration, SystemTime};

use iced::Task;
use kimun_core::{NoteVault, NotesValidation};

use crate::{
    KimunMessage,
    components::{
        easing::EMPHASIZED_DECELERATE,
        linear_progress::{CYCLE_DURATION, LinearProgress},
    },
    style_units::{LARGE_SPACING, SMALL_PADDING, SMALL_SPACING},
};

use super::KimunModal;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IndexType {
    Validate,
    Fast,
    Full,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IndexStatus {
    Error(String),
    Indexing,
    Done(Duration),
}

#[derive(Debug, Clone)]
pub enum IndexStatusUpdateMsg {
    Finished(IndexStatus),
}

impl From<IndexStatusUpdateMsg> for KimunMessage {
    fn from(value: IndexStatusUpdateMsg) -> Self {
        KimunMessage::IndexStatus(value)
    }
}

pub struct VaultIndexer {
    start_time: SystemTime,
    index_type: IndexType,
    status: IndexStatus,
}

impl VaultIndexer {
    pub fn new(vault: NoteVault, index_type: IndexType) -> (Self, Task<KimunMessage>) {
        let start_time = SystemTime::now();
        let task = match index_type {
            IndexType::Validate => {
                // We just validate the data
                let future = async move { vault.init_and_validate() };
                Task::perform(future, |res| match res {
                    Ok(report) => {
                        IndexStatusUpdateMsg::Finished(IndexStatus::Done(report.duration)).into()
                    }
                    Err(e) => {
                        IndexStatusUpdateMsg::Finished(IndexStatus::Error(e.to_string())).into()
                    }
                })
            }
            IndexType::Fast => {
                // We do a quick validation
                let future = async move { vault.index_notes(NotesValidation::Fast) };
                Task::perform(future, |res| match res {
                    Ok(report) => {
                        IndexStatusUpdateMsg::Finished(IndexStatus::Done(report.duration)).into()
                    }
                    Err(e) => {
                        IndexStatusUpdateMsg::Finished(IndexStatus::Error(e.to_string())).into()
                    }
                })
            }
            IndexType::Full => {
                // Full validation by rebuilding the database
                let future = async move { vault.force_rebuild() };
                Task::perform(future, |res| match res {
                    Ok(report) => {
                        IndexStatusUpdateMsg::Finished(IndexStatus::Done(report.duration)).into()
                    }
                    Err(e) => {
                        IndexStatusUpdateMsg::Finished(IndexStatus::Error(e.to_string())).into()
                    }
                })
            }
        };
        (
            Self {
                start_time,
                index_type,
                status: IndexStatus::Indexing,
            },
            task,
        )
    }

    fn view_indexing(&self) -> iced::Element<KimunMessage> {
        let text = match self.index_type {
            IndexType::Validate => "Validating Vault, please wait",
            IndexType::Fast => "Fast checking, please wait",
            IndexType::Full => "Fully reindexing vault, this may take a bit on large vaults",
        };

        iced::widget::container(
            iced::widget::column![
                iced::widget::text(text).width(iced::Length::Fill),
                LinearProgress::new()
                    .easing(&EMPHASIZED_DECELERATE)
                    .cycle_duration(Duration::from_secs_f32(CYCLE_DURATION))
                    .width(iced::Length::Fill)
            ]
            .width(iced::Length::Fill)
            .spacing(LARGE_SPACING),
        )
        .padding(SMALL_PADDING)
        .width(iced::Length::Fill)
        .into()
    }

    fn view_error(&self, error: String) -> iced::Element<KimunMessage> {
        iced::widget::container(iced::widget::column![
            iced::widget::text("There has been an error while indexing the Vault:"),
            iced::widget::text(error),
            iced::widget::vertical_space().height(SMALL_SPACING),
            iced::widget::button("Close").on_press(KimunMessage::CloseModal)
        ])
        .padding(SMALL_PADDING)
        .into()
    }

    fn view_done(&self, duration: &Duration) -> iced::Element<KimunMessage> {
        iced::widget::container(iced::widget::column![
            iced::widget::text(format!(
                "Finished indexing in {} seconds",
                duration.as_secs()
            )),
            iced::widget::vertical_space().height(SMALL_SPACING),
            iced::widget::button("Close").on_press(KimunMessage::CloseModal)
        ])
        .padding(SMALL_PADDING)
        .into()
    }
}

impl KimunModal for VaultIndexer {
    fn view(&self) -> iced::Element<crate::KimunMessage> {
        match &self.status {
            IndexStatus::Indexing => self.view_indexing(),
            IndexStatus::Error(e) => self.view_error(e.to_owned()),
            IndexStatus::Done(duration) => self.view_done(duration),
        }
    }

    fn get_width(&self) -> iced::Length {
        400.into()
    }

    fn get_height(&self) -> iced::Length {
        iced::Length::Shrink
    }

    fn update(&mut self, message: crate::KimunMessage) -> iced::Task<crate::KimunMessage> {
        if let KimunMessage::IndexStatus(status_update) = message {
            match status_update {
                IndexStatusUpdateMsg::Finished(status) => {
                    self.status = status;
                    Task::none()
                }
            }
        } else {
            Task::none()
        }
    }

    fn key_press(
        &self,
        _key: &iced::keyboard::Key,
        _modifiers: &iced::keyboard::Modifiers,
    ) -> iced::Task<crate::KimunMessage> {
        Task::none()
    }

    fn should_close_on_click(&self) -> bool {
        !matches!(self.status, IndexStatus::Indexing)
    }
}
