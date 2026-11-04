//! Following links (M5b-2 G4): the `FollowLink` chord and the editor's
//! double-click (ADR-0043) follow the **follow target** under the caret.
//!
//! The rules are the logic's ([`resolve`], shared with the TUI); this file
//! runs the lookup on tokio, applies the outcome on the main thread (the
//! note session's pattern, see [`super::note`]), and acts:
//!
//! - a note opens through the session (save before the switch; the FILES
//!   marker follows), as any other open;
//! - several notes of that name: the link chooser lists every one of them,
//!   and the one chosen opens ([`super::picker`], M6 G4);
//! - no note of that name: the create dialog offers to make it, and the
//!   note made opens ([`super::create`], M6 G4);
//! - an image or a markdown link to an attachment (`[x](doc.pdf)`) inside
//!   the vault shows it in the editor Panel ([`super::attachment`], M7a
//!   G3); a URL (or a file outside the vault) goes to the system's default
//!   program;
//! - a label (`#tag`) says it arrives later.
//!
//! A newer follow replaces one still looking up.

use std::ffi::OsStr;
use std::path::Path;
use std::rc::Rc;

use gpui_kit::Context;
use kimun_core::nfs::VaultPath;
use kimun_core::note::scan::LinkSpanKind;
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
    /// nothing there is followable. On the attachment view, open the
    /// attachment externally (there is no caret).
    pub(crate) fn follow_at_cursor(&mut self, cx: &mut Context<Self>) -> bool {
        if self.attachment.is_some() {
            self.open_attachment_externally(cx);
            return true;
        }
        let target = self.editor.read(cx).editor.follow_target_at_cursor();
        match target {
            Some(FollowTarget::Link { target, kind }) => self.follow_link(target, kind, cx),
            Some(FollowTarget::Label(name)) => self.follow_label(&name, cx),
            None => return false,
        }
        true
    }

    /// A label (`#tag`): its query view is M7.
    pub(crate) fn follow_label(&mut self, name: &str, cx: &mut Context<Self>) {
        self.flash(format!("#{name}: tag search arrives later"), cx);
    }

    /// Follow the link `target` of `kind`, from the open note: a wikilink
    /// names a note, a markdown link is a path (see [`resolve`]).
    pub(crate) fn follow_link(
        &mut self,
        target: String,
        kind: LinkSpanKind,
        cx: &mut Context<Self>,
    ) {
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
            tokio::spawn(async move { resolve(&vault, &from, &target, kind).await })
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
            FollowOutcome::OpenFile(file) => match self.vault_path_of(&file) {
                Some(path) => self.open_attachment(&path, cx),
                None => self.open_externally(target, file.as_os_str(), cx),
            },
            FollowOutcome::Note(path) => self.follow_to(path, cx),
            FollowOutcome::Missing(path) => self.offer_create(path, cx),
            FollowOutcome::Ambiguous(hits) => {
                let target = target.to_string();
                self.with_window(cx, move |workspace, window, cx| {
                    if super::modal::dialog_open(window, cx) {
                        workspace.flash_error(format!("{} notes match {target}", hits.len()), cx);
                    } else {
                        workspace.open_link_chooser(&target, hits, window, cx);
                    }
                });
            }
            FollowOutcome::Error(e) => {
                self.flash_error(format!("Couldn't follow {target}: {e}"), cx)
            }
        }
    }

    /// Open the note a link led to; a file that is no note shows in the
    /// attachment view.
    fn follow_to(&mut self, path: VaultPath, cx: &mut Context<Self>) {
        if path.is_note() {
            self.open_note(path, cx);
        } else {
            self.open_attachment(&path, cx);
        }
    }

    /// The vault path of `file`, an OS path a link resolved to; `None` when
    /// it lies outside the vault.
    fn vault_path_of(&self, file: &Path) -> Option<VaultPath> {
        let host = self.notes.as_ref()?;
        VaultPath::from_path(host.session.vault().workspace_path(), file).ok()
    }

    /// Hand `what` (a URL, a file) to the system's default program.
    fn open_externally(&mut self, target: &str, what: &OsStr, cx: &mut Context<Self>) {
        match (self.opener)(what) {
            Ok(()) => self.flash(format!("Opening {target}…"), cx),
            Err(e) => self.flash_error(format!("Couldn't open {target}: {e}"), cx),
        }
    }
}
