# Sprint Planning — 2026-04-10

**Attendees:** Me, [[maria]], [[carlos]], [[david]]

## Sprint Goals

1. Ship search caching to production (feature flag rollout)
2. Begin OAuth2 PKCE implementation
3. Search index tokenizer improvements

## Task Assignments

### Me
- [ ] Cache feature flag: shadow mode → 10% → full rollout
- [ ] Prepare all-hands demo slides
- [ ] Write [[observability]] proposal RFC

### [[maria]]
- [ ] OAuth2 PKCE: endpoint stubs and basic flow
- [ ] Draft auth endpoint specs for [[api-docs]]

### [[carlos]]
- [ ] Production Redis cluster sizing for caching
- [ ] Self-serve Redis provisioning tool (carry-over from last sprint)
- [ ] Jaeger cluster provisioning for [[observability]]

### [[david]]
- [ ] Tokenizer config changes from PR feedback
- [ ] Search ranking A/B test setup
- [ ] Benchmark full-text search with new tokenizer

## Capacity Notes

- [[carlos]] has on-call duty next week — reduce planned work
- New team member starting May 1 — plan onboarding stories for next sprint

## Risks

- Redis cluster sizing depends on production traffic patterns — may need to iterate
- OAuth2 timeline is aggressive — [[maria]] flagged potential scope creep with token revocation
