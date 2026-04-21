# Analysis: keybindings/validate.ts

Summary: Comprehensive validation system for keybinding configuration. Validates syntax, context names, action validity, duplicate detection, and reserved shortcut conflicts. Uses regex to detect JSON duplicate keys.

## External Dependencies

- **reservedShortcuts.ts** (line 4-6) — Provides reserved shortcut list and key normalization
  - Rust equivalent: Define reserved list as constant or module

## Issues

- [ ] ISSUE [HIGH]: Regex-based JSON duplicate key detection is fragile
  Impact: Lines 258-307: `checkDuplicateKeysInJson()` uses complex regex to re-parse JSON looking for duplicate keys. This breaks if JSON format varies (whitespace, nesting).
  Suggestion: Use proper JSON parser that tracks key order/duplicates. Or: use serde_json with custom deserializer that logs duplicate keys.

- [ ] ISSUE [MEDIUM]: Command binding validation is ad-hoc
  Impact: Line 196-206: Regex `/^command:[a-zA-Z0-9:\-_]+$/` validates command format inline. Command existence not checked; format rules are string-based.
  Suggestion: Define CommandName type in types.rs with validation; centralize command registry

- [ ] ISSUE [MEDIUM]: Valid context hardcoded; must stay in sync with types.ts
  Impact: Lines 60-79: VALID_CONTEXTS array must match KeybindingContextName type. No compile-time guarantee.
  Suggestion: Derive VALID_CONTEXTS from KeybindingContextName type using macro or code-gen

- [ ] ISSUE [MEDIUM]: Voice:pushToTalk validation is heuristic
  Impact: Line 220-242: Checks if binding is a bare letter (no modifiers) and warns. But warning is advisory; binding still works.
  Suggestion: Document why bare letters are problematic (auto-repeat during hold); consider error vs warning level

- [ ] ISSUE [LOW]: validateKeystroke allows empty key field
  Impact: Line 108-122: parseKeystroke() can return keystroke with empty key, but validation only checks if at least one field is set. Empty key can pass.
  Suggestion: Explicitly check `!parsed.key` as error condition

## Optimizations

- [ ] OPT [SAFETY]: Remove manual JSON regex parsing
  Why better: Fragile to JSON format changes; proper parsing is more reliable
  Approach: Use serde_json `RawValue` or streaming parser to detect duplicate keys properly

- [ ] OPT [IDIOM]: Use type-level validation for contexts
  Why better: Current array-based check is runtime; should be compile-time
  Approach: Use macro to derive VALID_CONTEXTS from enum, or use `strum` crate

- [ ] OPT [IDIOM]: Validator trait for composable validations
  Why better: Current function chaining (line 425-451) is hard to extend
  Approach: Define `trait KeybindingValidator`, implement for each check, compose in pipeline

- [ ] OPT [ERGONOMICS]: Separate user config validation from parsed bindings validation
  Why better: Current validateBindings() mixes two concerns (line 425-451)
  Approach: Split into `validate_structure()` and `validate_semantics()`

- [ ] OPT [PERF]: Cache VALID_CONTEXTS.includes() with HashSet
  Why better: isValidContext() called per binding; array includes() is O(n)
  Approach: Use lazy_static `HashSet<&'static str>`
