//! The in-process textarea backend: the rope buffer plus its input
//! interpreter (direct typing or the built-in vim emulation). Terminal-free;
//! the host's `BackendState` delegates here for its textarea arm.

use crate::logic::editor::rope_buffer::RopeBuffer;
use crate::logic::editor::snapshot::EditorMode;
use crate::logic::editor::vim::VimEngine;

/// How key events are translated into edits on the **rope buffer**.
/// The engine is boxed so the `Direct` arm doesn't pay the engine's size
/// (registers, dot-repeat state, replace stack — ~230 bytes).
#[derive(Debug, Default)]
pub enum InputInterpreter {
    /// Today's behavior: keys go straight to the textarea.
    #[default]
    Direct,
    /// Built-in vim emulation.
    Vim(Box<VimEngine>),
}

/// The in-process textarea storage plus its input interpreter.
#[derive(Debug)]
pub struct TextareaBackend {
    /// Which keystrokes are sharing an undo group. The **plain** backend's
    /// policy; the **vim** engine has its own and leaves this alone.
    pub typing: crate::logic::editor::typing_run::TypingRun,
    /// The open note's text and its edit history. Mutations go
    /// through `RopeBuffer::edit`.
    pub ta: RopeBuffer,
    pub input: InputInterpreter,
}

impl TextareaBackend {
    pub fn direct(text: crate::ropetext::Text) -> Self {
        Self {
            ta: RopeBuffer::new(text),
            typing: crate::logic::editor::typing_run::TypingRun::default(),
            input: InputInterpreter::Direct,
        }
    }
    pub fn vim(text: crate::ropetext::Text) -> Self {
        Self {
            ta: RopeBuffer::new(text),
            typing: crate::logic::editor::typing_run::TypingRun::default(),
            input: InputInterpreter::Vim(Box::default()),
        }
    }

    /// True when the input interpreter is the built-in vim emulation (any mode).
    pub fn is_vim(&self) -> bool {
        matches!(self.input, InputInterpreter::Vim(_))
    }

    /// Reconcile the input interpreter with a host-driven mouse
    /// selection change. The vim interpreter tracks it modally (a new
    /// selection enters Visual, a cleared one returns to Normal); the direct
    /// interpreter has nothing to reconcile.
    pub fn sync_mouse_selection(&mut self, has_selection: bool) {
        if let TextareaBackend {
            input: InputInterpreter::Vim(e),
            ta,
            ..
        } = self
        {
            e.sync_mouse_selection(has_selection, ta);
        }
    }

    /// The range the vim interpreter's Visual selection covers
    /// (`VimEngine::visual_range`: whole characters, whole rows for `V`).
    /// `None` outside Visual and under the direct interpreter.
    pub fn visual_range(&self) -> Option<((usize, usize), (usize, usize))> {
        match self {
            TextareaBackend {
                input: InputInterpreter::Vim(e),
                ta,
                ..
            } => e.visual_range(ta),
            _ => None,
        }
    }

    /// The selection as the user sees it: the Visual range under vim,
    /// otherwise the buffer's own selection. `None` without one.
    pub fn selection_as_shown(&self) -> Option<((usize, usize), (usize, usize))> {
        self.visual_range().or_else(|| self.ta.selection_range())
    }

    /// Whether the vim interpreter is in Visual over nothing — an empty row
    /// (see `VimEngine::visual_is_empty`). `false` elsewhere.
    pub fn visual_is_empty(&self) -> bool {
        match self {
            TextareaBackend {
                input: InputInterpreter::Vim(e),
                ta,
                ..
            } => e.visual_is_empty(ta),
            _ => false,
        }
    }

    /// Copy the vim interpreter's Visual selection and leave Visual (see
    /// `VimEngine::copy_visual`). `None` outside Visual and under the direct
    /// interpreter, where the buffer's own selection is what gets copied.
    pub fn copy_visual(&mut self) -> Option<String> {
        match self {
            TextareaBackend {
                input: InputInterpreter::Vim(e),
                ta,
                ..
            } => e.copy_visual(ta),
            _ => None,
        }
    }

    /// A mouse drag from `origin` to `pos` under the vim interpreter (see
    /// `VimEngine::select_dragged`). `false` where the plain drag selection
    /// stands: the direct interpreter, and vim's Insert and Replace.
    pub fn select_dragged(&mut self, origin: (usize, usize), pos: (usize, usize)) -> bool {
        match self {
            TextareaBackend {
                input: InputInterpreter::Vim(e),
                ta,
                ..
            } => e.select_dragged(ta, origin, pos),
            _ => false,
        }
    }

    /// Hand the vim interpreter the selection the host just left (see
    /// `VimEngine::adopt_host_selection`). A no-op for the direct interpreter.
    pub fn adopt_host_selection(&mut self) {
        if let TextareaBackend {
            input: InputInterpreter::Vim(e),
            ta,
            ..
        } = self
        {
            e.adopt_host_selection(ta);
        }
    }

    /// Let the vim interpreter record the live Visual selection for `gv`
    /// before the host edits it away. A no-op for the direct interpreter.
    pub fn conclude_visual(&mut self) {
        if let TextareaBackend {
            input: InputInterpreter::Vim(e),
            ta,
            ..
        } = self
        {
            e.conclude_visual(ta);
        }
    }

    /// True when a bare Space should start the leader sequence. Only the vim
    /// interpreter ever says yes (Normal mode, empty pending state); for the
    /// direct interpreter Space is just typing.
    pub fn space_leads(&self) -> bool {
        matches!(self,
            TextareaBackend { input: InputInterpreter::Vim(e), .. }
            if e.space_leads())
    }

    /// True when the vim interpreter is in linewise Visual (`V`). The
    /// highlight there must always run the full width of every selected
    /// row, regardless of the cursor's column.
    pub fn is_visual_line(&self) -> bool {
        matches!(self,
            TextareaBackend { input: InputInterpreter::Vim(e), .. }
            if *e.mode() == EditorMode::VisualLine)
    }

    /// Reset any transient input-interpreter state for a freshly loaded note
    /// (the vim interpreter returns to Normal; the direct interpreter carries
    /// no such state).
    pub fn reset_input_state(&mut self) {
        if let TextareaBackend {
            input: InputInterpreter::Vim(engine),
            ..
        } = self
        {
            // The buffer's marks went with its text (`RopeBuffer::replace`).
            engine.reset_to_normal();
        }
    }

    /// If the interpreter is vim, run it for this key and return the
    /// outcome. Returns `None` for the direct interpreter.
    pub fn vim_handle_key(
        &mut self,
        key: &crate::logic::input::KeyInput,
    ) -> Option<crate::logic::editor::vim::VimKeyOutcome> {
        match self {
            TextareaBackend {
                ta,
                input: InputInterpreter::Vim(engine),
                ..
            } => Some(engine.handle_key(key, ta)),
            _ => None,
        }
    }

    /// The in-progress input-command hint for the footer (the vim
    /// interpreter's pending count/operator/find/g sequence). `None` when
    /// there is no pending sequence.
    pub fn pending_input_hint(&self) -> Option<String> {
        match self {
            TextareaBackend {
                input: InputInterpreter::Vim(e),
                ..
            } => e.pending_hint(),
            _ => None,
        }
    }

    /// The footer modal-mode label, when the interpreter has one (vim).
    /// `None` for the plain Direct textarea.
    pub fn mode_label(&self) -> Option<String> {
        match self {
            TextareaBackend {
                input: InputInterpreter::Vim(engine),
                ..
            } => Some(engine.mode_label()),
            TextareaBackend { .. } => None,
        }
    }

    /// Alloc-free cursor-shape classifier for the render path.
    /// `None` = non-modal interpreter (Direct textarea — leave terminal cursor as-is).
    /// `Some(true)` = Insert mode (bar cursor).
    /// `Some(false)` = other modal mode (block cursor).
    pub fn modal_is_insert(&self) -> Option<bool> {
        match self {
            TextareaBackend {
                input: InputInterpreter::Vim(e),
                ..
            } => Some(*e.mode() == EditorMode::Insert),
            TextareaBackend { .. } => None,
        }
    }
}
