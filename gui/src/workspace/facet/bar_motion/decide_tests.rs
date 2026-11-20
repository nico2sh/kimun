//! Unit tests for the bar frame's decision ([`super::decide`]).
//!
//! Each case of ADR-0060 decision 12 is one set of facts. No window and no
//! kit types: the facts are plain values.

use super::{BarFacts, Decision, decide};

/// A bar that painted last frame and has not changed since.
fn settled() -> BarFacts {
    BarFacts {
        painted: true,
        first: false,
        width_changed: false,
        capped: false,
        reduced: false,
        changed: false,
        measure: false,
        moving: false,
        arrived: false,
        clips: false,
        past_cap: false,
    }
}

/// A bar whose rows changed, before its measuring frame.
fn changed() -> BarFacts {
    BarFacts {
        changed: true,
        measure: true,
        ..settled()
    }
}

/// A bar whose rows changed, measured, while its follow moves.
fn moving() -> BarFacts {
    BarFacts {
        changed: true,
        moving: true,
        ..settled()
    }
}

#[test]
fn a_bar_that_never_painted_snaps() {
    let facts = BarFacts {
        painted: false,
        first: true,
        ..changed()
    };
    assert_eq!(decide(&facts), Decision::Snap);
    let facts = BarFacts {
        painted: false,
        ..settled()
    };
    assert_eq!(decide(&facts), Decision::Snap);
}

#[test]
fn a_width_change_snaps() {
    for base in [changed(), moving(), settled()] {
        let facts = BarFacts {
            width_changed: true,
            ..base
        };
        assert_eq!(decide(&facts), Decision::Snap);
    }
}

#[test]
fn a_capped_bar_snaps() {
    for base in [changed(), moving(), settled()] {
        let facts = BarFacts {
            capped: true,
            ..base
        };
        assert_eq!(decide(&facts), Decision::Snap);
    }
}

#[test]
fn reduced_motion_snaps() {
    for base in [changed(), moving(), settled()] {
        let facts = BarFacts {
            reduced: true,
            ..base
        };
        assert_eq!(decide(&facts), Decision::Snap);
    }
}

#[test]
fn an_unchanged_bar_rests() {
    assert_eq!(decide(&settled()), Decision::Rest);
    // With no change, nothing else matters.
    let facts = BarFacts {
        arrived: true,
        clips: true,
        past_cap: true,
        moving: true,
        ..settled()
    };
    assert_eq!(decide(&facts), Decision::Rest);
}

#[test]
fn a_changed_bar_measures_first() {
    assert_eq!(decide(&changed()), Decision::Measure);
    // A change during a motion measures again before it moves on.
    let facts = BarFacts {
        moving: true,
        clips: true,
        ..changed()
    };
    assert_eq!(decide(&facts), Decision::Measure);
}

#[test]
fn a_measured_bar_moves_until_the_follow_ends() {
    assert_eq!(decide(&moving()), Decision::Move);
    let ended = BarFacts {
        moving: false,
        ..moving()
    };
    assert_eq!(decide(&ended), Decision::Rest);
}

#[test]
fn an_arrived_stop_snaps_only_when_the_frame_clips() {
    let clipped = BarFacts {
        arrived: true,
        clips: true,
        ..moving()
    };
    assert_eq!(decide(&clipped), Decision::Snap);
    let whole = BarFacts {
        arrived: true,
        ..moving()
    };
    assert_eq!(decide(&whole), Decision::Move);
    // The measuring frame holds the old height: it clips.
    let measuring = BarFacts {
        arrived: true,
        ..changed()
    };
    assert_eq!(decide(&measuring), Decision::Snap);
    // A frame that clips with no stop arrived moves on.
    let no_stop = BarFacts {
        clips: true,
        ..moving()
    };
    assert_eq!(decide(&no_stop), Decision::Move);
}

#[test]
fn a_height_past_the_cap_snaps_after_the_measuring_frame() {
    let measuring = BarFacts {
        past_cap: true,
        ..changed()
    };
    assert_eq!(decide(&measuring), Decision::Measure);
    let after = BarFacts {
        past_cap: true,
        ..moving()
    };
    assert_eq!(decide(&after), Decision::Snap);
    let ended = BarFacts {
        past_cap: true,
        moving: false,
        ..moving()
    };
    assert_eq!(decide(&ended), Decision::Snap);
}

#[test]
fn the_first_painted_frame_seeds_and_does_not_collapse_to_zero() {
    // The bar's first painted frame, with a change in it: the follow is
    // seeded with the painted height, and the frame measures from there.
    let facts = BarFacts {
        first: true,
        ..changed()
    };
    assert!(facts.seeds());
    assert_eq!(decide(&facts), Decision::Measure);
    // A frame that did not paint has no height to seed with.
    let unpainted = BarFacts {
        painted: false,
        ..facts
    };
    assert!(!unpainted.seeds());
    // A later frame does not seed again.
    assert!(!changed().seeds());
}
