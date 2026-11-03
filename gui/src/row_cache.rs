//! A frame-to-frame cache of painted rows, keyed by the row's content (its
//! runs).
//!
//! Every prepaint walks the visible rows. Most of them are unchanged since the
//! last frame (typing touches one row, scrolling brings in a few), so the
//! work per row (cell layout, styles, shaping) is done once per distinct
//! content and reused while that content stays on screen. Entries live two
//! frames: what a frame does not use is dropped at the next frame's end, so
//! the cache never grows past about two screens of rows.
//!
//! Invalidation: everything a cached row depends on besides its runs (face,
//! size, cell width, theme) goes in the frame key `K`; a different key
//! empties the cache.

use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::rc::Rc;

use kimun_notes::logic::editor::markdown::runs::Run;

type Bucket<V> = Vec<(Vec<Run>, Rc<V>)>;

pub struct RowCache<K, V> {
    key: Option<K>,
    current: HashMap<u64, Bucket<V>>,
    previous: HashMap<u64, Bucket<V>>,
    /// Rows built (cache misses) since the cache was made: the work counter
    /// the performance tests read.
    #[cfg(test)]
    built: usize,
}

impl<K, V> Default for RowCache<K, V> {
    fn default() -> Self {
        Self {
            key: None,
            current: HashMap::new(),
            previous: HashMap::new(),
            #[cfg(test)]
            built: 0,
        }
    }
}

impl<K: PartialEq, V> RowCache<K, V> {
    /// Start a frame painted under `key`; a key unlike the last frame's
    /// drops every row.
    pub fn begin_frame(&mut self, key: K) {
        if self.key.as_ref() != Some(&key) {
            self.current.clear();
            self.previous.clear();
            self.key = Some(key);
        }
    }

    /// The row painted for `runs`, built by `build` only when no frame since
    /// the last one painted the same runs.
    pub fn get_or_build(&mut self, runs: &[Run], build: impl FnOnce() -> V) -> Rc<V> {
        let hash = hash_runs(runs);
        if let Some(hit) = find(&self.current, hash, runs) {
            return hit;
        }
        // A row kept from the last frame moves over with the runs it was
        // stored under; only a new row copies its runs.
        let entry = match take(&mut self.previous, hash, runs) {
            Some(kept) => kept,
            None => {
                #[cfg(test)]
                {
                    self.built += 1;
                }
                (runs.to_vec(), Rc::new(build()))
            }
        };
        let value = entry.1.clone();
        self.current.entry(hash).or_default().push(entry);
        value
    }

    /// End the frame: rows it did not use are dropped at the next one's end.
    pub fn end_frame(&mut self) {
        self.previous = std::mem::take(&mut self.current);
    }

    #[cfg(test)]
    pub fn built(&self) -> usize {
        self.built
    }
}

fn find<V>(map: &HashMap<u64, Bucket<V>>, hash: u64, runs: &[Run]) -> Option<Rc<V>> {
    map.get(&hash)?
        .iter()
        .find(|(r, _)| r.as_slice() == runs)
        .map(|(_, v)| v.clone())
}

fn take<V>(
    map: &mut HashMap<u64, Bucket<V>>,
    hash: u64,
    runs: &[Run],
) -> Option<(Vec<Run>, Rc<V>)> {
    let bucket = map.get_mut(&hash)?;
    let ix = bucket.iter().position(|(r, _)| r.as_slice() == runs)?;
    Some(bucket.swap_remove(ix))
}

fn hash_runs(runs: &[Run]) -> u64 {
    let mut h = DefaultHasher::new();
    for run in runs {
        run.text.hash(&mut h);
        run.role.hash(&mut h);
        run.mods.hash(&mut h);
        run.code_box.hash(&mut h);
        for kind in run.overlays.iter() {
            kind.hash(&mut h);
        }
        // Run boundary: "ab"+"c" differs from "a"+"bc".
        0xffu8.hash(&mut h);
    }
    h.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use kimun_notes::logic::editor::markdown::runs::Role;
    use kimun_notes::logic::editor::overlay::OverlayKind;

    fn row(text: &str) -> Vec<Run> {
        vec![Run::new(text, Role::Plain, 0)]
    }

    #[test]
    fn an_unchanged_row_is_built_once_across_frames() {
        let mut c: RowCache<u8, String> = RowCache::default();
        for _ in 0..3 {
            c.begin_frame(1);
            assert_eq!(*c.get_or_build(&row("a"), || "A".into()), "A");
            c.end_frame();
        }
        assert_eq!(c.built(), 1);
    }

    #[test]
    fn any_difference_in_the_runs_rebuilds() {
        let mut c: RowCache<u8, u32> = RowCache::default();
        c.begin_frame(1);
        c.get_or_build(&row("ab"), || 0);
        let split = vec![Run::new("a", Role::Plain, 0), Run::new("b", Role::Plain, 0)];
        c.get_or_build(&split, || 1);
        let mut sel = row("ab");
        sel[0].overlays.insert(OverlayKind::Selection);
        c.get_or_build(&sel, || 2);
        c.get_or_build(&[Run::new("ab", Role::Sigil, 0)], || 3);
        assert_eq!(c.built(), 4);
        assert_eq!(*c.get_or_build(&sel, || 9), 2);
        assert_eq!(c.built(), 4);
    }

    #[test]
    fn rows_unused_for_a_frame_are_dropped_and_a_new_key_clears() {
        let mut c: RowCache<u8, u32> = RowCache::default();
        c.begin_frame(1);
        c.get_or_build(&row("a"), || 0);
        c.end_frame();
        // Frame 2 does not paint "a": kept one more frame only.
        c.begin_frame(1);
        c.end_frame();
        c.begin_frame(1);
        c.get_or_build(&row("a"), || 0);
        assert_eq!(c.built(), 2);
        c.end_frame();
        // Another face or size: everything is rebuilt.
        c.begin_frame(2);
        c.get_or_build(&row("a"), || 0);
        assert_eq!(c.built(), 3);
    }
}
