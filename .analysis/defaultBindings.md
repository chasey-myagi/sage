# Analysis: keybindings/defaultBindings.ts

Summary: Defines platform-specific default keybindings organized by UI context (Global, Chat, Settings, etc.). Uses feature flags and version checks for terminal capability detection.

## Issues

- [ ] ISSUE [HIGH]: Reserved keys cannot be rebound but enforcement is runtime-only
  Impact: Users can override ctrl+c/ctrl+d despite comments saying they cannot. Validation catches it, but type system doesn't prevent it.
  Suggestion: Move reserved keys to a separate readonly list and use type system to prevent them in the Rust model

- [ ] ISSUE [MEDIUM]: Feature flag system (Bun bundle features) needs Rust equivalent
  Impact: Lines 45, 52, 60, 88, 96, 268 use `feature()` function. Sage runtime must provide feature gate access.
  Suggestion: Define a feature provider trait/interface that the keybindings system depends on

- [ ] ISSUE [MEDIUM]: Complex platform/version detection logic for VT mode support
  Impact: Lines 21-25 check Bun vs Node.js versions to determine terminal VT mode. Brittle logic.
  Suggestion: Extract to platform detection module; provide a capability checker trait

## Optimizations

- [ ] OPT [IDIOM]: Extract platform capability checks into separate module
  Why better: Current logic mixes platform detection with keybinding defaults
  Approach: Create `platform.rs` module with `TerminalCapabilities` struct

- [ ] OPT [SAFETY]: Use enum for context names instead of string literals
  Why better: Typos in context strings could cause silent failures
  Approach: Use Rust enum, derive from types.rs
