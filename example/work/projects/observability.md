# Observability Improvements Proposal

## Problem

Current observability stack has gaps:
- No distributed tracing (hard to debug cross-service latency)
- Metrics are service-level only (no request-level breakdown)
- Log correlation requires manual grep across services

## Proposed Solution

### Phase 1: Distributed Tracing
- Adopt OpenTelemetry for instrumentation
- Deploy Jaeger as the trace backend
- Instrument the API gateway, search service, and auth service first
- Goal: trace a request from gateway through cache ([[search-caching]]) to database and back

### Phase 2: Enhanced Metrics
- Add request-level latency histograms (p50, p95, p99)
- Cache hit/miss ratios per workspace
- Auth token validation latency
- Expose via Prometheus `/metrics` endpoint

### Phase 3: Unified Dashboard
- Grafana dashboards linking traces, metrics, and logs
- Alert on p99 latency > 500ms
- Alert on cache hit rate < 50%

## Resources Needed

- [[carlos]]: Jaeger deployment and infrastructure
- [[david]]: Search service instrumentation
- [[maria]]: Auth service instrumentation
- Me: API gateway and coordination

## Timeline

- Phase 1: 3 weeks
- Phase 2: 2 weeks
- Phase 3: 1 week

## Related

- Discussed in 1:1 with manager — see [[manager-notes]]
- Connects to [[search-caching]] performance monitoring
