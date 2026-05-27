//! Criterion benchmarks for the incremental-parse machinery.
//!
//! Targets (per openspec/changes/incremental-parsed-buffer/):
//! - full_parse_5000_lines: 5–20 ms (reference)
//! - incremental_paragraph_insert_5000_lines: < 1 ms
//! - incremental_fallback_5000_lines: ≈ full_parse_5000_lines ± 5%
//! - wrap_5000_lines: if > 1 ms, open a wrap-incremental follow-up
//!   change (G4 trigger).

use criterion::{black_box, criterion_group, criterion_main, BatchSize, Criterion};
use kimun_notes::components::text_editor::markdown::ParsedBuffer;
use kimun_notes::components::text_editor::parse_incremental::{
    compute_damage_range, widen_to_safe, WidenResult,
};
use kimun_notes::components::text_editor::word_wrap::WordWrapLayout;

fn make_5000_line_buffer() -> Vec<String> {
    (0..5000)
        .map(|i| format!("paragraph number {i} with some sample text to give the parser work to do"))
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

fn bench_incremental_paragraph_insert_5000_lines(c: &mut Criterion) {
    let lines = make_5000_line_buffer();
    let initial_pb = ParsedBuffer::parse(&lines);
    let mut edited = lines.clone();
    edited[2500].push('x');

    c.bench_function("incremental_paragraph_insert_5000_lines", |b| {
        b.iter_batched(
            || initial_pb.clone(),
            |mut pb| {
                let damaged = compute_damage_range(&lines, &edited, 2500)
                    .expect("damaged should be Some");
                let widened = match widen_to_safe(&pb.kinds, damaged) {
                    WidenResult::Widened(r) => r,
                    WidenResult::FullRebuild => panic!("paragraph insert should take incremental path"),
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
    let rendered: Vec<Vec<bool>> = pb
        .lines
        .iter()
        .map(|p| p.content_vis.clone())
        .collect();
    c.bench_function("wrap_5000_lines", |b| {
        b.iter(|| {
            let layout = WordWrapLayout::compute(black_box(&lines), 80, &rendered);
            black_box(layout);
        });
    });
}

criterion_group!(
    benches,
    bench_full_parse_5000_lines,
    bench_incremental_paragraph_insert_5000_lines,
    bench_incremental_fallback_5000_lines,
    bench_wrap_5000_lines,
);
criterion_main!(benches);
