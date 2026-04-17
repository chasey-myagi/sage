# Wiki Schema

This agent's knowledge base follows the Sage wiki convention. See
https://github.com/chasey-myagi/sage-wiki for the full specification.

## Page types

- `pitfall` — things that failed, gotchas, undocumented limits
- `pattern` — successful workflows worth repeating
- `api-ref` — API endpoint behaviors confirmed from real sessions
- `decision` — architectural/design decisions with rationale
- `concept` — domain terminology
- `synthesis` — cross-topic analysis

## Frontmatter (required for every wiki/pages/*.md)

```yaml
---
slug: <kebab-case-name>
page_type: pitfall | pattern | api-ref | decision | concept | synthesis
confidence: low | medium | high
session_count: <integer>
last_updated: YYYY-MM-DD
sources: [session-id-1, session-id-2, ...]
---
```

## Confidence model

- `low` — 1 session, inferred
- `medium` — 1–2 sessions, directly observed
- `high` — 3+ sessions, consistent
