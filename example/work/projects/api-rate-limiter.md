# API Rate Limiter

## Overview

Token bucket rate limiter for the public API. Limits requests per API key.

## Configuration

| Parameter | Default | Description |
|-----------|---------|-------------|
| `rate_limit` | 100 | Requests per window |
| `window_secs` | 60 | Window duration in seconds |
| `burst_limit` | 20 | Max burst above rate |

## Implementation

- Algorithm: Token bucket
- Storage: Redis (shared across API instances)
- Headers: `X-RateLimit-Limit`, `X-RateLimit-Remaining`, `X-RateLimit-Reset`
- Response on limit: 429 Too Many Requests

## Related

- Reviewed by [[maria]] and [[carlos]]
- Part of the Q2 API hardening initiative
- See [[api-docs]] for the public documentation
