//! Criterion benchmarks for the editor parse + wrap pipeline.
//!
//! Baseline harness against the current pulldown-cmark implementation.
//! The treesitter-editor-rendering change repairs this file in step 1 so it
//! compiles, captures the pre-change numbers into baseline-bench.txt, and
//! re-expands it in step 9 against the new EditorTree API with tightened
//! targets.

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use kimun_notes::components::text_editor::markdown::ParsedBuffer;
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

fn bench_wrap_5000_lines(c: &mut Criterion) {
    let lines = make_5000_line_buffer();
    let parsed = ParsedBuffer::parse(&lines);
    let rendered: Vec<Vec<bool>> = parsed.iter().map(|p| p.content_vis.clone()).collect();
    c.bench_function("wrap_5000_lines", |b| {
        b.iter(|| {
            let layout = WordWrapLayout::compute(black_box(&lines), 80, &rendered);
            black_box(layout);
        });
    });
}

criterion_group!(benches, bench_full_parse_5000_lines, bench_wrap_5000_lines);
criterion_main!(benches);
