//! A markdown note exercising every element the Edit surface paints, for the
//! headless rendering tests (and the M3 parity audit).

/// Every markdown construct the painter has a rule for, one per line where
/// possible so tests can address it by row.
pub const RICH: &str = "\
# Heading one
## Heading two
### Heading three
#### Heading four
##### Heading five
###### Heading six
Plain **bold** and *italic* and ~~gone~~ and ***both*** end
Nested **bold *inner* bold** and `inline code` here
```
fn main() {}
let wide = 1;
```
> quoted line
>> nested quote
- bullet item
1. ordered item
- [ ] open task
- [x] done task
A [link](https://example.com) and [[Wiki Note]] and #label here
An image ![alt](pic.png) inline
---
Wide 理理 emoji 😀 combining e\u{301} end
This is a long paragraph that keeps going well past the width of a narrow pane so that it wraps onto several visual rows and the continuation rows show how wrapping lands.";
