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
use kimun_notes::ropetext::{Layout, Metrics, RowHints, Text};
use std::num::NonZeroU64;

fn snap_for(lines: &[String], cursor: (usize, usize), generation: u64) -> EditorSnapshot {
    let rev = NonZeroU64::new(generation.max(1)).unwrap();
    let clamped = if lines.is_empty() {
        (0, 0)
    } else {
        (cursor.0.min(lines.len() - 1), cursor.1)
    };
    EditorSnapshot::borrowed(lines, clamped, rev)
}

/// A view warmed on `lines`, with any deferred work completed.
///
/// Above `LARGE_BUFFER_THRESHOLD` (1000 rows) the first update installs a
/// placeholder parse and an unwrapped layout stub and hands the real work to a
/// background task. A bench that never completes it measures a view permanently
/// waiting: every later update re-stubs, which is O(rows), so the incremental
/// path it means to measure is buried. The component installs the results when
/// the task returns; so does this.
fn warmed_view(
    lines: &[String],
    cursor: (usize, usize),
    rect: ratatui::layout::Rect,
) -> kimun_notes::components::text_editor::view::MarkdownEditorView {
    use kimun_notes::components::text_editor::view::MarkdownEditorView;

    let mut view = MarkdownEditorView::new();
    view.update(&snap_for(lines, cursor, 1), rect);
    let text = Text::from(lines.join("\n").as_str());
    if let Some(generation) = view.take_pending_full_parse() {
        view.install_full_parse(generation, ParsedBuffer::parse(&text));
    }
    if let Some(job) = view.take_pending_full_layout() {
        let hints: Vec<RowHints<'_>> = Vec::new();
        let layout = Layout::compute(&job.text, job.width, Metrics::default(), &hints);
        view.install_full_layout(job.generation, layout);
    }
    view
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
            let pb = ParsedBuffer::parse_lines(black_box(&lines));
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
    let initial_pb = ParsedBuffer::parse_lines(&lines);
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
                let slice = ParsedBuffer::parse_range_lines(black_box(&edited), widened.clone());
                pb.splice(widened, slice);
                black_box(pb);
            },
            BatchSize::SmallInput,
        );
    });
}

fn bench_incremental_fallback_5000_lines(c: &mut Criterion) {
    let lines = make_5000_line_buffer();
    let _initial_pb = ParsedBuffer::parse_lines(&lines);
    // Insert ``` at row 2500 — line count changes → fallback path.
    let mut edited = lines.clone();
    edited.insert(2500, "```".to_string());

    c.bench_function("incremental_fallback_5000_lines", |b| {
        b.iter(|| {
            // Simulate the fallback path: full parse of the edited buffer.
            let pb = ParsedBuffer::parse_lines(black_box(&edited));
            black_box(pb);
        });
    });
}

fn bench_wrap_5000_lines(c: &mut Criterion) {
    let lines = make_5000_line_buffer();
    let pb = ParsedBuffer::parse_lines(&lines);
    let rendered: Vec<Vec<bool>> = pb.lines.iter().map(|p| p.content_vis.clone()).collect();
    c.bench_function("wrap_5000_lines", |b| {
        b.iter(|| {
            let text = Text::from(lines.join("\n").as_str());
            let hints: Vec<RowHints<'_>> = rendered
                .iter()
                .map(|row| RowHints {
                    visible: row.as_slice(),
                    inset: 0,
                })
                .collect();
            let layout = Layout::compute(black_box(&text), 80, Metrics::default(), &hints);
            black_box(layout);
        });
    });
}

fn bench_full_view_update_5000_lines_incremental(c: &mut Criterion) {
    use ratatui::layout::Rect;

    let lines = make_5000_line_buffer();
    let rect = Rect {
        x: 0,
        y: 0,
        width: 80,
        height: 40,
    };

    // Warm the view: do a full parse on the original buffer once.
    let warmed = warmed_view(&lines, (2500, 0), rect);

    // Edited buffer: single-char insert at row 2500 (same line count).
    let mut edited = lines.clone();
    edited[2500].push('x');

    c.bench_function("full_view_update_5000_lines_incremental", |b| {
        b.iter_batched(
            || warmed.clone(),
            |mut v| {
                // The component tells the view which rows its edit touched.
                // Without it the view diffs two materialised copies of the whole
                // note — work the editor never does, and ~6x the real cost.
                v.note_damage(2500..2501, 0);
                v.update(
                    &snap_for(black_box(&edited), (2500, edited[2500].len()), 2),
                    rect,
                );
                black_box(v);
            },
            BatchSize::SmallInput,
        );
    });
}

fn bench_full_view_update_5000_lines_backspace(c: &mut Criterion) {
    use ratatui::layout::Rect;

    let lines = make_5000_line_buffer();
    let rect = Rect {
        x: 0,
        y: 0,
        width: 80,
        height: 40,
    };

    let warmed = warmed_view(&lines, (2500, 0), rect);

    // Edited buffer: single-char delete at row 2500 (Backspace mid-line).
    let mut edited = lines.clone();
    edited[2500].pop();

    c.bench_function("full_view_update_5000_lines_backspace", |b| {
        b.iter_batched(
            || warmed.clone(),
            |mut v| {
                // The component tells the view which rows its edit touched.
                // Without it the view diffs two materialised copies of the whole
                // note — work the editor never does, and ~6x the real cost.
                v.note_damage(2500..2501, 0);
                v.update(
                    &snap_for(black_box(&edited), (2500, edited[2500].len()), 2),
                    rect,
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
            v.update(&snap_for(black_box(&lines), (0, 0), 1), rect);
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
        out.push(format!(
            "- list item {i} with text content for editing tests"
        ));
        if i % 7 == 0 {
            out.push(String::new());
        }
    }
    out
}

fn bench_full_view_update_heavy_lists_typing(c: &mut Criterion) {
    use ratatui::layout::Rect;

    let lines = make_heavy_lists_buffer();
    let rect = Rect {
        x: 0,
        y: 0,
        width: 80,
        height: 40,
    };

    let target_row = 250.min(lines.len() - 1);
    let warmed = warmed_view(&lines, (target_row, 0), rect);

    // Single-char append inside an item's content. Pre-edit row is a
    // ListMarker (lazy_depth == 1) inside the loose list. The v3 §3.0
    // relaxation skips the lazy_depth guard for this shape; the
    // intra-construct widener tier finds an End(Item) boundary and
    // splices a narrow slice. Pre-v3 this fixture cap-tripped to a
    // full ParsedBuffer::parse (~493 µs); post-v3 it lands at ~36 µs.
    let mut edited = lines.clone();
    edited[target_row].push('x');

    c.bench_function("full_view_update_heavy_lists_571_typing", |b| {
        b.iter_batched(
            || warmed.clone(),
            |mut v| {
                v.note_damage(target_row..target_row + 1, 0);
                v.update(
                    &snap_for(
                        black_box(&edited),
                        (target_row, edited[target_row].len()),
                        2,
                    ),
                    rect,
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
            v.update(&snap_for(black_box(&lines), (0, 0), 1), rect);
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
    let warmed = warmed_view(&lines, (target_row, 0), rect);

    let mut edited = lines.clone();
    edited[target_row].push('x');

    c.bench_function("full_view_update_blockquotes_400_typing", |b| {
        b.iter_batched(
            || warmed.clone(),
            |mut v| {
                v.note_damage(target_row..target_row + 1, 0);
                v.update(
                    &snap_for(
                        black_box(&edited),
                        (target_row, edited[target_row].len()),
                        2,
                    ),
                    rect,
                );
                black_box(v);
            },
            BatchSize::SmallInput,
        );
    });
}

/// The incremental parse as the view actually runs it.
///
/// `bench_incremental_paragraph_insert_5000_lines` above measures
/// `parse_range_lines`, which takes rows the caller already has. The live path
/// (`view.rs`, `try_incremental_parse`) holds a `Text` and calls `parse_range`,
/// which has to produce the rows itself — so that is where a full-buffer copy
/// could hide, and nothing was measuring it.
fn bench_incremental_range_parse_5000_lines(c: &mut Criterion) {
    let lines = make_5000_line_buffer();
    let pb = ParsedBuffer::parse_lines(&lines);
    let mut edited = lines.clone();
    edited[2500].push('x');
    let text = Text::from(edited.join("\n").as_str());

    let damaged = compute_damage_range(&lines, &edited, 2500).expect("damaged should be Some");
    let widened = match widen_to_safe(&pb.kinds, damaged) {
        WidenResult::Widened(r) => r,
        WidenResult::FullRebuild => panic!("a paragraph edit should stay incremental"),
    };

    c.bench_function("incremental_range_parse_5000_lines", |b| {
        b.iter(|| {
            let slice = ParsedBuffer::parse_range(black_box(&text), widened.clone());
            black_box(slice);
        });
    });
}

/// What pressing Enter costs, against what typing a character costs.
///
/// A newline changes the line count, which makes `try_incremental_parse` bail
/// AND makes `view.rs`'s layout gate take the full-rebuild branch — so the whole
/// document is re-parsed and re-wrapped. Typing a character does neither. The
/// pair is the measurement: the gap between them is what an incremental path
/// across a line-count change would be worth.
///
/// Run at two sizes because the behaviour differs either side of
/// `LARGE_BUFFER_THRESHOLD` (1000 rows): above it both gates install a stub and
/// defer to a background task, so the keystroke does not pay the rebuild — it
/// pays a frame of unstyled, unwrapped rendering instead.
fn bench_newline_vs_typing(c: &mut Criterion) {
    use kimun_notes::components::text_editor::view::MarkdownEditorView;
    use ratatui::layout::Rect;

    let rect = Rect {
        x: 0,
        y: 0,
        width: 80,
        height: 40,
    };

    for rows in [800usize, 5000] {
        let lines: Vec<String> = (0..rows)
            .map(|i| {
                format!("paragraph number {i} with some sample text to give the parser work to do")
            })
            .collect();
        let mid = rows / 2;

        // Build every `Text` ONCE. `EditorSnapshot::borrowed` joins the rows and
        // constructs a rope, which is O(rows) — inside the loop it costs more
        // than the thing being measured and hides it completely. The editor uses
        // `of_buffer`, which rebuilds nothing, so that is what this must use.
        let base = Text::from(lines.join("\n").as_str());

        let mut typed_lines = lines.clone();
        typed_lines[mid].push('x');
        let typed = Text::from(typed_lines.join("\n").as_str());

        // Enter: the row splits in two, so the line count changes.
        let mut split_lines = lines.clone();
        let tail = split_lines[mid].split_off(20);
        split_lines.insert(mid + 1, tail);
        let split = Text::from(split_lines.join("\n").as_str());

        let rev = |n: u64| NonZeroU64::new(n).unwrap();
        let mut warmed = MarkdownEditorView::new();
        warmed.update(
            &EditorSnapshot::of_buffer(base.clone(), (mid, 0), rev(1)),
            rect,
        );
        // Above `LARGE_BUFFER_THRESHOLD` the first update installs a placeholder
        // parse and an unwrapped layout stub and defers the real work to a
        // background task. A bench that never completes that work measures a view
        // permanently waiting — every later update re-stubs, which is O(rows) and
        // buries the difference this is trying to see. Do what the component does
        // when the task returns.
        if let Some(generation) = warmed.take_pending_full_parse() {
            warmed.install_full_parse(generation, ParsedBuffer::parse(&base));
        }
        if let Some(job) = warmed.take_pending_full_layout() {
            let hints: Vec<RowHints<'_>> = Vec::new();
            let layout = Layout::compute(&job.text, job.width, Metrics::default(), &hints);
            warmed.install_full_layout(job.generation, layout);
        }

        let mut group = c.benchmark_group(format!("keystroke_{rows}_rows"));
        group.bench_function("typing", |b| {
            b.iter_batched(
                || warmed.clone(),
                |mut v| {
                    // The component reports the rows its own edit touched; without
                    // this the view falls back to diffing two materialised copies
                    // of the whole note, which is not what the editor does.
                    v.note_damage(mid..mid + 1, 0);
                    let snap =
                        EditorSnapshot::of_buffer(black_box(typed.clone()), (mid, 21), rev(2));
                    v.update(&snap, rect);
                    black_box(v);
                },
                BatchSize::SmallInput,
            );
        });
        group.bench_function("newline", |b| {
            b.iter_batched(
                || warmed.clone(),
                |mut v| {
                    v.note_damage(mid..mid + 2, 1);
                    let snap =
                        EditorSnapshot::of_buffer(black_box(split.clone()), (mid + 1, 0), rev(2));
                    v.update(&snap, rect);
                    black_box(v);
                },
                BatchSize::SmallInput,
            );
        });
        group.finish();
    }
}

criterion_group!(
    benches,
    bench_newline_vs_typing,
    bench_incremental_range_parse_5000_lines,
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
