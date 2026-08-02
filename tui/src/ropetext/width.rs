//! How many terminal cells a piece of text occupies.

use unicode_width::UnicodeWidthStr;

/// Cell measurements a layout needs, so a caller that renders differently can
/// say so instead of the layout assuming.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Metrics {
    /// Cells between tab stops. A tab advances to the next multiple of this.
    pub tab_width: usize,
}

impl Default for Metrics {
    fn default() -> Self {
        Self {
            tab_width: Metrics::DEFAULT_TAB_WIDTH,
        }
    }
}

impl Metrics {
    /// Cells between tab stops, absent a caller saying otherwise.
    ///
    /// Public because a caller that draws the text this crate wrapped has to
    /// measure a tab the same way, and a second `4` written down beside this one
    /// agrees only by luck. Anything that expands or paints a tab should derive
    /// its stop from here rather than declare its own.
    pub const DEFAULT_TAB_WIDTH: usize = 4;

    /// Cells `cluster` occupies when drawn starting at cell `column`.
    ///
    /// Position-dependent, because a tab's width is the distance to the next tab
    /// stop and nothing else. Measuring a tab as a fixed width — or, as
    /// `unicode-width` alone does, as zero — makes wrapping disagree with the
    /// renderer about where a row ends, and every column derived from either is
    /// then wrong by the difference.
    pub fn width_at(&self, cluster: &str, column: usize) -> usize {
        // The byte check rather than `cluster == "\t"`: this runs once per cluster
        // per wrap, so a full wrap of a large note runs it hundreds of thousands of
        // times, and a length test that fails immediately beats a string compare.
        if cluster.len() == 1 && cluster.as_bytes()[0] == b'\t' {
            let stop = self.tab_width.max(1);
            stop - (column % stop)
        } else {
            cluster.width()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_cluster_is_measured_whole() {
        let m = Metrics::default();
        // The first codepoint of each of these is narrow; the cluster is not.
        assert_eq!(m.width_at("\u{1F1EA}\u{1F1F8}", 0), 2, "flag");
        assert_eq!(m.width_at("\u{2764}\u{FE0F}", 0), 2, "heart with VS16");
        assert_eq!(m.width_at("1\u{FE0F}\u{20E3}", 0), 2, "keycap");
        assert_eq!(m.width_at("\u{3042}", 0), 2, "CJK");
        assert_eq!(m.width_at("a", 0), 1);
        assert_eq!(m.width_at("e\u{301}", 0), 1, "e plus combining acute");
    }

    #[test]
    fn zero_width_clusters_measure_zero() {
        let m = Metrics::default();
        for zero in ["\u{200B}", "\u{00AD}", "\u{200C}", "\u{FEFF}", "\u{301}"] {
            assert_eq!(m.width_at(zero, 0), 0, "{zero:?}");
        }
    }

    #[test]
    fn a_tab_advances_to_the_next_stop() {
        let m = Metrics::default();
        assert_eq!(m.width_at("\t", 0), 4);
        assert_eq!(m.width_at("\t", 1), 3);
        assert_eq!(m.width_at("\t", 3), 1);
        assert_eq!(m.width_at("\t", 4), 4);
    }

    #[test]
    fn a_tab_width_of_zero_still_advances() {
        let m = Metrics { tab_width: 0 };
        assert_eq!(m.width_at("\t", 0), 1, "forward progress is not optional");
    }
}
