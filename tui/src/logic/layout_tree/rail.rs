//! Where the GUI's **Activity rail** sits (ADR-0055) and the rules that
//! turn with it. The rail is not a leaf of the layout tree: the position is
//! a window-level setting, and everything here is plain data, tested
//! without a window.

use serde::{Deserialize, Serialize};

use super::Dir;

/// The window edge the rail sits on. `Left` is the default and the
/// behavior before the rail could move.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RailPosition {
    #[default]
    Left,
    Right,
    Top,
    Bottom,
}

impl RailPosition {
    /// Every position, in the order a chooser lists them.
    pub const ALL: [RailPosition; 4] = [
        RailPosition::Left,
        RailPosition::Right,
        RailPosition::Top,
        RailPosition::Bottom,
    ];

    /// Whether the rail is a row (`Top`, `Bottom`) and not a column.
    pub fn horizontal(self) -> bool {
        matches!(self, RailPosition::Top | RailPosition::Bottom)
    }

    /// The direction that points from the rail into the layout.
    pub fn toward(self) -> Dir {
        match self {
            RailPosition::Left => Dir::Right,
            RailPosition::Right => Dir::Left,
            RailPosition::Top => Dir::Down,
            RailPosition::Bottom => Dir::Up,
        }
    }

    /// The direction that points from the layout to the rail: the opposite
    /// of [`Self::toward`].
    pub fn facing(self) -> Dir {
        match self {
            RailPosition::Left => Dir::Left,
            RailPosition::Right => Dir::Right,
            RailPosition::Top => Dir::Up,
            RailPosition::Bottom => Dir::Down,
        }
    }

    /// Whether `dir` runs along the rail's axis: the two directions that
    /// move the cursor between cells, and that place a view from the
    /// rail (ADR-0055 §6).
    pub fn place_dir(self, dir: Dir) -> bool {
        match self {
            RailPosition::Left | RailPosition::Right => matches!(dir, Dir::Up | Dir::Down),
            RailPosition::Top | RailPosition::Bottom => matches!(dir, Dir::Left | Dir::Right),
        }
    }

    /// The position `word` names (trimmed, any case), as `[gui] rail` and a
    /// preset's `rail` write it. `None` for anything else.
    pub fn parse(word: &str) -> Option<RailPosition> {
        let word = word.trim();
        Self::ALL
            .into_iter()
            .find(|rail| word.eq_ignore_ascii_case(rail.name()))
    }

    /// [`Self::name`] with a capital, for a row or a control.
    pub fn label(self) -> &'static str {
        match self {
            RailPosition::Left => "Left",
            RailPosition::Right => "Right",
            RailPosition::Top => "Top",
            RailPosition::Bottom => "Bottom",
        }
    }

    /// The word the commands and the settings use.
    pub fn name(self) -> &'static str {
        match self {
            RailPosition::Left => "left",
            RailPosition::Right => "right",
            RailPosition::Top => "top",
            RailPosition::Bottom => "bottom",
        }
    }
}
