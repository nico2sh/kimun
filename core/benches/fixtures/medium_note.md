---
title: Architecture notes
created: 2026-04-20
tags: [architecture, draft]
---

# Architecture overview

The system is split into three layers: ingestion, processing, and presentation. Each layer is independently deployable but they share a common configuration surface.

## Ingestion

The ingestion layer pulls from three sources: the upstream event bus (#eventbus), the periodic batch job (#batch), and the manual operator console. Each source feeds into a normalization stage that converts heterogeneous payloads into the canonical [[schemas/event]] shape.

Key decisions:

- Use `serde_json` for deserialization rather than a custom parser — see [[decisions/serde-vs-custom]].
- Buffer up to 10,000 events in memory before flushing to disk. The buffer is implemented as a ring with `crossbeam-channel` backing it.
- On crash recovery, replay from the last checkpoint. Checkpoints are written every 30 seconds.

```rust
fn ingest(event: RawEvent) -> Result<NormalizedEvent, IngestError> {
    let parsed = serde_json::from_slice(&event.payload)?;
    let normalized = normalize(parsed)?;
    Ok(normalized)
}
```

## Processing

Processing runs in two modes: streaming (for the realtime path) and batch (for backfills and recomputation). Both modes share the same transformation library at [[lib/transform]].

The #streaming path uses a Kafka-style log abstraction. We considered using actual Kafka — see [[decisions/kafka-vs-internal]] — and rejected it for ops complexity. The internal log is backed by SQLite with a custom FTS5 index.

Performance targets:

- p50 latency: 50ms
- p99 latency: 500ms
- Throughput: 10k events/sec sustained

Current measurements (#perf 2026-04):

- p50: 38ms ✓
- p99: 612ms ✗ (regression from the last release — investigate)
- Throughput: 12.4k events/sec ✓

### Backfill mode

Backfill walks a date range and re-emits events into the processing pipeline. It uses the same transformation code but bypasses the streaming buffer. See `processing/backfill.rs` for the entry point.

The backfill job is idempotent: re-running over an already-processed range produces the same output (modulo timestamps). This is important for the #disaster-recovery story — we can rebuild any window from raw events.

## Presentation

The presentation layer serves the web UI ([[ui/web]]), the CLI ([[cli/kimun]]), and the Slack bot ([[integrations/slack]]). All three consume the same GraphQL API at `api.internal/graphql`.

Authentication uses short-lived JWTs signed by the auth service. Refresh tokens live in `httpOnly` cookies. See [[security/auth]] for the threat model.

## Open questions

- How do we handle out-of-order events from #eventbus when the upstream clock skews? Current proposal: bucket by ingestion time, not event time, for the realtime path. Discuss with @nico in next sync.
- Should we expose the raw event log to power users? Privacy review pending — see [[legal/raw-log-exposure]].
- The #batch job has been flaky on Mondays. Theory: it conflicts with the weekly compaction. Need to confirm.

## References

- [[architecture/overview-2025]]
- [[decisions/serde-vs-custom]]
- [[decisions/kafka-vs-internal]]
- Internal RFC: https://internal.example.com/rfc/0042
- Public docs: https://docs.example.com/architecture
