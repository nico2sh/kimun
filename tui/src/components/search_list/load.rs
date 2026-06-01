//! Generation-stamped async-load lifecycle shared by one-shot and streamed
//! delivery. A new load bumps the generation and aborts the prior task;
//! `drain` discards any results stamped with a stale generation.

use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};

use super::seams::{Emit, Loaded, RowSource, SearchRow};

pub(super) struct LoadEngine<R: SearchRow> {
    generation: u64,
    rx: Receiver<(u64, Loaded<R>)>,
    tx: Sender<(u64, Loaded<R>)>,
    task: Option<tokio::task::JoinHandle<()>>,
    redraw: Arc<dyn Fn() + Send + Sync>,
    pub(super) loading: bool,
}

impl<R: SearchRow> LoadEngine<R> {
    pub(super) fn new(redraw: Arc<dyn Fn() + Send + Sync>) -> Self {
        let (tx, rx) = channel();
        Self { generation: 0, rx, tx, task: None, redraw, loading: false }
    }

    pub(super) fn start(&mut self, source: Arc<dyn RowSource<R>>, query: String) {
        if let Some(t) = self.task.take() {
            t.abort();
        }
        self.generation += 1;
        self.loading = true;
        let emit = Emit::new(self.tx.clone(), self.generation, self.redraw.clone());
        self.task = Some(tokio::spawn(async move {
            source.load(&query, emit).await;
        }));
    }

    /// The generation of the most recently started load. `poll` compares this
    /// against the generation it last applied so it can clear stale rows from a
    /// superseded load before applying a streamed (`Push`) source's results.
    pub(super) fn generation(&self) -> u64 {
        self.generation
    }

    pub(super) fn drain(&mut self) -> Vec<Loaded<R>> {
        let mut out = Vec::new();
        while let Ok((stamp, ev)) = self.rx.try_recv() {
            if stamp != self.generation {
                continue;
            }
            match &ev {
                Loaded::Replace(_) | Loaded::Done => self.loading = false,
                Loaded::Push(_) => {}
            }
            out.push(ev);
        }
        out
    }
}
