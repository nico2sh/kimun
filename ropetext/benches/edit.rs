//! What an edit costs on a note big enough to matter.
//!
//! These are the numbers the design is accountable to. `tui`'s old line shim
//! rebuilt a `Vec<String>` on every mutation — 1.06 ms at 5 000 rows, per
//! keystroke — and the case for owning the buffer rests on that becoming
//! invisible. An insert should be dominated by the rope splice, an undo should
//! be restoring a snapshot rather than replaying anything, and taking a snapshot
//! should not scale with the note at all.

use criterion::{BatchSize, Criterion, black_box, criterion_group, criterion_main};
use ropetext::{Column, EditBuffer, Text};

/// The same corpus `benches/layout.rs` uses, so the two files' numbers describe
/// the same note.
fn note(rows: usize) -> Text {
    let mut out = String::new();
    for row in 0..rows {
        out.push_str(&format!(
            "paragraph number {row} with some sample text to give the parser work to do\n"
        ));
    }
    Text::from(out.as_str())
}

/// A buffer over `note(5_000)` with the cursor parked mid-note.
///
/// Cloning the text is a pointer copy, which is the only reason a fresh buffer
/// per iteration is affordable — otherwise the setup would cost more than the
/// thing being measured.
fn buffer_at_midpoint(text: &Text) -> EditBuffer {
    let mut buffer = EditBuffer::new(text.clone());
    let at = buffer
        .text()
        .position(2_500, Column::new(1))
        .expect("addressable");
    buffer.set_cursor(at);
    buffer
}

fn bench_insert(c: &mut Criterion) {
    let text = note(5_000);
    c.bench_function("insert_at_5000_lines", |b| {
        // Batched rather than insert-then-undo: undoing inside the routine would
        // measure both, and a buffer that grows over the run measures neither.
        b.iter_batched(
            || buffer_at_midpoint(&text),
            |mut buffer| {
                // From the cursor, as a keystroke does — the position is already
                // resolved, so this is the edit and not a lookup.
                let at = buffer.cursor();
                let mut txn = buffer.begin();
                txn.insert(at, "x");
                black_box(txn.commit())
            },
            BatchSize::SmallInput,
        )
    });
}

fn bench_undo(c: &mut Criterion) {
    let text = note(5_000);
    c.bench_function("undo_5000_lines", |b| {
        b.iter_batched(
            || {
                let mut buffer = buffer_at_midpoint(&text);
                let at = buffer.cursor();
                let mut txn = buffer.begin();
                txn.insert(at, "x");
                txn.commit().expect("changed");
                buffer
            },
            |mut buffer| black_box(buffer.undo()),
            BatchSize::SmallInput,
        )
    });
}

fn bench_snapshot(c: &mut Criterion) {
    let mut group = c.benchmark_group("snapshot_clone");
    // Two sizes, because the claim is not "fast" but "flat": a snapshot is the
    // text rather than a copy of it, so 5 000 rows must cost what 10 do.
    for rows in [10usize, 5_000] {
        let buffer = EditBuffer::new(note(rows));
        group.bench_function(format!("{rows}_rows"), |b| {
            b.iter(|| black_box(buffer.snapshot()))
        });
    }
    group.finish();
}

criterion_group!(benches, bench_insert, bench_undo, bench_snapshot);
criterion_main!(benches);
