# Search Caching Design

## Problem

Search queries hit the database directly, resulting in 120ms+ latency. Repeat queries for the same terms are common.

## Solution

Redis-backed cache layer with pub/sub invalidation.

### Cache Key Format

```
search:{workspace_id}:{sha256(normalized_query)}
```

### TTL

5 minutes. Conservative to start — can increase based on hit rate data.

### Invalidation

On note create/update/delete, publish to Redis channel `cache:invalidate:{workspace_id}`. All API instances subscribe and flush workspace-specific cache entries.

## Results (Staging)

- Cache hit latency: **8ms** (93% reduction)
- Cache miss latency: **125ms** (slight overhead from Redis check)
- Hit rate after 1 hour: **72%**
- Memory usage: ~50MB for 10K cached queries

## Architecture

```
Client -> API Gateway -> Cache Check (Redis)
                              |
                         Hit? -> Return cached
                         Miss? -> Query DB -> Store in Redis -> Return
                              
Note mutation -> Pub/Sub -> All API instances flush cache
```

## Rollout Plan

1. Deploy behind feature flag
2. Shadow mode (cache but still query DB, compare results)
3. Enable for 10% of traffic
4. Full rollout

## Related

- [[api-rate-limiter]] (same Redis cluster)
- Reviewed by [[carlos]] (Redis infra) and [[david]] (search)
