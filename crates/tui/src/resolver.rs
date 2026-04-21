//! Key resolver with chord support.
//!
//! Ported from CC `keybindings/resolver.ts`.

use crate::keybindings::KeybindingsConfig;
use crate::keys::matches_key;

// =============================================================================
// Types
// =============================================================================

/// A single parsed keystroke (one step in a chord).
#[derive(Debug, Clone, PartialEq)]
pub struct ParsedKeystroke {
    pub key: String,
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub meta: bool,
}

/// Pending state for multi-keystroke chord matching.
pub type ChordState = Vec<ParsedKeystroke>;

/// The result of resolving a key input.
#[derive(Debug, Clone, PartialEq)]
pub enum ResolveResult {
    /// Input matched a bound action.
    Match(String),
    /// No binding found for this input.
    None,
    /// Partial chord match; more keystrokes expected.
    ChordStarted(ChordState),
    /// An in-progress chord was cancelled (e.g., by escape or an unrecognized key).
    ChordCancelled,
}

// =============================================================================
// Internal helpers
// =============================================================================

/// Parse a single key-ID string like `"ctrl+x"` into a `ParsedKeystroke`.
pub fn parse_keystroke(key_id: &str) -> ParsedKeystroke {
    let lower = key_id.to_lowercase();
    let parts: Vec<&str> = lower.split('+').collect();
    let key = parts.last().copied().unwrap_or("").to_string();
    ParsedKeystroke {
        key,
        ctrl: parts.contains(&"ctrl"),
        alt: parts.contains(&"alt"),
        shift: parts.contains(&"shift"),
        meta: parts.contains(&"meta"),
    }
}

/// Parse a chord string (space-separated key IDs) into individual keystrokes.
fn parse_chord_str(chord_str: &str) -> Vec<ParsedKeystroke> {
    chord_str.split(' ').map(parse_keystroke).collect()
}

/// Convert a `ParsedKeystroke` back to a key-ID string for use with `matches_key`.
fn keystroke_to_key_id(ks: &ParsedKeystroke) -> String {
    let mut parts: Vec<&str> = Vec::new();
    if ks.ctrl {
        parts.push("ctrl");
    }
    if ks.alt || ks.meta {
        parts.push("alt");
    }
    if ks.shift {
        parts.push("shift");
    }
    parts.push(&ks.key);
    parts.join("+")
}

/// Check whether two `ParsedKeystroke`s represent the same key combination.
///
/// Collapses `alt` and `meta` into one logical modifier — legacy terminals
/// cannot distinguish them (see match.ts in CC).
fn keystrokes_equal(a: &ParsedKeystroke, b: &ParsedKeystroke) -> bool {
    a.key == b.key
        && a.ctrl == b.ctrl
        && a.shift == b.shift
        && (a.alt || a.meta) == (b.alt || b.meta)
}

/// Check whether `data` matches a `ParsedKeystroke`.
fn data_matches_keystroke(data: &str, ks: &ParsedKeystroke) -> bool {
    matches_key(data, &keystroke_to_key_id(ks))
}

// =============================================================================
// Public API
// =============================================================================

/// Resolve a single-keystroke key input against bindings.
///
/// Only considers single-keystroke (non-chord) bindings. Last match wins,
/// matching CC's "last-match-wins" override semantics.
///
/// The `contexts` parameter is accepted for API compatibility; it is not yet
/// used because `KeybindingsConfig` does not carry context metadata.
pub fn resolve_key(data: &str, _contexts: &[&str], bindings: &KeybindingsConfig) -> ResolveResult {
    let mut matched: Option<String> = None;

    for (action, keys) in bindings {
        for key_id in keys {
            let s = key_id.as_str();
            if s.contains(' ') {
                continue; // skip chord bindings
            }
            if matches_key(data, s) {
                matched = Some(action.clone());
            }
        }
    }

    match matched {
        Some(action) => ResolveResult::Match(action),
        None => ResolveResult::None,
    }
}

/// Resolve a key input with chord state tracking.
///
/// Handles multi-keystroke bindings like `"ctrl+x ctrl+e"`.
///
/// # Arguments
/// * `data` – raw terminal input bytes
/// * `_contexts` – active context names (reserved for future context filtering)
/// * `bindings` – keybinding config; chord bindings use space-separated key IDs
/// * `pending` – mutable chord state; `None` when not in a chord
///
/// # Semantics (mirrors CC resolver.ts)
/// - Escape always cancels a pending chord.
/// - Longer chords take priority over exact single-key matches.
/// - Last match wins among bindings at the same chord length.
pub fn resolve_key_with_chord_state(
    data: &str,
    _contexts: &[&str],
    bindings: &KeybindingsConfig,
    pending: &mut Option<ChordState>,
) -> ResolveResult {
    // Escape always cancels an in-progress chord.
    if data == "\x1b" && pending.is_some() {
        *pending = None;
        return ResolveResult::ChordCancelled;
    }

    let prefix_len = pending.as_ref().map_or(0, |p| p.len());

    let mut has_longer_chord = false;
    let mut exact_action: Option<String> = None;
    let mut next_keystroke: Option<ParsedKeystroke> = None;

    for (action, keys) in bindings {
        for key_id in keys {
            let chord = parse_chord_str(key_id.as_str());
            if chord.is_empty() || chord.len() <= prefix_len {
                continue;
            }

            // Verify the pending prefix matches this chord's prefix.
            let prefix_ok = match pending.as_ref() {
                Some(pend) => pend
                    .iter()
                    .zip(chord.iter())
                    .all(|(expected, got)| keystrokes_equal(expected, got)),
                None => true,
            };
            if !prefix_ok {
                continue;
            }

            let next_step = &chord[prefix_len];
            if !data_matches_keystroke(data, next_step) {
                continue;
            }

            if chord.len() == prefix_len + 1 {
                // Exact match — last one wins.
                exact_action = Some(action.clone());
                next_keystroke = Some(next_step.clone());
            } else {
                // This binding extends further; prefer waiting.
                has_longer_chord = true;
                if next_keystroke.is_none() {
                    next_keystroke = Some(next_step.clone());
                }
            }
        }
    }

    // Longer chords take priority over exact matches (CC semantics).
    if has_longer_chord {
        let ks = next_keystroke.unwrap();
        let mut new_state = pending.clone().unwrap_or_default();
        new_state.push(ks);
        *pending = Some(new_state.clone());
        return ResolveResult::ChordStarted(new_state);
    }

    if let Some(action) = exact_action {
        *pending = None;
        return ResolveResult::Match(action);
    }

    // No match.
    let had_pending = pending.is_some();
    *pending = None;
    if had_pending {
        ResolveResult::ChordCancelled
    } else {
        ResolveResult::None
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keys::KeyId;
    use indexmap::IndexMap;

    fn bindings(pairs: &[(&str, &[&str])]) -> KeybindingsConfig {
        pairs
            .iter()
            .map(|(action, keys)| {
                (
                    action.to_string(),
                    keys.iter().map(|k| KeyId::from(*k)).collect(),
                )
            })
            .collect()
    }

    // -- parse_keystroke --

    #[test]
    fn parse_keystroke_ctrl_x() {
        let ks = parse_keystroke("ctrl+x");
        assert_eq!(ks.key, "x");
        assert!(ks.ctrl);
        assert!(!ks.shift);
        assert!(!ks.alt);
    }

    #[test]
    fn parse_keystroke_alt_enter() {
        let ks = parse_keystroke("alt+enter");
        assert_eq!(ks.key, "enter");
        assert!(ks.alt);
        assert!(!ks.ctrl);
    }

    #[test]
    fn parse_keystroke_plain_key() {
        let ks = parse_keystroke("escape");
        assert_eq!(ks.key, "escape");
        assert!(!ks.ctrl);
        assert!(!ks.alt);
    }

    // -- resolve_key --

    #[test]
    fn resolve_key_single_match() {
        let b = bindings(&[("action.submit", &["enter"])]);
        assert_eq!(
            resolve_key("\r", &["Global"], &b),
            ResolveResult::Match("action.submit".to_string())
        );
    }

    #[test]
    fn resolve_key_no_match() {
        let b = bindings(&[("action.submit", &["enter"])]);
        assert_eq!(resolve_key("\x1b", &["Global"], &b), ResolveResult::None);
    }

    #[test]
    fn resolve_key_skips_chords() {
        let b = bindings(&[("action.ext", &["ctrl+x ctrl+e"])]);
        // ctrl+x alone should NOT match the chord action
        assert_eq!(resolve_key("\x18", &["Global"], &b), ResolveResult::None);
    }

    #[test]
    fn resolve_key_last_match_wins() {
        let mut b: KeybindingsConfig = IndexMap::new();
        b.insert("action.a".to_string(), vec![KeyId::from("ctrl+k")]);
        b.insert("action.b".to_string(), vec![KeyId::from("ctrl+k")]);
        // Both bind ctrl+k; IndexMap preserves insertion order so action.b wins.
        let result = resolve_key("\x0b", &["Global"], &b);
        assert_eq!(result, ResolveResult::Match("action.b".to_string()));
    }

    // -- resolve_key_with_chord_state --

    #[test]
    fn chord_two_step_match() {
        // ctrl+x ctrl+e → action.external
        let b = bindings(&[("action.external", &["ctrl+x ctrl+e"])]);
        let mut pending: Option<ChordState> = None;

        // Step 1: ctrl+x → chord started
        let r = resolve_key_with_chord_state("\x18", &["Global"], &b, &mut pending);
        assert!(matches!(r, ResolveResult::ChordStarted(_)));
        assert!(pending.is_some());

        // Step 2: ctrl+e → match
        let r = resolve_key_with_chord_state("\x05", &["Global"], &b, &mut pending);
        assert_eq!(r, ResolveResult::Match("action.external".to_string()));
        assert!(pending.is_none());
    }

    #[test]
    fn chord_cancelled_by_escape() {
        let b = bindings(&[("action.external", &["ctrl+x ctrl+e"])]);
        let mut pending: Option<ChordState> = None;

        // Start chord
        resolve_key_with_chord_state("\x18", &["Global"], &b, &mut pending);
        assert!(pending.is_some());

        // Escape cancels it
        let r = resolve_key_with_chord_state("\x1b", &["Global"], &b, &mut pending);
        assert_eq!(r, ResolveResult::ChordCancelled);
        assert!(pending.is_none());
    }

    #[test]
    fn chord_cancelled_by_wrong_key() {
        let b = bindings(&[("action.external", &["ctrl+x ctrl+e"])]);
        let mut pending: Option<ChordState> = None;

        // Start chord with ctrl+x
        resolve_key_with_chord_state("\x18", &["Global"], &b, &mut pending);
        assert!(pending.is_some());

        // Wrong second key → cancelled
        let r = resolve_key_with_chord_state("\x0b", &["Global"], &b, &mut pending);
        assert_eq!(r, ResolveResult::ChordCancelled);
        assert!(pending.is_none());
    }

    #[test]
    fn single_key_no_chord_none() {
        let b = bindings(&[("action.external", &["ctrl+x ctrl+e"])]);
        let mut pending: Option<ChordState> = None;

        // ctrl+e alone with no pending chord → None
        let r = resolve_key_with_chord_state("\x05", &["Global"], &b, &mut pending);
        assert_eq!(r, ResolveResult::None);
    }

    #[test]
    fn longer_chord_preferred_over_single_match() {
        // Both a single-key binding and a two-step chord share the same first key.
        let mut b: KeybindingsConfig = IndexMap::new();
        b.insert("action.single".to_string(), vec![KeyId::from("ctrl+x")]);
        b.insert(
            "action.chord".to_string(),
            vec![KeyId::from("ctrl+x ctrl+k")],
        );
        let mut pending: Option<ChordState> = None;

        // ctrl+x: the chord binding takes priority (longer chord wins).
        let r = resolve_key_with_chord_state("\x18", &["Global"], &b, &mut pending);
        assert!(
            matches!(r, ResolveResult::ChordStarted(_)),
            "longer chord should win over single key match, got {r:?}"
        );
    }
}
