//! Following links (M5b-2 G4): the `FollowLink` chord and the editor's
//! double-click (ADR-0043) follow the **follow target** under the caret.
//!
//! The rules are the logic's ([`resolve`], shared with the TUI); this file
//! runs the lookup on tokio, applies the outcome on the main thread (the
//! note session's pattern, see [`super::note`]), and acts:
//!
//! - a note opens through the session (save before the switch; the FILES
//!   marker follows), as any other open;
//! - several notes of that name: the first opens, and the Footer says there
//!   were more (the chooser is M6);
//! - no note of that name: the Footer says so; nothing is created (the
//!   create dialog is M6);
//! - a URL or an image goes to the system's default program;
//! - an attachment link and a label (`#tag`) say they arrive later.
//!
//! A newer follow replaces one still looking up.

use std::ffi::OsStr;
use std::rc::Rc;

use gpui_kit::Context;
use kimun_core::nfs::VaultPath;
use kimun_notes::logic::editor::claim::FollowTarget;
use kimun_notes::logic::follow::{FollowOutcome, resolve};
use tokio::task::JoinError;

use super::Workspace;

/// Hands a URL or a file to the system's default program. The app's is
/// [`system_opener`]; tests record instead of opening.
pub(crate) type Opener = Rc<dyn Fn(&OsStr) -> std::io::Result<()>>;

/// The system's default program (`open::that_detached`): the OS handoff,
/// at the edge.
pub(crate) fn system_opener() -> Opener {
    Rc::new(|target| open::that_detached(target))
}

impl Workspace {
    /// The `FollowLink` chord: follow what is under the caret. False when
    /// nothing there is followable.
    pub(crate) fn follow_at_cursor(&mut self, cx: &mut Context<Self>) -> bool {
        let target = self.editor.read(cx).editor.follow_target_at_cursor();
        match target {
            Some(FollowTarget::Link(target)) => self.follow_link(target, cx),
            Some(FollowTarget::Label(name)) => self.follow_label(&name, cx),
            None => return false,
        }
        true
    }

    /// A label (`#tag`): its query view is M7.
    pub(crate) fn follow_label(&mut self, name: &str, cx: &mut Context<Self>) {
        self.flash(format!("#{name}: tag search arrives later"), cx);
    }

    /// Follow the link `target`, from the open note.
    pub(crate) fn follow_link(&mut self, target: String, cx: &mut Context<Self>) {
        let Some(host) = self.notes.as_ref() else {
            return;
        };
        // Links are followed from a note; without one there is no text.
        let Some(from) = host.note.clone() else {
            return;
        };
        let vault = host.session.vault().clone();
        let lookup = {
            let target = target.clone();
            tokio::spawn(async move { resolve(&vault, &from, &target).await })
        };
        self.following = Some(cx.spawn(async move |this, cx| {
            let outcome = lookup.await;
            this.update(cx, |workspace, cx| workspace.followed(&target, outcome, cx))
                .ok();
        }));
    }

    /// The lookup of `target` ended: act on where it leads.
    pub(crate) fn followed(
        &mut self,
        target: &str,
        outcome: Result<FollowOutcome, JoinError>,
        cx: &mut Context<Self>,
    ) {
        self.following = None;
        let outcome = outcome.unwrap_or_else(|e| FollowOutcome::Error(e.to_string()));
        match outcome {
            FollowOutcome::External(url) => self.open_externally(target, url.as_ref(), cx),
            FollowOutcome::OpenFile(file) => self.open_externally(target, file.as_os_str(), cx),
            FollowOutcome::Note(path) => {
                self.follow_to(path, cx);
            }
            FollowOutcome::Missing(_) => self.flash_error(format!("{target} not found"), cx),
            FollowOutcome::Ambiguous(hits) => {
                let count = hits.len();
                if let Some((first, _)) = hits.into_iter().next() {
                    let path = first.path;
                    if self.follow_to(path.clone(), cx) {
                        self.flash(format!("{count} notes match {target}: opened {path}"), cx);
                    }
                }
            }
            FollowOutcome::Error(e) => {
                self.flash_error(format!("Couldn't follow {target}: {e}"), cx)
            }
        }
    }

    /// Open the note a link led to. An attachment says it arrives later.
    /// Whether a note open was asked for.
    fn follow_to(&mut self, path: VaultPath, cx: &mut Context<Self>) -> bool {
        if !path.is_note() {
            self.flash(
                format!("{path}: opening attachments from a link arrives later"),
                cx,
            );
            return false;
        }
        self.open_note(path, cx);
        true
    }

    /// Hand `what` (a URL, a file) to the system's default program.
    fn open_externally(&mut self, target: &str, what: &OsStr, cx: &mut Context<Self>) {
        match (self.opener)(what) {
            Ok(()) => self.flash(format!("Opening {target}…"), cx),
            Err(e) => self.flash_error(format!("Couldn't open {target}: {e}"), cx),
        }
    }
}
