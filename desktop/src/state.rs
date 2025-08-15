use dioxus_radio::hooks::RadioChannel;

#[derive(PartialEq, Eq, Clone, Debug, Copy, Hash)]
pub enum KimunChannel {
    Global,
    Header,
}

#[derive(Debug)]
pub struct AppState {
    pub content_type: ContentType,
}

#[derive(Debug)]
pub enum ContentType {
    None,
    Note { dirty: bool },
    Directory,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            content_type: ContentType::None,
        }
    }
}

impl AppState {
    pub fn has_dirty_content(&self) -> bool {
        match self.content_type {
            ContentType::Note { dirty } => dirty,
            _ => false,
        }
    }
    pub fn mark_content_clean(&mut self) {
        if let ContentType::Note { ref mut dirty } = self.content_type {
            *dirty = false
        };
    }
    pub fn mark_content_dirty(&mut self) {
        if let ContentType::Note { ref mut dirty } = self.content_type {
            *dirty = true
        };
    }
    pub fn set_content_type(&mut self, content_type: ContentType) {
        self.content_type = content_type;
    }
}

impl RadioChannel<AppState> for KimunChannel {}
