# Snippets

Random code spans mixed with prose. Lots of `#define` and `#[derive]` to stress the exclusion-zone logic.

Inline: use `#[derive(Debug)]` on structs and `#include <stdio.h>` in C headers. Also `# header` is markdown but `#tag` inside `\`` becomes a `#code-span`.

```rust
#[derive(Debug, Clone)]
struct Event {
    id: u64,
    payload: Vec<u8>,
    // #internal marker — not a real tag
}

impl Event {
    fn new() -> Self {
        // #constructor
        Self { id: 0, payload: vec![] }
    }
}
```

Some prose between blocks. The above struct is used by the #ingestion layer.

```python
# This is a Python comment, not a hashtag
def process(event):
    # #another comment
    return event["payload"]

class Handler:
    """Handles #events from the bus."""
    pass
```

```c
#define MAX_BUFFER 4096
#define MIN_BUFFER 128

#include <stdio.h>
#include <stdlib.h>

int main(int argc, char **argv) {
    // #main entry point
    printf("Hello, #world\n");
    return 0;
}
```

Back to prose. Real tags: #real-tag and #another.

```sql
-- #sql comment
SELECT * FROM events WHERE tag = '#urgent';
SELECT count(*) FROM notes WHERE body LIKE '%#hashtag%';
```

```yaml
# YAML comment
tags:
  - "#urgent"
  - "#review"
  - "##draft"  # this should not be a label
```

Real tag at end: #wrap-up
