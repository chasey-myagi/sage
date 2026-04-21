# Analysis: keybindings/parser.ts

Summary: Pure utility functions to parse keystroke strings ("ctrl+shift+k") into structured ParsedKeystroke objects and convert back to display strings with platform-aware naming.

## Issues

- [ ] ISSUE [MEDIUM]: Unicode arrow symbols hardcoded (↑↓←→)
  Impact: Lines 56-66 accept Unicode arrows but parser also accepts ASCII names. Terminal/input system may not reliably produce these symbols.
  Suggestion: Accept arrows only in display context; normalize input to 'up'/'down'/'left'/'right' keys

- [ ] ISSUE [LOW]: Empty key case not handled
  Impact: parseKeystroke('ctrl+') returns keystroke with empty key field. Callers must check, but no guard.
  Suggestion: Either return null for invalid input or validate in parseChord before returning

- [ ] ISSUE [LOW]: Key name case normalization inconsistency
  Impact: Line 69 normalizes to lowercase for all keys including 'space' (converted to ' '). But special cases like 'esc' are mapped before case check.
  Suggestion: Consolidate all key aliases before or after case normalization

## Optimizations

- [ ] OPT [IDIOM]: Replace switch statement with HashMap for key aliases
  Why better: More data-driven, easier to extend, matches Rust idiomatic pattern
  Approach: Use `phf::Map` for compile-time perfect hash

- [ ] OPT [IDIOM]: keystrokeToString can be derived or use Display trait
  Why better: Makes it auto-available in string contexts
  Approach: Implement Display trait for ParsedKeystroke

- [ ] OPT [PERF]: Create once, reuse keyToDisplayName mapping
  Why better: Function called per keystroke, lookups repeated
  Approach: Use static HashMap initialized once
