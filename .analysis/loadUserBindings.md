# Analysis: keybindings/loadUserBindings.ts

Summary: User keybinding loader with hot-reload support via file watching. Manages global cached state, handles both sync and async loading, integrates with feature gates and telemetry.

## External Dependencies

- **chokidar** (line 12) — File system watcher for monitoring ~/.claude/keybindings.json changes
  - Rust equivalent: `notify` crate
  - Risk: API mismatch. Chokidar's `awaitWriteFinish` config (line 389-391) has no direct equivalent in `notify`. Must implement file stability check manually.

- **fs/promises** (line 14) — Async file I/O
  - Rust equivalent: `tokio::fs` for async, `std::fs` for sync

- **createSignal** (line 22) — Custom signal/event emitter from utils
  - Rust equivalent: `tokio::sync::broadcast` or custom event channel
  - Risk: Signal semantics must match (subscription model, emission, cleanup)

- **GrowthBook feature gate** (line 16) — `getFeatureValue_CACHED_MAY_BE_STALE()`
  - Rust equivalent: Sage feature system
  - Risk: "CACHED_MAY_BE_STALE" suggests stale feature values; Sage must provide equivalent

## Issues

- [ ] ISSUE [BLOCKER]: File watching implementation incompatible
  Impact: Chokidar's `awaitWriteFinish: { stabilityThreshold, pollInterval }` (line 389-391) ensures file writes complete before firing. `notify` crate doesn't have this.
  Suggestion: Implement own file stability debounce: watch raw events, delay emission by stabilityThreshold, reset on each event. Use `tokio::time::sleep` for debounce.

- [ ] ISSUE [BLOCKER]: Global mutable state requires synchronization
  Impact: Lines 66-70: `watcher`, `initialized`, `disposed`, `cachedBindings`, `cachedWarnings` are global mutable. No Mutex/RwLock protection.
  Suggestion: Wrap in `Mutex<LoaderState>` or `RwLock<CachedBindings>` for thread-safe access

- [ ] ISSUE [HIGH]: Sync and async code paths duplicate logic
  Impact: Lines 133-237 (async loadKeybindings) vs 259-345 (sync loadKeybindingsSyncWithWarnings) — 80% duplicate code for JSON parsing, validation, error handling
  Suggestion: Extract common parsing logic to shared function; async calls it, sync has direct IO fallback

- [ ] ISSUE [HIGH]: Feature gate check bypasses user customization entirely
  Impact: Line 137, 267: If tengu_keybinding_customization_release is false, user config is ignored. No partial support.
  Suggestion: Design allows feature gate; OK for MVP, but document for future granularity

- [ ] ISSUE [MEDIUM]: Daily telemetry logging uses naive date comparison
  Impact: Line 84: `lastCustomBindingsLogDate === today` string comparison. Works but fragile if timezone changes or date format changes.
  Suggestion: Use timestamp-based check or proper date library

- [ ] ISSUE [MEDIUM]: Error handling swallows context
  Impact: Line 339-344 in sync version: catch-all with no error logging in sync path. Errors silently fall back to defaults.
  Suggestion: Log errors for debugging; provide mechanism to surface loader failures

## Optimizations

- [ ] OPT [IDIOM]: Use lazy_static or once_cell for cached parsed default bindings
  Why better: Default bindings are constant; parsing them repeatedly is wasteful
  Approach: `lazy_static! { static ref DEFAULT_PARSED = parseBindings(DEFAULT_BINDINGS); }`

- [ ] OPT [SAFETY]: Provide mutable initialization guard, not disposable flag
  Why better: Current `initialized && disposed` flags are easy to misstate
  Approach: Use Rust's type system: `InitializedWatcher` newtype, prevent re-init via type

- [ ] OPT [ERGONOMICS]: Combine loadKeybindings() and loadKeybindingsSyncWithWarnings()
  Why better: Two nearly-identical functions are confusing
  Approach: Single async function `loadKeybindings()` + wrapper sync load from blocking thread if needed

- [ ] OPT [PERF]: Cache file path computation
  Why better: getKeybindingsPath() called multiple times
  Approach: Compute once at module init
