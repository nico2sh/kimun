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

/// 571-row loose-list buffer matching `dev-fixtures/widener-stress/
/// heavy_lists_loose.md`: 500 unordered list items + a blank row
/// every 7th item. The whole buffer is ONE CommonMark loose list
/// per §5.2 — every row has `lazy_depth == 1`, the v2 structural
/// guard rejects every edit, and both wideners cap-trip. The
/// `widener_metrics` session data showed 0% incremental success on
/// this shape. This bench measures the actual full-rebuild cost of
/// a single-char edit so we can decide whether the limitation is a
/// product issue or stays within typing-latency budget.
fn make_heavy_lists_buffer() -> Vec<String> {
    let mut out = Vec::with_capacity(571);
    for i in 1..=500 {
        out.push(format!("- list item {i} with text content for editing tests"));
        if i % 7 == 0 {
            out.push(String::new());
        }
    }
    out
}

fn bench_full_view_update_heavy_lists_typing(c: &mut Criterion) {
    use kimun_notes::components::text_editor::view::MarkdownEditorView;
    use ratatui::layout::Rect;

    let lines = make_heavy_lists_buffer();
    let rect = Rect {
        x: 0,
        y: 0,
        width: 80,
        height: 40,
    };

    let target_row = 250.min(lines.len() - 1);
    let mut warmed = MarkdownEditorView::new();
    warmed.update(&snap_for(&lines, (target_row, 0), 1), rect, None);

    // Single-char append inside an item's content. Pre-edit row is a
    // ListMarker inside the loose list; the v2 lazy_depth guard will
    // bail and the view falls back to a full ParsedBuffer::parse.
    let mut edited = lines.clone();
    edited[target_row].push('x');

    c.bench_function("full_view_update_heavy_lists_571_typing", |b| {
        b.iter_batched(
            || warmed.clone(),
            |mut v| {
                v.update(
                    &snap_for(black_box(&edited), (target_row, edited[target_row].len()), 2),
                    rect,
                    None,
                );
                black_box(v);
            },
            BatchSize::SmallInput,
        );
    });
}

fn bench_full_view_update_heavy_lists_first_parse(c: &mut Criterion) {
    use kimun_notes::components::text_editor::view::MarkdownEditorView;
    use ratatui::layout::Rect;

    let lines = make_heavy_lists_buffer();
    let rect = Rect {
        x: 0,
        y: 0,
        width: 80,
        height: 40,
    };

    c.bench_function("full_view_update_heavy_lists_571_first_parse", |b| {
        b.iter(|| {
            let mut v = MarkdownEditorView::new();
            v.update(&snap_for(black_box(&lines), (0, 0), 1), rect, None);
            black_box(v);
        });
    });
}

/// 400-row blockquote buffer modelled on `example/work/widener-stress/
/// blockquotes_lazy.md`: 100 blockquotes, each followed by a
/// lazy-continuation paragraph row and a blank separator. Edits to
/// the `>` row exercise the intra-construct widener on the
/// blockquote-end boundary; edits to the lazy-continuation row land
/// inside the blockquote (lazy_depth > 0, Plain kind) and bail at
/// the §3.0 guard (Plain is NOT in the qualifying set).
///
/// This bench measures the intra-construct win on the `> a` row
/// pattern. Once the §3.0 relaxation widens to include `Plain` (via
/// a post-widening sanity check), the lazy-continuation row will
/// also become incremental.
fn make_blockquotes_lazy_buffer() -> Vec<String> {
    let mut out = Vec::with_capacity(400);
    for i in 1..=100 {
        out.push(format!("> Blockquote paragraph {i}"));
        out.push(format!("lazy continuation line for paragraph {i}"));
        out.push("another continuation line".to_string());
        out.push(String::new());
    }
    out
}

fn bench_full_view_update_blockquotes_typing(c: &mut Criterion) {
    use kimun_notes::components::text_editor::view::MarkdownEditorView;
    use ratatui::layout::Rect;

    let lines = make_blockquotes_lazy_buffer();
    let rect = Rect {
        x: 0,
        y: 0,
        width: 80,
        height: 40,
    };

    // Edit the `>` row of the 50th blockquote (row 50*4 = 200).
    let target_row = 200;
    let mut warmed = MarkdownEditorView::new();
    warmed.update(&snap_for(&lines, (target_row, 0), 1), rect, None);

    let mut edited = lines.clone();
    edited[target_row].push('x');

    c.bench_function("full_view_update_blockquotes_400_typing", |b| {
        b.iter_batched(
            || warmed.clone(),
            |mut v| {
                v.update(
                    &snap_for(black_box(&edited), (target_row, edited[target_row].len()), 2),
                    rect,
                    None,
                );
                black_box(v);
            },
            BatchSize::SmallInput,
        );
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
    bench_full_view_update_heavy_lists_typing,
    bench_full_view_update_heavy_lists_first_parse,
    bench_full_view_update_blockquotes_typing,
);
criterion_main!(benches);
