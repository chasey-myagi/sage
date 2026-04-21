# Analysis: cost-tracker.ts

**Summary**: Session-level cost state management, persistence (save/restore), and cost aggregation by model. Handles formatting and analytics logging.

## Dependencies

- `@anthropic-ai/sdk` → BetaUsage (usage tracking)
- `chalk` → terminal color output (non-critical, dev-only)
- `./bootstrap/state.js` → global cost state accessors (addToTotalCostState, getCostCounter, getTokenCounter, etc.)
- `./utils/modelCost.js` → calculateUSDCost, core pricing logic
- `./utils/config.js` → project config persistence
- Analytics service → logEvent for instrumentation
- Model utilities → getCanonicalName for aggregation

## Structure

### Types

- **StoredCostState** — TS interface for persisted session cost:
  ```
  {
    totalCostUSD: number
    totalAPIDuration: number
    totalAPIDurationWithoutRetries: number
    totalToolDuration: number
    totalLinesAdded: number
    totalLinesRemoved: number
    lastDuration: number | undefined
    modelUsage: { [modelName: string]: ModelUsage } | undefined
  }
  ```

- **ModelUsage** — Nested in state: `{ inputTokens, outputTokens, cacheReadInputTokens, cacheCreationInputTokens, webSearchRequests, costUSD, contextWindow, maxOutputTokens }`

### Key Functions

1. **getStoredSessionCosts(sessionId)** — Loads costs from project config if session ID matches (prevents cross-session data leaks).
2. **restoreCostStateForSession(sessionId)** — Wrapper to restore + hydrate with context windows.
3. **saveCurrentSessionCosts(fpsMetrics?)** — Persists accumulated costs back to config.
4. **formatCost(cost, maxDecimalPlaces)** → Formatting helper: ≥$0.5 uses 2 decimals, <$0.5 uses 4 decimals.
5. **formatModelUsage()** → Multi-line string aggregating usage by canonical model name (Haiku, Sonnet, Opus).
6. **formatTotalCost()** → Full cost summary with duration, lines changed, model breakdown.
7. **addToTotalSessionCost(cost, usage, model)** → Main entry point: updates state + metrics + analytics; **recursive for advisor usage**.
8. **addToTotalModelUsage(cost, usage, model)** → Accumulates usage into per-model record.

## Issues（Rust Porting Concerns）

- [ ] ISSUE [BLOCKER]: `bootstrap/state.js` is a global mutable state module (side effects). In Rust, this requires `Mutex<>` / `Arc<Mutex<>>` or thread-local storage.
  Impact: Rust won't allow unsynchronized mutation; code must be refactored to pass state explicitly or use Arc + Mutex.
  Suggestion: Move from global to dependency injection (pass &mut state or Arc<Mutex<State>> to functions). Check Sage agent-core's state management pattern.

- [ ] ISSUE [HIGH]: Project config save/restore pattern (`getCurrentProjectConfig` + `saveCurrentProjectConfig`). In Rust, must ensure atomic writes and no data loss.
  Impact: Partial config writes could corrupt stored costs.
  Suggestion: Use atomic file operations (write to temp, rename) or SQLite transaction semantics.

- [ ] ISSUE [HIGH]: Recursive `addToTotalSessionCost` for advisor usage (line 316-320). If advisor chains are deep, can exceed stack limits.
  Impact: May panic in rare cases with many nested advisors.
  Suggestion: Convert recursion to loop with accumulator; add max-depth guard + warning.

- [ ] ISSUE [HIGH]: `modelUsage[shortName]` accumulation assumes all models of same short name are equivalent cost-wise. But if Opus 4.6 fast-mode and normal coexist, they have different costs but same short name.
  Impact: Cost aggregation by short name loses fast-mode cost distinction.
  Suggestion: Include speed/mode in aggregation key or track separately.

- [ ] ISSUE [MEDIUM]: Float arithmetic precision — repeated addition in loops (line 204-209) risks floating-point rounding errors over many sessions.
  Impact: Negligible for typical session costs (<$1000 per session) but can accumulate over long projects.
  Suggestion: Accumulate as integer cents (multiply by 100) if precision critical; or use decimal crate.

- [ ] ISSUE [MEDIUM]: Missing null-check for `projectConfig.lastModelUsage` — if undefined, line 100 `.map()` would fail silently.
  Impact: Model usage not restored if config is missing the field.
  Suggestion: Add explicit null check + warn; initialize to empty object.

- [ ] ISSUE [LOW]: Web search requests aggregation (line 220) uses `formatNumber()` but web search field comes from `usage.webSearchRequests` which may be undefined in some usage types.
  Impact: Formatting may fail if field is missing.
  Suggestion: Use `?? 0` pattern consistently.

## Optimizations

- [ ] OPT [ERGONOMICS]: Separate concerns: cost calculation, aggregation, formatting, persistence. Move formatting into a dedicated module.
  Why better: Easier to swap formatters (JSON output, CSV, database insert).
  Approach: Extract formatCost, formatModelUsage, formatTotalCost into cost-formatter.ts.

- [ ] OPT [PERF]: `formatModelUsage()` rebuilds aggregation every call. If called frequently, cache the aggregated map.
  Why better: Avoids O(n) iterations for display.
  Approach: Memoize aggregation result; invalidate on addToTotalModelUsage().

- [ ] OPT [IDIOM]: Use `Object.fromEntries()` + `.map()` pattern (line 100-109, 160-171) is idiomatic TS but verbose. In Rust, use struct initialization.
  Why better: Cleaner, type-safe.
  Approach: Define ModelUsage struct in Rust with struct literals.

- [ ] OPT [SAFETY]: Project config persistence has no version field. If schema changes, old configs break silently.
  Why better: Prevents silent data loss.
  Approach: Add schema version + migration function.

## Sage Type Mapping

**StoredCostState** → Rust struct (new, for persistence):
```rust
pub struct PersistedCostState {
  pub total_cost_usd: f64,
  pub total_api_duration_ms: u64,
  pub total_api_duration_without_retries_ms: u64,
  pub total_tool_duration_ms: u64,
  pub total_lines_added: u64,
  pub total_lines_removed: u64,
  pub last_duration_ms: Option<u64>,
  pub model_usage: HashMap<String, PersistedModelUsage>,
}

pub struct PersistedModelUsage {
  pub input_tokens: u64,
  pub output_tokens: u64,
  pub cache_read_tokens: u64,
  pub cache_write_tokens: u64,
  pub web_search_requests: u64,  // add if needed
  pub cost_usd: f64,
}
```

**addToTotalSessionCost logic** → Sage cost tracking function:
```rust
pub fn add_to_total_session_cost(
  state: &mut CostState,
  cost_usd: f64,
  usage: &Usage,
  model: &str,
) -> f64 {
  // Update model usage
  // Log to analytics
  // Return total (including advisor costs)
}
```
