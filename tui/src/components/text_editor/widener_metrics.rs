//! Counters for the hybrid widener's per-keystroke outcomes.
//!
//! Always on (atomic increments cost ~5ns). Surfaced when the env
//! var `KIMUN_DUMP_WIDENER_METRICS=1` is set: [`dump_if_enabled`]
//! prints the snapshot to stderr at app exit.
//!
//! Categories are exclusive — each call to
//! `MarkdownEditorView::try_incremental_parse` increments exactly one
//! of {`incremental_reset`, `incremental_fallback`,
//! `full_line_count_change`, `full_kind_guard`, `full_lazy_depth`,
//! `full_blank_transition`, `full_cap_trip`, `full_verify_failed`,
//! `full_no_damage`}. `attempted` is the sum.
//!
//! Derived metrics the consumer cares about:
//!
//! - successful_incremental_rate
//!   = (reset + fallback) / attempted
//! - fast_path_share
//!   = reset / (reset + fallback)
//!   — climbing toward 1 means Option A (tighten reset_boundaries)
//!   would eliminate the fallback path's overhead with low impact.
//! - guard_sprawl
//!   = (kind_guard + lazy_depth + blank_transition) / attempted
//!   — high values mean the call-site guard tower is the bottleneck
//!   and the underlying boundary model is too loose.
//! - verify_hit_rate
//!   = verify_failed / fallback
//!   — non-zero proves widen_to_safe needs the verify in release;
//!   zero across many sessions argues for demoting verify to debug.

use std::sync::atomic::{AtomicU64, Ordering};

/// Why `try_incremental_parse` did NOT take the splice path. One of
/// these is recorded for every full rebuild; on success a separate
/// `IncrementalReset` / `IncrementalFallback` is recorded.
#[derive(Debug, Clone, Copy)]
pub enum BailReason {
    /// Line-count gate at the top of `try_incremental_parse`.
    LineCountChange,
    /// `compute_damage_range` returned None — no actual text change
    /// despite the generation bump.
    NoDamage,
    /// One of the v1 `looks_like_*` flip checks or kind-was-marker
    /// guards triggered.
    KindGuard,
    /// V2 `lazy_depth[row±1] > 0` guard triggered.
    LazyDepth,
    /// V2 blank ↔ non-blank transition with non-blank neighbour
    /// triggered.
    BlankTransition,
    /// Both `expand_to_reset_boundary` AND `widen_to_safe` returned
    /// `FullRebuild` — no widening fits under the caps.
    CapTrip,
    /// Post-slice undamaged-row verify (widen_to_safe fallback path)
    /// detected a kinds/elements/content_vis divergence.
    VerifyFailed,
}

/// Which widener produced the splice that succeeded.
#[derive(Debug, Clone, Copy)]
pub enum SuccessPath {
    /// `expand_to_reset_boundary` succeeded — the boundary set is
    /// known reset, so no post-slice verify ran.
    ResetBoundary,
    /// `widen_to_safe` succeeded after `expand_to_reset_boundary`
    /// returned `FullRebuild`. The post-slice verify ran and passed.
    WidenToSafe,
}

pub struct WidenerMetrics {
    /// Raw count of `try_incremental_parse` entries — bumped before
    /// ANY early return, including the empty-buffer first-parse case
    /// that the categorised counters skip. Lets us distinguish
    /// "function never called" from "function called but always took
    /// the uncounted first-parse path".
    pub entries: AtomicU64,
    /// Raw count of `MarkdownEditorView::update` calls — bumped
    /// unconditionally on entry. Detects backends that bypass the
    /// view-update path entirely.
    pub view_updates: AtomicU64,
    pub incremental_reset: AtomicU64,
    pub incremental_fallback: AtomicU64,
    pub full_line_count_change: AtomicU64,
    pub full_no_damage: AtomicU64,
    pub full_kind_guard: AtomicU64,
    pub full_lazy_depth: AtomicU64,
    pub full_blank_transition: AtomicU64,
    pub full_cap_trip: AtomicU64,
    pub full_verify_failed: AtomicU64,
    /// First-parse / placeholder-installed: `parsed_buffer.lines`
    /// was empty when the call started.
    pub first_parse: AtomicU64,
}

impl WidenerMetrics {
    const fn new() -> Self {
        Self {
            entries: AtomicU64::new(0),
            view_updates: AtomicU64::new(0),
            incremental_reset: AtomicU64::new(0),
            incremental_fallback: AtomicU64::new(0),
            full_line_count_change: AtomicU64::new(0),
            full_no_damage: AtomicU64::new(0),
            full_kind_guard: AtomicU64::new(0),
            full_lazy_depth: AtomicU64::new(0),
            full_blank_transition: AtomicU64::new(0),
            full_cap_trip: AtomicU64::new(0),
            full_verify_failed: AtomicU64::new(0),
            first_parse: AtomicU64::new(0),
        }
    }

    /// Bump on every entry to `try_incremental_parse`, before any
    /// early return.
    pub fn entered(&self) {
        self.entries.fetch_add(1, Ordering::Relaxed);
    }

    /// Bump on every entry to `MarkdownEditorView::update`.
    pub fn view_updated(&self) {
        self.view_updates.fetch_add(1, Ordering::Relaxed);
    }

    /// Bump when `try_incremental_parse` bailed on the empty-buffer
    /// first-parse path (no parent state to splice against).
    pub fn first_parse_seen(&self) {
        self.first_parse.fetch_add(1, Ordering::Relaxed);
    }

    /// Record a full-rebuild outcome and return `None` so callers can
    /// `return METRICS.bail(...)` in one line.
    pub fn bail<T>(&self, reason: BailReason) -> Option<T> {
        let counter = match reason {
            BailReason::LineCountChange => &self.full_line_count_change,
            BailReason::NoDamage => &self.full_no_damage,
            BailReason::KindGuard => &self.full_kind_guard,
            BailReason::LazyDepth => &self.full_lazy_depth,
            BailReason::BlankTransition => &self.full_blank_transition,
            BailReason::CapTrip => &self.full_cap_trip,
            BailReason::VerifyFailed => &self.full_verify_failed,
        };
        counter.fetch_add(1, Ordering::Relaxed);
        None
    }

    /// Record a successful incremental splice.
    pub fn ok(&self, path: SuccessPath) {
        let counter = match path {
            SuccessPath::ResetBoundary => &self.incremental_reset,
            SuccessPath::WidenToSafe => &self.incremental_fallback,
        };
        counter.fetch_add(1, Ordering::Relaxed);
    }

    /// Read every counter into a snapshot for printing/derivations.
    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            entries: self.entries.load(Ordering::Relaxed),
            view_updates: self.view_updates.load(Ordering::Relaxed),
            incremental_reset: self.incremental_reset.load(Ordering::Relaxed),
            incremental_fallback: self.incremental_fallback.load(Ordering::Relaxed),
            full_line_count_change: self.full_line_count_change.load(Ordering::Relaxed),
            full_no_damage: self.full_no_damage.load(Ordering::Relaxed),
            full_kind_guard: self.full_kind_guard.load(Ordering::Relaxed),
            full_lazy_depth: self.full_lazy_depth.load(Ordering::Relaxed),
            full_blank_transition: self.full_blank_transition.load(Ordering::Relaxed),
            full_cap_trip: self.full_cap_trip.load(Ordering::Relaxed),
            full_verify_failed: self.full_verify_failed.load(Ordering::Relaxed),
            first_parse: self.first_parse.load(Ordering::Relaxed),
        }
    }
}

pub static METRICS: WidenerMetrics = WidenerMetrics::new();

#[derive(Debug, Clone, Copy)]
pub struct Snapshot {
    pub entries: u64,
    pub view_updates: u64,
    pub incremental_reset: u64,
    pub incremental_fallback: u64,
    pub full_line_count_change: u64,
    pub full_no_damage: u64,
    pub full_kind_guard: u64,
    pub full_lazy_depth: u64,
    pub full_blank_transition: u64,
    pub full_cap_trip: u64,
    pub full_verify_failed: u64,
    pub first_parse: u64,
}

impl Snapshot {
    pub fn attempted(&self) -> u64 {
        self.incremental_reset
            + self.incremental_fallback
            + self.full_line_count_change
            + self.full_no_damage
            + self.full_kind_guard
            + self.full_lazy_depth
            + self.full_blank_transition
            + self.full_cap_trip
            + self.full_verify_failed
    }

    pub fn successful_incremental(&self) -> u64 {
        self.incremental_reset + self.incremental_fallback
    }

    pub fn successful_incremental_rate(&self) -> f64 {
        let denom = self.attempted();
        if denom == 0 { 0.0 } else { self.successful_incremental() as f64 / denom as f64 }
    }

    pub fn fast_path_share(&self) -> f64 {
        let denom = self.successful_incremental();
        if denom == 0 { 0.0 } else { self.incremental_reset as f64 / denom as f64 }
    }

    pub fn guard_sprawl_rate(&self) -> f64 {
        let denom = self.attempted();
        if denom == 0 {
            0.0
        } else {
            (self.full_kind_guard + self.full_lazy_depth + self.full_blank_transition) as f64
                / denom as f64
        }
    }

    pub fn verify_hit_rate(&self) -> f64 {
        // Verify only runs on the widen_to_safe fallback path. Hit rate =
        // verify_failed / (verify_failed + fallback_success).
        let denom = self.full_verify_failed + self.incremental_fallback;
        if denom == 0 { 0.0 } else { self.full_verify_failed as f64 / denom as f64 }
    }
}

/// Print the snapshot to stderr when `KIMUN_DUMP_WIDENER_METRICS=1`.
/// No-op otherwise. Intended for the app shutdown path.
pub fn dump_if_enabled() {
    if std::env::var("KIMUN_DUMP_WIDENER_METRICS").as_deref() != Ok("1") {
        return;
    }
    let s = METRICS.snapshot();
    eprintln!(
        "[widener-metrics] session totals\n  \
         view_updates            = {:>10}\n  \
         try_incremental_entries = {:>10}\n  \
         first_parse (uncounted) = {:>10}\n  \
         ---\n  \
         incremental_reset       = {:>10}  ({:5.1}%)\n  \
         incremental_fallback    = {:>10}  ({:5.1}%)\n  \
         full_line_count_change  = {:>10}  ({:5.1}%)\n  \
         full_no_damage          = {:>10}  ({:5.1}%)\n  \
         full_kind_guard         = {:>10}  ({:5.1}%)\n  \
         full_lazy_depth         = {:>10}  ({:5.1}%)\n  \
         full_blank_transition   = {:>10}  ({:5.1}%)\n  \
         full_cap_trip           = {:>10}  ({:5.1}%)\n  \
         full_verify_failed      = {:>10}  ({:5.1}%)\n  \
         attempted (categorised) = {:>10}\n  \
         ---\n  \
         successful_incremental_rate = {:5.1}%\n  \
         fast_path_share             = {:5.1}%\n  \
         guard_sprawl_rate           = {:5.1}%\n  \
         verify_hit_rate             = {:5.1}%",
        s.view_updates,
        s.entries,
        s.first_parse,
        s.incremental_reset, pct(s.incremental_reset, s.attempted()),
        s.incremental_fallback, pct(s.incremental_fallback, s.attempted()),
        s.full_line_count_change, pct(s.full_line_count_change, s.attempted()),
        s.full_no_damage, pct(s.full_no_damage, s.attempted()),
        s.full_kind_guard, pct(s.full_kind_guard, s.attempted()),
        s.full_lazy_depth, pct(s.full_lazy_depth, s.attempted()),
        s.full_blank_transition, pct(s.full_blank_transition, s.attempted()),
        s.full_cap_trip, pct(s.full_cap_trip, s.attempted()),
        s.full_verify_failed, pct(s.full_verify_failed, s.attempted()),
        s.attempted(),
        s.successful_incremental_rate() * 100.0,
        s.fast_path_share() * 100.0,
        s.guard_sprawl_rate() * 100.0,
        s.verify_hit_rate() * 100.0,
    );
}

fn pct(numer: u64, denom: u64) -> f64 {
    if denom == 0 { 0.0 } else { (numer as f64 / denom as f64) * 100.0 }
}
