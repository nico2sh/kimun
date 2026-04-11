# API Documentation

Public API reference for external developers.

## Authentication

### API Keys
- Generated in the developer dashboard
- Include in `Authorization: Bearer <key>` header
- Rate limited per key — see [[api-rate-limiter]]

### OAuth2 (coming soon)
- Authorization code flow for server-side apps
- PKCE flow for SPAs and mobile apps
- See [[2026-04-08-auth-flow]] for design decisions
- [[maria]] is leading the implementation

## Endpoints

### Search
```
GET /api/v1/search?q={query}&workspace_id={id}
```
- Results cached for 5 minutes — see [[search-caching]]
- Supports full-text search with relevance ranking
- Returns paginated results (default 20, max 100)

### Notes CRUD
```
POST   /api/v1/notes
GET    /api/v1/notes/{id}
PUT    /api/v1/notes/{id}
DELETE /api/v1/notes/{id}
```

### Workspaces
```
GET    /api/v1/workspaces
POST   /api/v1/workspaces
```

## Rate Limits

See [[api-rate-limiter]] for details.

| Tier | Requests/min | Burst |
|------|-------------|-------|
| Free | 60 | 10 |
| Pro | 300 | 50 |
| Enterprise | 1000 | 200 |
