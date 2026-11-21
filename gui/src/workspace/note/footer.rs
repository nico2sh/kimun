//! The counts and the git state that the Footer shows: backlinks,
//! properties and the git summary, fetched in the background.
//! It does not draw them. The Footer does that.

use std::path::Path;

use gpui_kit::Context;
use kimun_notes::logic::layout_tree::GroupId;
use kimun_notes::logic::message::AppMessage;
use kimun_notes::util::git_status::GitSummary;

use crate::workspace::Workspace;

impl Workspace {
    /// Re-read the property count of the note `group` holds (it was saved).
    pub(in crate::workspace) fn refresh_note_properties_in(&mut self, group: GroupId) {
        if let Some((host, tx)) = self.host_and_tx_in(group)
            && let Some(note) = host.note.clone()
        {
            host.meta.refresh_properties(&note, tx);
        }
    }

    /// A note, attachment or folder was renamed, moved or deleted: links to
    /// the open note may have been rewritten or gone with it. Re-read both
    /// counts, keeping the ones shown until they land.
    pub(crate) fn refresh_note_counts(&mut self) {
        if let Some((host, tx)) = self.active_host_and_tx()
            && let Some(note) = host.note.clone()
        {
            host.meta.refresh_backlinks(&note, tx);
            host.meta.refresh_properties(&note, tx);
        }
    }

    /// Re-read the workspace's git summary (throttled; a no-op with no
    /// repository to look at).
    pub(crate) fn refresh_git(&mut self) {
        if let Some((host, tx)) = self.active_host_and_tx() {
            host.meta.refresh_git(tx);
        }
    }

    /// A git summary fetch landed: the Footer shows what the note meta keeps
    /// (a failed fetch keeps the last known state).
    pub(crate) fn git_loaded(
        &mut self,
        root: &Path,
        status: Option<GitSummary>,
        cx: &mut Context<Self>,
    ) {
        let Some(host) = self.active_state_mut().notes.as_mut() else {
            return;
        };
        host.meta.git_loaded(root, status);
        let shown = host.meta.git().copied();
        if self.status.git != shown {
            self.status.git = shown;
            cx.notify();
        }
    }

    /// Leader `g s`: look again at the footer's summary, and toast the full
    /// state: the branch, how far it is from its upstream, and the changed
    /// paths by kind. Read off the UI thread; the toast arrives on the app
    /// channel.
    pub(crate) fn show_git_status(&mut self, cx: &mut Context<Self>) {
        let Some((host, tx)) = self.active_host_and_tx() else {
            return;
        };
        let tx = tx.clone();
        // The one `git status` run answers both: the toast and the footer.
        host.meta.begin_git_probe();
        let root = host
            .session
            .vault()
            .workspace_path()
            .as_path()
            .to_path_buf();
        tokio::spawn(async move {
            let detail = kimun_notes::util::git_status::detail(root.clone()).await;
            let text = match &detail {
                Some(detail) => detail.describe(),
                None => "Not a git repository".to_string(),
            };
            tx.send(AppMessage::GitStatusLoaded {
                root,
                status: detail.as_ref().map(GitSummary::from),
            })
            .ok();
            tx.send(AppMessage::FlashMessage(text)).ok();
        });
        cx.notify();
    }

    /// A count fetched in the background: kept when it is for the open note.
    /// It goes to the group holding the note it counts, active or not.
    pub(crate) fn note_count_loaded(&mut self, message: AppMessage) {
        let counted = match &message {
            AppMessage::BacklinkCountLoaded { path, .. }
            | AppMessage::PropertyCountLoaded { path, .. } => Some(path),
            _ => None,
        };
        let group = counted
            .and_then(|path| self.group_where(|h| h.note.as_ref() == Some(path)))
            .unwrap_or_else(|| self.layout.active_group());
        if let Some(host) = self.host_in_mut(group)
            && let Some(note) = host.note.as_ref()
        {
            // Only the counts are routed here: nothing comes back.
            host.meta.handle(message, note);
        }
    }
}
