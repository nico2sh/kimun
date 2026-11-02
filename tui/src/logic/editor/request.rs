//! What the editor asks of its host.

/// Something the editor wants its host to do, because the editor does not own the
/// surface that does it (a menu, the find bar). Drained with `take_requests`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditorRequest {
    /// Open the note's context menu (file & note ops); the host owns the path.
    ContextMenu,
}
