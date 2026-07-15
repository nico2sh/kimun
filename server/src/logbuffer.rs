//! In-memory ring buffer of recent WARN/ERROR log events, exposed on the web
//! UI's Logs page — so "why is my server degraded?" is answerable from the
//! browser without shell access to the process stdout. Full logs still go to
//! stdout via the fmt layer; this buffer only keeps the last [`CAPACITY`]
//! warnings and errors.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use tracing::field::{Field, Visit};
use tracing_subscriber::Layer;

/// Entries kept before the oldest is dropped. Small on purpose: this is a
/// triage window, not log storage.
pub const CAPACITY: usize = 200;

/// One captured log event.
#[derive(Debug, Clone)]
pub struct LogEntry {
    pub time: SystemTime,
    pub level: tracing::Level,
    pub target: String,
    pub message: String,
}

/// Shared ring buffer; cheap to clone (all clones see the same entries).
#[derive(Clone, Default)]
pub struct LogBuffer {
    entries: Arc<Mutex<VecDeque<LogEntry>>>,
}

impl LogBuffer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&self, entry: LogEntry) {
        let mut entries = self.entries.lock().expect("log buffer lock poisoned");
        if entries.len() == CAPACITY {
            entries.pop_front();
        }
        entries.push_back(entry);
    }

    /// Captured entries, newest first — for the web UI.
    pub fn list(&self) -> Vec<LogEntry> {
        let entries = self.entries.lock().expect("log buffer lock poisoned");
        entries.iter().rev().cloned().collect()
    }

    /// A tracing layer that feeds this buffer. Register it alongside the fmt
    /// layer at startup.
    pub fn layer(&self) -> BufferLayer {
        BufferLayer {
            buffer: self.clone(),
        }
    }
}

/// Tracing layer that copies WARN and ERROR events into a [`LogBuffer`].
pub struct BufferLayer {
    buffer: LogBuffer,
}

impl<S: tracing::Subscriber> Layer<S> for BufferLayer {
    fn on_event(
        &self,
        event: &tracing::Event<'_>,
        _ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        let level = *event.metadata().level();
        // tracing orders levels by verbosity: ERROR < WARN < INFO < …
        if level > tracing::Level::WARN {
            return;
        }
        let mut visitor = MessageVisitor(String::new());
        event.record(&mut visitor);
        self.buffer.push(LogEntry {
            time: SystemTime::now(),
            level,
            target: event.metadata().target().to_string(),
            message: visitor.0,
        });
    }
}

/// Flattens an event's fields into one line: the `message` field verbatim,
/// any extra fields appended as `key=value`.
struct MessageVisitor(String);

impl Visit for MessageVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        use std::fmt::Write;
        if field.name() == "message" {
            let _ = write!(self.0, "{value:?}");
        } else {
            if !self.0.is_empty() {
                self.0.push(' ');
            }
            let _ = write!(self.0, "{}={:?}", field.name(), value);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tracing_subscriber::layer::SubscriberExt;

    #[test]
    fn captures_warn_and_error_but_not_info() {
        let buffer = LogBuffer::new();
        let subscriber = tracing_subscriber::registry().with(buffer.layer());
        tracing::subscriber::with_default(subscriber, || {
            tracing::info!("routine");
            tracing::warn!("watch out");
            tracing::error!(code = 7, "it broke");
        });

        let entries = buffer.list();
        assert_eq!(entries.len(), 2);
        // Newest first.
        assert_eq!(entries[0].level, tracing::Level::ERROR);
        assert!(entries[0].message.contains("it broke"));
        assert!(entries[0].message.contains("code=7"));
        assert_eq!(entries[1].level, tracing::Level::WARN);
        assert_eq!(entries[1].message, "watch out");
    }

    #[test]
    fn ring_buffer_drops_oldest() {
        let buffer = LogBuffer::new();
        for i in 0..(CAPACITY + 5) {
            buffer.push(LogEntry {
                time: SystemTime::now(),
                level: tracing::Level::WARN,
                target: "test".into(),
                message: format!("msg {i}"),
            });
        }
        let entries = buffer.list();
        assert_eq!(entries.len(), CAPACITY);
        assert_eq!(entries[0].message, format!("msg {}", CAPACITY + 4));
        assert_eq!(entries.last().unwrap().message, "msg 5");
    }
}
