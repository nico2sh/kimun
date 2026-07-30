//! Layout cost at the size that matters.
//!
//! `tui`'s `wrap_5000_lines` set the budget this replaces: under a millisecond for
//! a full wrap of a 5 000-row note, since it runs on every text-mutating frame.
//! Wrapping over a rope reads rows through `Text::line` rather than indexing a
//! `Vec<String>`, so the question this answers is whether owning the layout cost
//! anything measurable.

use criterion::{Criterion, black_box, criterion_group, criterion_main};
use ropetext::{EditBuffer, Layout, Metrics, Text};

/// The same corpus `tui`'s `wrap_5000_lines` uses, so the two numbers can be
/// compared. Each row is one wrap at width 80.
fn note(rows: usize) -> Text {
    let mut out = String::new();
    for row in 0..rows {
        out.push_str(&format!(
            "paragraph number {row} with some sample text to give the parser work to do\n"
        ));
    }
    Text::from(out.as_str())
}

fn bench_compute(c: &mut Criterion) {
    let text = note(5_000);
    c.bench_function("layout_compute_5000_rows", |b| {
        b.iter(|| {
            let layout = Layout::compute(black_box(&text), 80, Metrics::default(), &[]);
            black_box(layout.visual_line_count())
        })
    });
}

fn bench_relayout(c: &mut Criterion) {
    let text = note(5_000);
    let mut buffer = EditBuffer::new(text);
    let mut layout = Layout::compute(buffer.text(), 80, Metrics::default(), &[]);
    c.bench_function("layout_relayout_one_row_of_5000", |b| {
        b.iter(|| {
            // Type a character and take it back, so the measured row does not grow
            // over the run — otherwise the wrap work climbs with the iteration
            // count and the number means nothing.
            let at = buffer
                .text()
                .position(2_500, ropetext::Column::new(1))
                .expect("addressable");
            let mut txn = buffer.begin();
            txn.insert(at, "x");
            let change = txn.commit().expect("changed");
            layout.relayout_rows(
                buffer.text(),
                &[],
                black_box(change.rows()),
                change.line_delta(),
            );
            let undone = buffer.undo().expect("there was a change");
            layout.relayout_rows(buffer.text(), &[], undone.rows(), undone.line_delta());
            black_box(layout.visual_line_count())
        })
    });
}

criterion_group!(benches, bench_compute, bench_relayout);
criterion_main!(benches);
