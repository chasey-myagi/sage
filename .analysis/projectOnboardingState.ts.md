# Analysis: projectOnboardingState.ts

Summary: State management module for project onboarding progress. Defines `Step` type, provides functions to query/update onboarding state via config persistence, with memoization for visibility logic.

## External Dependencies

- `lodash-es/memoize.js` — Caches function result across calls. Rust equivalent: `once_cell::sync::Lazy<bool>` or custom memoization via `Arc<Mutex<Option<T>>>` if state is mutable.
- `path.join` — Standard library (`path::Path::join` in Rust).
- Internal utilities: `config.js`, `cwd.js`, `file.js`, `fsOperations.js` — all will have Rust equivalents in the ported codebase.

## Issues

- [ ] ISSUE [HIGH]: `memoize()` caches function result indefinitely. The memoized `shouldShowProjectOnboarding()` returns a single bool for the entire session, but the underlying state (`getCurrentProjectConfig()`, filesystem checks) can change. If config or CLAUDE.md is created during a session, the function won't re-evaluate.
  Impact: UI shows stale onboarding state after CLAUDE.md is created; user sees onboarding prompt when they shouldn't.
  Suggestion: Either (a) don't memoize (re-evaluate on every call), (b) invalidate cache when config changes, or (c) use a reactive state system that notifies observers when the result changes.

- [ ] ISSUE [HIGH]: `Step.isCompletable` and `Step.isEnabled` are computed once in `getSteps()` and never recomputed. If the workspace state changes mid-session (e.g., a file is created), these flags won't update.
  Impact: Step completability doesn't reflect actual state; UI shows incorrect progress.
  Suggestion: Either recompute steps on demand or add a change listener to trigger recomputation when filesystem events occur.

- [ ] ISSUE [MEDIUM]: `incrementProjectOnboardingSeenCount()` and `maybeMarkProjectOnboardingComplete()` both call `saveCurrentProjectConfig()`, which may involve I/O (file writes or API calls). These are called synchronously without error handling.
  Impact: Silent failures if config save fails (e.g., permission denied); state becomes inconsistent.
  Suggestion: Add explicit error handling. In Rust, return `Result<(), ConfigError>` so callers know to handle failures.

- [ ] ISSUE [MEDIUM]: `isProjectOnboardingComplete()` filters steps by `isCompletable && isEnabled`, then checks if all have `isComplete`. If a step becomes disabled mid-session (e.g., the workspace is no longer empty), the completion check doesn't account for it — it may still require a disabled step to be complete.
  Impact: Onboarding state becomes inconsistent if workspace state changes.
  Suggestion: Clarify intent: should disabled steps count toward completion? Add explicit comment or refactor to `steps.filter(...).length === steps.filter(...).filter(s => s.isComplete).length`.

- [ ] ISSUE [MEDIUM]: `getFsImplementation()` is called directly from `getSteps()` without type safety. If the FS implementation doesn't have `existsSync()`, the code breaks at runtime.
  Impact: Relies on runtime duck typing; Rust would require a trait with methods.
  Suggestion: In Rust, define `trait FileSystemOps { fn exists_sync(&self, path: &Path) -> bool; }` and ensure all implementations provide this method.

## Optimizations

- [ ] OPT [SAFETY]: `Step` type has no validation that `key` is unique or follows a naming convention. When multiple steps with the same key are added, the system won't detect duplicates.
  Why better: Compile-time guarantees that keys are distinct.
  Approach: Use an enum instead of string keys: `enum StepKey { Workspace, ClaudeMd }`, or use a const array of allowed keys with static assertions.

- [ ] OPT [SAFETY]: `getCurrentProjectConfig().projectOnboardingSeenCount` is assumed to be an integer. If config is corrupted or migrated, it could be a string or null. Rust's type system would catch this.
  Why better: No defensive null/type checks needed; the type system ensures it's always an int.
  Approach: In Rust, use a proper struct with typed fields: `struct ProjectConfig { project_onboarding_seen_count: u32 }`.

- [ ] OPT [IDIOM]: Memoization pattern is imperative (lodash function wrapping). In Rust with static lifetime, a cleaner approach is `once_cell::sync::Lazy<bool>` or `std::sync::OnceLock<bool>` (1.70+).
  Why better: More idiomatic for Rust; clearer intent that value is computed once.
  Approach: Replace `const shouldShowProjectOnboarding = memoize((): boolean => { ... })` with `thread_local! { static SHOW_ONBOARDING: OnceLock<bool> = OnceLock::new(); }`.

- [ ] OPT [PERF]: `isDirEmpty()` is called in `getSteps()` on every invocation. If the workspace is large, this I/O could be slow. Combined with the memoization issue (HIGH), the function never recomputes, so the I/O cost is paid once but the result may be stale.
  Why better: Cache the result with invalidation on filesystem events.
  Approach: Listen to filesystem watchers (via `chokidar` or OS FSEvents) and invalidate when the workspace directory changes.

- [ ] OPT [ERGONOMICS]: `Step` type has many boolean flags (`isComplete`, `isCompletable`, `isEnabled`). A discriminated union or state enum would be clearer.
  Why better: Reduces boolean explosion; makes state transitions explicit.
  Approach: Define `enum StepState { NotStarted, InProgress, Complete, Blocked }` and remove redundant flags.
