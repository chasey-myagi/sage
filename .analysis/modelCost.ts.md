# Analysis: utils/modelCost.ts

**Summary**: Model pricing registry with cost-per-million-tokens constants and cost calculation logic. Core function for USD conversion.

## Dependencies

- `@anthropic-ai/sdk/resources/beta/messages/messages.mjs` → BetaUsage type (import only)
- `./model/configs.js` → Model configuration constants
- `./model/model.js` → Model name canonicalization functions
- Analytics service for logging unknown models

## Structure

### Types

- **ModelCosts** — Record type: `{ inputTokens, outputTokens, promptCacheWriteTokens, promptCacheReadTokens, webSearchRequests }` (all numbers = $/Mtok)
- Pricing tiers: `COST_TIER_3_15`, `COST_TIER_15_75`, `COST_TIER_5_25`, `COST_TIER_30_150`, `COST_HAIKU_35`, `COST_HAIKU_45`
- `MODEL_COSTS` record mapping canonical model names → ModelCosts

### Key Functions

1. **tokensToUSDCost(modelCosts, usage)** — Calculates total cost:
   - `(usage.input_tokens / 1_000_000) * costs.inputTokens`
   - `(usage.output_tokens / 1_000_000) * costs.outputTokens`
   - `(usage.cache_read_input_tokens / 1_000_000) * costs.promptCacheReadTokens`
   - `(usage.cache_creation_input_tokens / 1_000_000) * costs.promptCacheWriteTokens`
   - `usage.server_tool_use?.web_search_requests * costs.webSearchRequests`

2. **getModelCosts(model, usage)** — Returns ModelCosts for a model, with Opus 4.6 fast-mode override logic
3. **calculateUSDCost(resolvedModel, usage)** — Public API wrapper
4. **calculateCostFromTokens(model, tokens)** — Helper for side queries (classifier, etc.)
5. **formatPrice(price)** → "$3" / "$0.80" formatting
6. **formatModelPricing(costs)** → "$3/$15 per Mtok" display

## Issues（Rust Porting Concerns）

- [ ] ISSUE [HIGH]: Web search requests not tracked in Sage `Cost` or `Usage` structs (only in CC's server_tool_use).
  Impact: If Sage needs to bill web search separately, implementation will miss costs.
  Suggestion: Add `web_search_requests: u64` to Sage Usage struct, add `web_search: f64` to Cost struct.

- [ ] ISSUE [MEDIUM]: `BetaUsage` type assumptions — Sage must match Anthropic SDK's field names exactly (input_tokens, cache_read_input_tokens, cache_creation_input_tokens, server_tool_use.web_search_requests).
  Impact: If SDK version changes or fields rename, cost calculation breaks silently.
  Suggestion: Ensure SDK pinned + tests validate all field names.

- [ ] ISSUE [MEDIUM]: Fast-mode Opus 4.6 override tied to `usage.speed === 'fast'` property — must verify this field exists in Sage's Usage analog.
  Impact: Fast-mode costs may not be detected correctly.
  Suggestion: Check Anthropic SDK usage type documentation + add test for fast-mode cost lookup.

- [ ] ISSUE [LOW]: Floating-point arithmetic in cost calculation — no guard against overflow/underflow for extremely large token counts.
  Impact: Cost estimates are small enough (typically <$100 per run) that this is negligible, but a 1B-token run could theoretically overflow.
  Suggestion: Use same approach as CC (simple f64 multiplication); add assertion in tests for reasonable bounds.

## Optimizations

- [ ] OPT [IDIOM]: ModelCosts record → Rust struct with f64 fields + derived PartialEq/Clone/Debug.
  Why better: Type safety + zero cost abstraction vs TS record.
  Approach: `#[derive(Debug, Clone, PartialEq)] pub struct ModelCost { pub input_per_million: f64, ... }` (Sage already has this).

- [ ] OPT [PERF]: `MODEL_COSTS` hashmap lookup O(1) — already optimal in TS. In Rust use `phf` crate for compile-time perfect hashing if registry is large.
  Why better: Eliminates runtime hash computation (marginal).
  Approach: Use static PHF map instead of lazy_static HashMap.

- [ ] OPT [SAFETY]: Pricing tier mutations — add unit tests to verify constant tiers match official pricing (Anthropic docs).
  Why better: Prevents stale pricing from silently degrading accuracy.
  Approach: Add doctest with expected tier values; link to https://platform.claude.com/docs/en/about-claude/pricing.

- [ ] OPT [ERGONOMICS]: Cache-write + cache-read unit costs have a fixed ratio (3.75x and 0.3x for Sonnet). Consider deriving one from the other.
  Why better: Reduces manual error in pricing tier definition.
  Approach: Document the relationship; consider constants that derive from inputTokens (e.g., `cache_write_cost = input_cost * 1.25`).

## Sage Type Mapping

**CC ModelCosts field** → **Sage ModelCost**:
- `inputTokens` → `input_per_million`
- `outputTokens` → `output_per_million`
- `promptCacheReadTokens` → `cache_read_per_million`
- `promptCacheWriteTokens` → `cache_write_per_million`
- `webSearchRequests` → *(missing in Sage, requires addition)*

**CC Usage fields used** → **Sage Usage**:
- `input_tokens` → `input` (u64)
- `output_tokens` → `output` (u64)
- `cache_read_input_tokens` → `cache_read` (u64)
- `cache_creation_input_tokens` → `cache_write` (u64)
- `server_tool_use?.web_search_requests` → *(missing in Sage)*
