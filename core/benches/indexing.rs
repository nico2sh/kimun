use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use kimun_core::nfs::VaultPath;
use kimun_core::note::{extract_labels, scan::label_matches, NoteDetails};

const SMALL: &str = include_str!("fixtures/small_note.md");
const MEDIUM: &str = include_str!("fixtures/medium_note.md");
const CODE_HEAVY: &str = include_str!("fixtures/code_heavy.md");
const HASHTAG_HEAVY: &str = include_str!("fixtures/hashtag_heavy.md");

fn fixtures() -> Vec<(&'static str, &'static str)> {
    vec![
        ("small", SMALL),
        ("medium", MEDIUM),
        ("code_heavy", CODE_HEAVY),
        ("hashtag_heavy", HASHTAG_HEAVY),
    ]
}

fn bench_get_chunks_and_links(c: &mut Criterion) {
    let mut group = c.benchmark_group("get_chunks_and_links");
    let path = VaultPath::note_path_from("/bench.md");
    for (name, text) in fixtures() {
        group.throughput(Throughput::Bytes(text.len() as u64));
        group.bench_with_input(BenchmarkId::from_parameter(name), &text, |b, &text| {
            b.iter(|| NoteDetails::chunks_and_links_of(black_box(&path), black_box(text)));
        });
    }
    group.finish();
}

fn bench_get_content_chunks(c: &mut Criterion) {
    let mut group = c.benchmark_group("get_content_chunks");
    for (name, text) in fixtures() {
        group.throughput(Throughput::Bytes(text.len() as u64));
        group.bench_with_input(BenchmarkId::from_parameter(name), &text, |b, &text| {
            b.iter(|| NoteDetails::content_chunks_of(black_box(text)));
        });
    }
    group.finish();
}

fn bench_label_matches(c: &mut Criterion) {
    let mut group = c.benchmark_group("label_matches");
    for (name, text) in fixtures() {
        group.throughput(Throughput::Bytes(text.len() as u64));
        group.bench_with_input(BenchmarkId::from_parameter(name), &text, |b, &text| {
            b.iter(|| {
                let v: Vec<_> = label_matches(black_box(text)).collect();
                v
            });
        });
    }
    group.finish();
}

fn bench_extract_labels(c: &mut Criterion) {
    let mut group = c.benchmark_group("extract_labels");
    for (name, text) in fixtures() {
        group.throughput(Throughput::Bytes(text.len() as u64));
        group.bench_with_input(BenchmarkId::from_parameter(name), &text, |b, &text| {
            b.iter(|| extract_labels(black_box(text)));
        });
    }
    group.finish();
}

// Synthetic large-vault simulation: tile MEDIUM 200x to approximate a single
// note of ~2MB. Catches algorithmic regressions that small inputs hide.
fn bench_large_synthetic(c: &mut Criterion) {
    let mut group = c.benchmark_group("get_chunks_and_links_large");
    let large = MEDIUM.repeat(200);
    let path = VaultPath::note_path_from("/bench.md");
    group.throughput(Throughput::Bytes(large.len() as u64));
    group.bench_function("medium_x200", |b| {
        b.iter(|| NoteDetails::chunks_and_links_of(black_box(&path), black_box(large.as_str())));
    });
    group.finish();
}

criterion_group!(
    benches,
    bench_get_chunks_and_links,
    bench_get_content_chunks,
    bench_label_matches,
    bench_extract_labels,
    bench_large_synthetic,
);
criterion_main!(benches);
