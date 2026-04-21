# Analysis: commands/init.ts

Summary: TypeScript command definition that exports the `/init` command, with two prompt variants (legacy and new 8-phase) for CLAUDE.md setup workflow. Feature-flagged via `NEW_INIT` and environment variables.

## Issues

- [ ] ISSUE [HIGH]: Long embedded prompt strings (NEW_INIT_PROMPT ~190 lines). These are maintenance burdens — changes to prompt structure aren't version-controlled separately. Suggestion: extract prompts to separate .md or .txt files and load them at runtime, allowing future prompts to be edited independently of the command logic.
  Impact: Prompt evolution gets messy; any prompt change requires touching this file and breaking the command structure.
  Suggestion: Create `src/commands/init/prompts/` directory with `old-prompt.md` and `new-prompt.md` files. Load them via `readFileSync(fileURLToPath(import.meta.resolve('...')))`

- [ ] ISSUE [MEDIUM]: Feature flag decision tree (`feature('NEW_INIT') && (process.env.USER_TYPE === 'ant' || isEnvTruthy(process.env.CLAUDE_CODE_NEW_INIT))`) appears twice (line 230-232 and 246-248), creating duplication. In Rust, this becomes error-prone if flags diverge.
  Impact: Future maintainers must keep both checks in sync; adding a third flag check requires updating both locations.
  Suggestion: Extract flag check to a helper function: `function shouldUseNewInitWorkflow(): boolean`

- [ ] ISSUE [MEDIUM]: `Command` type contract is loose. No validation that `getPromptForCommand()` returns exactly what the system expects. When porting to Rust, the compiler won't catch mismatches between expected and actual return shape.
  Impact: Runtime errors if prompt structure changes unexpectedly.
  Suggestion: Narrow `getPromptForCommand()` return type or add inline JSDoc describing the exact shape expected.

- [ ] ISSUE [LOW]: `contentLength: 0` with comment "Dynamic content" indicates the system can't know content size upfront. Rust porting must preserve this pattern — metadata claims don't match reality.
  Impact: UI may misbehave if it assumes contentLength > 0 for rendering estimates.
  Suggestion: Document why this field must be 0 (dynamic prompt at runtime).

## Optimizations

- [ ] OPT [IDIOM]: `source: 'builtin'` is a string literal. In Rust, this should be an enum (`enum CommandSource { Builtin, Custom }`).
  Why better: Provides type safety and exhaustive matching at compile time.
  Approach: Define `CommandSource` enum and use `source: CommandSource::Builtin`.

- [ ] OPT [ERGONOMICS]: `description` is computed via a ternary in a getter. Rust should compute this once and store it (or make the `description` field lazy).
  Why better: Avoids re-evaluating the same condition on every property access.
  Approach: Compute and store in a static/const or use `once_cell::sync::Lazy` for deferred single evaluation.

- [ ] OPT [SAFETY]: `process.env.USER_TYPE` and `process.env.CLAUDE_CODE_NEW_INIT` are read with no null checks until `isEnvTruthy()`. If env vars are undefined, the check still works (falsy), but it's implicit. Rust would require explicit `Option<String>` handling.
  Why better: Makes intent explicit and testable.
  Approach: Use named helper `function getNewInitFlag(): boolean { return isEnvTruthy(process.env.CLAUDE_CODE_NEW_INIT) }` and clarify that `USER_TYPE !== 'ant'` check is also a gate.

- [ ] OPT [PERF]: Calling `maybeMarkProjectOnboardingComplete()` on every command invocation (via `getPromptForCommand()`) is a side effect hidden in a getter. This could trigger filesystem writes on read-only operations.
  Why better: Separates pure data retrieval from side effects.
  Approach: Move the call to an explicit init setup phase, not the property accessor. Or use a separate `onCommandInvoke()` lifecycle hook.
