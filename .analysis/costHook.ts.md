# Analysis: costHook.ts

**Summary**: React hook that outputs cost summary to stdout on process exit. Minimal wrapper around cost-tracker functions.

## Dependencies

- `react` → useEffect hook (standard React patterns)
- `./cost-tracker.js` → formatTotalCost, saveCurrentSessionCosts
- `./utils/billing.js` → hasConsoleBillingAccess gate (feature flag)

## Structure

### Function

**useCostSummary(getFpsMetrics?)**:
- Registers exit listener via `process.on('exit')`
- Checks billing access gate (hasConsoleBillingAccess())
- Writes formatted cost to stdout if access granted
- Saves session costs to project config
- Cleanup on component unmount

## Issues（Rust Porting Concerns）

- [ ] ISSUE [MEDIUM]: Direct `process.stdout.write()` inside exit handler is unsafe. If process is already exiting, I/O may be buffered/lost or cause deadlock.
  Impact: Cost summary may not display on exit in all scenarios (abrupt termination, signal handlers, etc.).
  Suggestion: Use `process.stderr` as fallback; write to temp file instead if reliability critical.

- [ ] ISSUE [MEDIUM]: `process.on('exit')` callback with async operations (saveCurrentSessionCosts may trigger I/O). The exit event does NOT allow async cleanup in Node.js.
  Impact: Config save may not complete if process exits too quickly.
  Suggestion: Use `process.beforeExit` (allows async) or pre-save on-demand rather than at exit.

- [ ] ISSUE [LOW]: React hook cleanup (`return () => process.off('exit', f)`) is correct but assumes hook is mounted exactly once. If multiple instances exist, cleanup becomes complex.
  Impact: Memory leak if useCostSummary called multiple times.
  Suggestion: Use singleton pattern or context provider at app root.

- [ ] ISSUE [LOW]: `getFpsMetrics?.()` is optional but saveCurrentSessionCosts always accepts it. If undefined, FPS metrics silently omitted from config.
  Impact: FPS data loss if hook doesn't receive getFpsMetrics provider.
  Suggestion: Add warning if getFpsMetrics not provided and FPS tracking is expected.

## Optimizations

- [ ] OPT [SAFETY]: Wrap exit handler in try-catch to prevent unhandled exceptions in exit path.
  Why better: Ensures cleanup always runs.
  Approach:
  ```ts
  const f = () => {
    try { ... } catch (e) { console.error(e); }
  }
  ```

- [ ] OPT [ERGONOMICS]: Extract exit handler setup into a separate useExitHandler hook.
  Why better: Reusable, composable pattern.
  Approach: Create generic useExitHandler(callback) hook; reuse in other modules.

- [ ] OPT [IDIOM]: React hook pattern is correct but consider moving process event logic into a context provider or useEffect in a wrapper component.
  Why better: Centralizes exit behavior.
  Approach: Create CostTrackerProvider component that owns the exit handler.

## Sage Porting

In Rust (non-React), this becomes a shutdown handler in the main agent loop:

```rust
// Pseudo-code for Sage runtime
pub fn on_process_exit(state: &CostState) {
  if has_console_billing_access() {
    eprintln!("\n{}", format_total_cost(state));
  }
  save_session_costs(state);
}

// Register in main():
let _guard = OnExitGuard::new(|| on_process_exit(&state));
```

Or use `Arc<Mutex<State>>` + signal handlers for cleanup.
