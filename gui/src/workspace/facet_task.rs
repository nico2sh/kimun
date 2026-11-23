//! Ticking a task in a facet view's piece (ADR-0062). The piece never
//! writes: it asks, and the workspace refuses or hands the toggle to the
//! vault, which checks the note before it writes.

use gpui_kit::Context;
use kimun_core::error::VaultError;
use kimun_core::nfs::VaultPath;

use super::Workspace;
use crate::tasks;

/// How a toggle ended, short of a failure.
enum Toggled {
    Done,
    /// The line was no longer that task: the note changed.
    Stale,
}

/// The flash for a note that changed under a piece.
pub(crate) fn changed_flash(note: &VaultPath) -> String {
    format!("{note} changed. The pieces are up to date: click the task again.")
}

/// The flash for a note that has unsaved text in an editor.
pub(crate) fn unsaved_flash(note: &VaultPath) -> String {
    format!("Save {note} before you tick its tasks here.")
}

impl Workspace {
    /// Flip the task on source line `line` of `note`, which a piece showed as
    /// `seen`. Refused while an editor holds unsaved text of `note`, or the
    /// vault is read-only. A save redraws the pieces; so does a stale answer.
    pub(super) fn toggle_facet_task(
        &mut self,
        note: VaultPath,
        line: usize,
        seen: String,
        cx: &mut Context<Self>,
    ) {
        let unsaved = self.group_ids().into_iter().any(|group| {
            self.host_in(group)
                .and_then(|host| host.note.as_ref())
                .is_some_and(|open| open.is_like(&note))
                && self.editor_dirty_in(group, cx)
        });
        if unsaved {
            self.flash_error(unsaved_flash(&note), cx);
            return;
        }
        if self.status.read_only() {
            let text = format!(
                "Read-only: {note} can't be saved until the names that differ only by case are fixed."
            );
            self.flash_error(text, cx);
            return;
        }
        let Some(vault) = self.vault_or_flash(cx) else {
            return;
        };
        let write = {
            let note = note.clone();
            tokio::spawn(async move {
                match vault.toggle_task(&note, line, &seen).await {
                    Ok(()) => Ok(Toggled::Done),
                    Err(VaultError::TaskChanged { .. }) => Ok(Toggled::Stale),
                    Err(e) => Err(e.user_message().unwrap_or_else(|| e.to_string())),
                }
            })
        };
        cx.spawn(async move |this, cx| {
            let result = tasks::joined(write).await;
            this.update(cx, |workspace, cx| match result {
                Ok(Toggled::Done) => {
                    workspace.refresh_facets(&note, cx);
                    // A clean editor on the note takes the new text, or its
                    // next save would write the old box back.
                    workspace.reload_from_disk(note.clone(), cx);
                }
                Ok(Toggled::Stale) => {
                    workspace.refresh_facets(&note, cx);
                    workspace.flash_error(changed_flash(&note), cx);
                }
                Err(e) => workspace.flash_error(format!("Couldn't tick the task: {e}"), cx),
            })
            .ok();
        })
        .detach();
    }
}
