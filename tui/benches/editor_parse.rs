//! Criterion benchmarks for the incremental-parse machinery.
//!
//! Targets (per openspec/changes/incremental-parsed-buffer/):
//! - full_parse_5000_lines: 5–20 ms (reference)
//! - incremental_paragraph_insert_5000_lines: < 1 ms
//! - incremental_fallback_5000_lines: ≈ full_parse_5000_lines ± 5%
//! - wrap_5000_lines: if > 1 ms, open a wrap-incremental follow-up
//!   change (G4 trigger).

use criterion::{BatchSize, Criterion, black_box, criterion_group, criterion_main};
use kimun_notes::components::text_editor::markdown::ParsedBuffer;
use kimun_notes::components::text_editor::parse_incremental::{
    WidenResult, compute_damage_range, widen_to_safe,
};
use kimun_notes::components::text_editor::snapshot::EditorSnapshot;
use kimun_notes::components::text_editor::word_wrap::WordWrapLayout;
use std::num::NonZeroU64;

fn snap_for<'a>(
    lines: &'a [String],
    cursor: (usize, usize),
    generation: u64,
) -> EditorSnapshot<'a> {
    let rev = NonZeroU64::new(generation.max(1)).unwrap();
    let clamped = if lines.is_empty() {
        (0, 0)
    } else {
        (cursor.0.min(lines.len() - 1), cursor.1)
    };
    EditorSnapshot::borrowed(lines, clamped, rev)
}

fn make_5000_line_buffer() -> Vec<String> {
    (0..5000)
        .map(|i| {
            format!("paragraph number {i} with some sample text to give the parser work to do")
        })
        .collect()
}

fn bench_full_parse_5000_lines(c: &mut Criterion) {
    let lines = make_5000_line_buffer();
    c.bench_function("full_parse_5000_lines", |b| {
        b.iter(|| {
            let pb = ParsedBuffer::parse(black_box(&lines));
            black_box(pb);
        });
    });
}

fn bench_compute_damage_range_5000_lines(c: &mut Criterion) {
    let lines = make_5000_line_buffer();
    let mut edited = lines.clone();
    // Backspace at line boundary: shrinks the buffer by one row,
    // forcing compute_damage_range's slow LCP/LCS path (the fast
    // cursor-hint path bails on line-count changes).
    edited.remove(2500);
    c.bench_function("compute_damage_range_backspace_5000_lines", |b| {
        b.iter(|| {
            let r = compute_damage_range(black_box(&lines), black_box(&edited), 2500);
            black_box(r);
        });
    });
}

fn bench_incremental_paragraph_insert_5000_lines(c: &mut Criterion) {
    let lines = make_5000_line_buffer();
    let initial_pb = ParsedBuffer::parse(&lines);
    let mut edited = lines.clone();
    edited[2500].push('x');

    c.bench_function("incremental_paragraph_insert_5000_lines", |b| {
        b.iter_batched(
            || initial_pb.clone(),
            |mut pb| {
                let damaged =
                    compute_damage_range(&lines, &edited, 2500).expect("damaged should be Some");
                let widened = match widen_to_safe(&pb.kinds, damaged) {
                    WidenResult::Widened(r) => r,
                    WidenResult::FullRebuild => {
                        panic!("paragraph insert should take incremental path")
                    }
                };
                let slice = ParsedBuffer::parse_range(black_box(&edited), widened.clone());
                pb.splice(widened, slice);
                black_box(pb);
            },
            BatchSize::SmallInput,
        );
    });
}

fn bench_incremental_fallback_5000_lines(c: &mut Criterion) {
    let lines = make_5000_line_buffer();
    let _initial_pb = ParsedBuffer::parse(&lines);
    // Insert ``` at row 2500 — line count changes → fallback path.
    let mut edited = lines.clone();
    edited.insert(2500, "```".to_string());

    c.bench_function("incremental_fallback_5000_lines", |b| {
        b.iter(|| {
            // Simulate the fallback path: full parse of the edited buffer.
            let pb = ParsedBuffer::parse(black_box(&edited));
            black_box(pb);
        });
    });
}

fn bench_wrap_5000_lines(c: &mut Criterion) {
    let lines = make_5000_line_buffer();
    let pb = ParsedBuffer::parse(&lines);
    let rendered: Vec<Vec<bool>> = pb.lines.iter().map(|p| p.content_vis.clone()).collect();
    c.bench_function("wrap_5000_lines", |b| {
        b.iter(|| {
            let layout = WordWrapLayout::compute(black_box(&lines), 80, &rendered);
            black_box(layout);
        });
    });
}

fn bench_full_view_update_5000_lines_incremental(c: &mut Criterion) {
    use kimun_notes::components::text_editor::view::MarkdownEditorView;
    use ratatui::layout::Rect;

    let lines = make_5000_line_buffer();
    let rect = Rect {
        x: 0,
        y: 0,
        width: 80,
        height: 40,
    };

    // Warm the view: do a full parse on the original buffer once.
    let mut warmed = MarkdownEditorView::new();
    warmed.update(&snap_for(&lines, (2500, 0), 1), rect, None);

    // Edited buffer: single-char insert at row 2500 (same line count).
    let mut edited = lines.clone();
    edited[2500].push('x');

    c.bench_function("full_view_update_5000_lines_incremental", |b| {
        b.iter_batched(
            || warmed.clone(),
            |mut v| {
                v.update(
                    &snap_for(black_box(&edited), (2500, edited[2500].len()), 2),
                    rect,
                    None,
                );
                black_box(v);
            },
            BatchSize::SmallInput,
        );
    });
}

fn bench_full_view_update_5000_lines_backspace(c: &mut Criterion) {
    use kimun_notes::components::text_editor::view::MarkdownEditorView;
    use ratatui::layout::Rect;

    let lines = make_5000_line_buffer();
    let rect = Rect {
        x: 0,
        y: 0,
        width: 80,
        height: 40,
    };

    let mut warmed = MarkdownEditorView::new();
    warmed.update(&snap_for(&lines, (2500, 0), 1), rect, None);

    // Edited buffer: single-char delete at row 2500 (Backspace mid-line).
    let mut edited = lines.clone();
    edited[2500].pop();

    c.bench_function("full_view_update_5000_lines_backspace", |b| {
        b.iter_batched(
            || warmed.clone(),
            |mut v| {
                v.update(
                    &snap_for(black_box(&edited), (2500, edited[2500].len()), 2),
                    rect,
                    None,
                );
                black_box(v);
            },
            BatchSize::SmallInput,
        );
    });
}

fn bench_full_view_update_5000_lines_first_parse(c: &mut Criterion) {
    use kimun_notes::components::text_editor::view::MarkdownEditorView;
    use ratatui::layout::Rect;

    let lines = make_5000_line_buffer();
    let rect = Rect {
        x: 0,
        y: 0,
        width: 80,
        height: 40,
    };

    c.bench_function("full_view_update_5000_lines_first_parse", |b| {
        b.iter(|| {
            let mut v = MarkdownEditorView::new();
            v.update(&snap_for(black_box(&lines), (0, 0), 1), rect, None);
            black_box(v);
        });
    });
}

criterion_group!(
    benches,
    bench_full_parse_5000_lines,
    bench_compute_damage_range_5000_lines,
    bench_incremental_paragraph_insert_5000_lines,
    bench_incremental_fallback_5000_lines,
    bench_wrap_5000_lines,
    bench_full_view_update_5000_lines_incremental,
    bench_full_view_update_5000_lines_first_parse,
    bench_full_view_update_5000_lines_backspace,
);
criterion_main!(benches);
