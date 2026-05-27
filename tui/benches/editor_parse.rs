//! Criterion benchmarks for the editor parse + wrap pipeline.
//!
//! Phase-1 benches: pulldown-cmark full parse + wrap baseline plus the new
//! tree-sitter incremental + autocomplete-trigger probes. Phase-2 (full
//! pulldown→tree-sitter ParsedLine derivation) is tracked as a follow-up
//! change; bench targets for that step land alongside the rewrite.

use criterion::{black_box, criterion_group, criterion_main, BatchSize, Criterion};
use kimun_notes::components::text_editor::markdown::ParsedBuffer;
use kimun_notes::components::text_editor::parse_incremental::lines_diff_to_input_edit;
use kimun_notes::components::text_editor::treesitter_parser::EditorTree;
use kimun_notes::components::text_editor::word_wrap::WordWrapLayout;

/// Realistic note shape: 500 paragraphs of 9 text lines + 1 blank separator.
/// Yields ≈ 5000 lines total. Mirrors how a markdown note is actually
/// structured (block separators), which is what the parsers were built for.
fn make_5000_line_buffer() -> Vec<String> {
    let mut lines = Vec::with_capacity(5000);
    for i in 0..500 {
        for j in 0..9 {
            lines.push(format!(
                "paragraph {i} line {j} with some sample text to give the parser work to do"
            ));
        }
        lines.push(String::new());
    }
    lines
}

/// Same shape but with simulated markdown headings every 50 lines.
#[allow(dead_code)]
fn make_5000_line_structured_buffer() -> Vec<String> {
    let mut lines = make_5000_line_buffer();
    for i in (0..lines.len()).step_by(50) {
        lines[i] = format!("# Section {}", i / 50);
    }
    lines
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

fn bench_treesitter_full_parse_5000_lines(c: &mut Criterion) {
    let lines = make_5000_line_buffer();
    c.bench_function("treesitter_full_parse_5000_lines", |b| {
        b.iter(|| {
            let mut t = EditorTree::new();
            t.parse_full(black_box(&lines));
            black_box(t);
        });
    });
}

fn bench_treesitter_incremental_paragraph_insert_5000_lines(c: &mut Criterion) {
    let lines = make_5000_line_buffer();
    // Insert "x" at the end of row 2500.
    let mut edited = lines.clone();
    edited[2500].push('x');

    c.bench_function("treesitter_incremental_paragraph_insert_5000_lines", |b| {
        b.iter_batched(
            || {
                let mut t = EditorTree::new();
                t.parse_full(&lines);
                t
            },
            |mut t| {
                let edit = lines_diff_to_input_edit(&lines, &edited, 2500)
                    .expect("intra-line insert must produce an InputEdit");
                t.apply_edit(black_box(edit), black_box(&edited));
                black_box(t);
            },
            BatchSize::SmallInput,
        );
    });
}

fn bench_diff_alone(c: &mut Criterion) {
    let lines = make_5000_line_buffer();
    let mut edited = lines.clone();
    edited[2500].push('x');
    c.bench_function("lines_diff_to_input_edit_alone", |b| {
        b.iter(|| {
            let e = lines_diff_to_input_edit(
                black_box(&lines),
                black_box(&edited),
                black_box(2500),
            );
            black_box(e);
        });
    });
}

fn bench_apply_edit_alone(c: &mut Criterion) {
    let lines = make_5000_line_buffer();
    let mut edited = lines.clone();
    edited[2500].push('x');

    c.bench_function("apply_edit_alone", |b| {
        b.iter_batched(
            || {
                let mut t = EditorTree::new();
                t.parse_full(&lines);
                let edit = lines_diff_to_input_edit(&lines, &edited, 2500).unwrap();
                (t, edit)
            },
            |(mut t, edit)| {
                t.apply_edit(black_box(edit), black_box(&edited));
                black_box(t);
            },
            BatchSize::SmallInput,
        );
    });
}

criterion_group!(
    benches,
    bench_full_parse_5000_lines,
    bench_wrap_5000_lines,
    bench_treesitter_full_parse_5000_lines,
    bench_treesitter_incremental_paragraph_insert_5000_lines,
    bench_diff_alone,
    bench_apply_edit_alone,
);
criterion_main!(benches);
