# Analysis: keybindings/resolver.ts

Summary: Core keybinding resolution engine. Matches key input against parsed bindings with context awareness. Supports multi-keystroke chords with pending state management and "last binding wins" override semantics.

## External Dependencies

- **Ink library (Key type)** — Used on line 1: `import type { Key } from '../ink.js'`. Represents keyboard input with modifier flags (ctrl, alt, shift, meta, super).
  - Rust equivalent: Define a custom `KeyInput` struct with modifier flags, or depend on a terminal input crate like `crossterm` / `termion`
  - Risk: Ink's Key semantics may differ from terminal library semantics (e.g., escape key meta quirk on line 89)

## Issues

- [ ] ISSUE [HIGH]: Ink.Key dependency with undocumented quirks
  Impact: Line 86-89: "QUIRK: Ink sets key.meta=true when escape is pressed". This non-obvious behavior must be replicated in Rust.
  Suggestion: Document this quirk; create KeyInput wrapper that normalizes Ink's escape behavior on input

- [ ] ISSUE [HIGH]: Nullable action field has no type safety
  Impact: action is `string | null` (unbind marker). Must check on lines 56, 232. Type system doesn't prevent null dereference.
  Suggestion: Use `Option<String>` in Rust; `None` is explicit

- [ ] ISSUE [MEDIUM]: Chord state is not thread-safe
  Impact: pending: ParsedKeystroke[] | null is mutable state tracked by caller. No synchronization if multiple contexts use resolver concurrently.
  Suggestion: Ensure caller properly synchronizes chord state; document this assumption

- [ ] ISSUE [MEDIUM]: Chord prefix matching uses O(n·m) loop (line 196-207)
  Impact: For each chord in contextBindings, test if testChord is a prefix. Scales poorly with many multi-key bindings.
  Suggestion: Use Trie structure for O(m) prefix matching, though current scale may not justify it

## Optimizations

- [ ] OPT [IDIOM]: Use HashMap for context lookup instead of Set + filter
  Why better: Line 193-194 filters bindings; could pre-index by context
  Approach: Build once: `HashMap<ContextName, Vec<ParsedBinding>>` during initialization

- [ ] OPT [SAFETY]: Replace "last match wins" loop with rfind
  Why better: Current loop (line 225-228) iterates through all contextBindings; rfind does it in reverse once
  Approach: Use `rfind()` iterator method: `contextBindings.rfind(|b| chordExactlyMatches(testChord, b))`

- [ ] OPT [ERGONOMICS]: Extract chord matching into separate Matcher struct
  Why better: Reduces function parameters; encapsulates matching logic
  Approach: Create `ChordMatcher { bindings, context_map }` with methods for prefix/exact matching
