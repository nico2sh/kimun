# Project notes — realistic mixed markdown

This is a representative kimun note with the usual mix of constructs you
would actually edit in production. Headings, paragraphs, lists, code,
quotes, links, emphasis.

## Background

Brown fox jumps the dog. *italic* and **bold** and `inline code` plus a
[[WikiLink]] sit inline.

Reference link: [Rust book](https://doc.rust-lang.org/book/).

## Goals

- ship the v2 hybrid widener
- collect metrics across realistic notes
- decide between Option A vs B vs status quo

## Findings so far

1. Fast-path share hit 100% on the dogfooding session.
2. `full_lazy_depth` dominated the rejects at ~40%.
3. Widen_to_safe never fired.

## Code snippet

```rust
fn try_incremental_parse(&self, lines: &[String]) -> Option<Splice> {
    if self.parsed_buffer.lines.is_empty() {
        return None;
    }
    // …
}
```

> Important: the lazy_depth guard runs BEFORE either widener. Edits
> inside or adjacent to a lazy construct fall back to full parse.
> Tightening reset_boundaries to encode "adjacent to lazy" would
> subsume the guard.

## Loose list with mixed content

- alpha
- beta with `code` mid-item

  paragraph continuation inside item beta

- gamma

  - nested item under gamma
  - another nested item

## Indented code as multi-chunk

    fn hello() {
        println!("hi");
    }

    fn another() {
        println!("there");
    }

End of note.