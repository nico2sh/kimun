//! The toolkit-free half of GUI motion (ADR-0060): the two named speeds,
//! the reveal order and the snap rule. It names no GPUI
//! type, so it is tested without a window. The GPUI half, with the kit's
//! motion types, is `workspace::motion`.
//!
//! The numbers to tune by feel are [`Speed::duration`] and [`FRAME_PART`].

use std::time::Duration;

/// The kind of change a motion belongs to. A call site names a speed and
/// never passes a raw duration. Every speed shares one curve.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Speed {
    /// Small things, such as a tab appearing.
    Quick,
    /// A pane opening or closing.
    #[default]
    Standard,
}

impl Speed {
    pub const fn duration(self) -> Duration {
        match self {
            Speed::Quick => Duration::from_millis(120),
            Speed::Standard => Duration::from_millis(200),
        }
    }
}

/// The part of a motion's time that the frame owns. The content owns the
/// rest. Opening, the frame moves in the first part and the content appears
/// in the rest. Closing, the content fades in the first part of the time
/// the content owns, and the frame closes in the rest, so a closing pane
/// never shows squeezed content.
pub const FRAME_PART: f32 = 0.6;

/// The two things that move in a pane's motion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    /// The pane's size.
    Frame,
    /// What the pane holds.
    Content,
}

/// When one stage runs, from the start of the motion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Timing {
    pub delay: Duration,
    pub duration: Duration,
}

/// The timing of `stage` in a motion of `speed`, `opening` or closing. The
/// two stages of one motion never overlap, and together they take the
/// speed's duration. Each stage eases out on its own, in either direction,
/// because each is a value of its own (the one curve of ADR-0060).
pub fn timing(speed: Speed, stage: Stage, opening: bool) -> Timing {
    let total = speed.duration();
    let frame = total.mul_f32(FRAME_PART);
    let content = total - frame;
    match (stage, opening) {
        (Stage::Frame, true) => Timing {
            delay: Duration::ZERO,
            duration: frame,
        },
        (Stage::Content, true) => Timing {
            delay: frame,
            duration: content,
        },
        (Stage::Content, false) => Timing {
            delay: Duration::ZERO,
            duration: content,
        },
        (Stage::Frame, false) => Timing {
            delay: content,
            duration: frame,
        },
    }
}

/// What the renderer knows about this frame that decides a snap.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SnapFacts {
    /// No frame was drawn before this one.
    pub first_frame: bool,
    /// The window size differs from the last frame's.
    pub viewport_changed: bool,
    /// A divider is being dragged.
    pub divider_drag: bool,
    /// Zoom started or ended since the last frame.
    pub zoom_changed: bool,
    /// The rail moved to another edge since the last frame.
    pub rail_moved: bool,
}

/// Whether this frame draws the new state at once. Reduced motion is not
/// here: the kit adopts the end state by itself.
pub fn must_snap(facts: SnapFacts) -> bool {
    facts.first_frame
        || facts.viewport_changed
        || facts.divider_drag
        || facts.zoom_changed
        || facts.rail_moved
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn close(a: Duration, b: Duration) -> bool {
        a.abs_diff(b) < Duration::from_micros(10)
    }

    #[test]
    fn speeds_are_120_and_200_ms() {
        assert_eq!(Speed::Quick.duration(), Duration::from_millis(120));
        assert_eq!(Speed::Standard.duration(), Duration::from_millis(200));
    }

    #[test]
    fn opening_moves_the_frame_then_the_content() {
        let total = Speed::Standard.duration();
        let frame = timing(Speed::Standard, Stage::Frame, true);
        let content = timing(Speed::Standard, Stage::Content, true);
        assert_eq!(frame.delay, Duration::ZERO);
        assert!(close(frame.duration, total.mul_f32(FRAME_PART)));
        assert!(close(content.delay, frame.delay + frame.duration));
        assert!(close(content.delay + content.duration, total));
    }

    #[test]
    fn closing_fades_the_content_then_closes_the_frame() {
        let total = Speed::Standard.duration();
        let content = timing(Speed::Standard, Stage::Content, false);
        let frame = timing(Speed::Standard, Stage::Frame, false);
        assert_eq!(content.delay, Duration::ZERO);
        assert!(close(content.duration, total.mul_f32(1.0 - FRAME_PART)));
        assert!(close(frame.delay, content.delay + content.duration));
        assert!(close(frame.delay + frame.duration, total));
    }

    #[test]
    fn the_frame_always_has_the_larger_part() {
        for speed in [Speed::Quick, Speed::Standard] {
            for opening in [true, false] {
                let frame = timing(speed, Stage::Frame, opening).duration;
                let content = timing(speed, Stage::Content, opening).duration;
                assert!(frame > content, "{speed:?}, opening: {opening}");
            }
        }
    }

    #[test]
    fn a_rail_move_snaps() {
        assert!(must_snap(SnapFacts {
            rail_moved: true,
            ..SnapFacts::default()
        }));
    }

    #[test]
    fn any_snap_fact_snaps_and_none_does_not() {
        assert!(!must_snap(SnapFacts::default()));
        let none = SnapFacts::default();
        assert!(must_snap(SnapFacts {
            first_frame: true,
            ..none
        }));
        assert!(must_snap(SnapFacts {
            viewport_changed: true,
            ..none
        }));
        assert!(must_snap(SnapFacts {
            divider_drag: true,
            ..none
        }));
        assert!(must_snap(SnapFacts {
            zoom_changed: true,
            ..none
        }));
    }
}
