# Unified Plan: CC init Port to Rust

## Overview

Port two TypeScript files to Rust:
1. `commands/init.ts` — `/init` command with 8-phase CLAUDE.md setup workflow
2. `projectOnboardingState.ts` — Onboarding state tracking and completion logic

## Critical Issues (Block Port Without Fix)

### 1. Memoization Staleness (projectOnboardingState.ts:63-76)

**Source Issue**: `shouldShowProjectOnboarding = memoize((): boolean => { ... })`

**Problem**: Result cached indefinitely; doesn't invalidate when config or CLAUDE.md existence changes. UI shows stale onboarding state.

**Rust Implementation Options**:
- **Option A (Simplest)**: Don't memoize — just call `should_show_project_onboarding()` on demand. Performance cost is negligible unless called 100s of times per frame.
- **Option B (Recommended)**: Use `Arc<Mutex<Option<bool>>>` with explicit invalidation method `invalidate_onboarding_cache()` called when config is saved.
- **Option C (Over-engineered)**: Use `tokio::sync::watch::channel` to emit invalidation events when state changes.

**Recommendation**: Implement Option A for MVP (no memoization). Add caching layer later if profiling shows it's hot.

### 2. Step Recomputation (projectOnboardingState.ts:19-41)

**Problem**: `getSteps()` computes flags once; they don't reflect workspace state changes mid-session.

**Rust Implementation**: Recompute `get_steps()` on demand. Cache only if filesystem watchers are in place (not in MVP scope).

### 3. Embedded Prompt Strings (commands/init.ts:6-224)

**Problem**: ~190 lines of NEW_INIT_PROMPT embedded in code; maintenance burden.

**Rust Implementation**: Extract to `src/commands/init/prompts/new_init.md` and load at runtime via `include_str!` macro.

## Medium Priorities (Address in Port)

### 4. Error Handling in Config Save

**Source**: `saveCurrentProjectConfig()` called without error handling in:
- `maybeMarkProjectOnboardingComplete()` (line 56)
- `incrementProjectOnboardingSeenCount()` (line 79)

**Rust Implementation**: Define `ConfigError` enum with `#[thiserror]`. Return `Result<(), ConfigError>` from both functions. Propagate to callers.

### 5. Feature Flag Duplication

**Source** (lines 230-232, 246-248): Same feature check in two places.

**Rust Implementation**: Extract to `fn should_use_new_init_workflow() -> bool`:
```rust
fn should_use_new_init_workflow() -> bool {
    cfg!(feature = "new_init") && (
        env::var("USER_TYPE").ok() == Some("ant".to_string()) ||
        env::var("CLAUDE_CODE_NEW_INIT").ok().map_or(false, |v| is_env_truthy(&v))
    )
}
```

### 6. Type-Safe Command Source

**Source**: `source: 'builtin'` is string literal.

**Rust Implementation**:
```rust
#[derive(Debug, Clone, Copy)]
pub enum CommandSource {
    Builtin,
    Custom,
}

pub struct Command {
    pub source: CommandSource,
    // ...
}
```

## Low Priorities (Nice-to-Have)

### 7. Step Type Improvements

**Source**: `Step` type with multiple boolean flags.

**Rust Implementation**:
```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StepKey {
    Workspace,
    ClaudeMd,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Step {
    pub key: StepKey,
    pub text: String,
    pub is_complete: bool,
    pub is_completable: bool,
    pub is_enabled: bool,
}
```

Alternatively, use enum to represent state:
```rust
pub enum StepState {
    NotStarted,
    InProgress,
    Complete,
    Blocked { reason: String },
}
```

### 8. ProjectConfig Type Safety

**Source**: `projectOnboardingSeenCount` assumed to be integer.

**Rust Implementation**:
```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectConfig {
    pub has_completed_project_onboarding: bool,
    pub project_onboarding_seen_count: u32,
    // ...
}
```

## Dependencies to Wire Up

- `serde` + `serde_json` for config I/O (replace config.js)
- `std::path::PathBuf` for path operations
- `tokio` for async patterns (if keeping async)
- `thiserror` or `anyhow` for error handling
- `once_cell` or `std::sync::OnceLock` for lazy statics (if needed)

## Build & Test Checklist

- [ ] Rust implementation compiles with `cargo build -p coding-agent` (zero warnings)
- [ ] All tests pass: `cargo test`
- [ ] Config I/O works (reading/writing CLAUDE.md detection, seen count)
- [ ] Feature flag branching works correctly
- [ ] Step computation reflects actual filesystem state
- [ ] No hidden I/O in getters (move side effects out)
- [ ] Push to branch: `git push origin v0.2.0/sage-init`

## Files to Create/Modify

- `src/commands/init.rs` — Main command definition
- `src/commands/init/prompts/new_init.md` — Extracted prompt
- `src/project_onboarding.rs` — State management module
- `src/errors.rs` — Add `ConfigError` enum (or update existing)
- Update `src/main.rs` or command registration to wire in new command
