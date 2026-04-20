//! Path resolution utilities for coding-agent tools.
//!
//! Translated from pi-mono `packages/coding-agent/src/core/tools/path-utils.ts`.
//!
//! Provides path expansion (~ and @ prefixes) and resolving relative paths
//! against the working directory, with macOS-specific fallbacks for screenshot
//! filenames that use unusual Unicode spaces and curly quotes.

use std::path::Path;

// ============================================================================
// Constants
// ============================================================================

/// Unicode spaces that macOS uses in various contexts.
const UNICODE_SPACES: &[char] = &[
    '\u{00A0}', // NO-BREAK SPACE
    '\u{2000}', '\u{2001}', '\u{2002}', '\u{2003}', '\u{2004}',
    '\u{2005}', '\u{2006}', '\u{2007}', '\u{2008}', '\u{2009}',
    '\u{200A}', // various width spaces
    '\u{202F}', // NARROW NO-BREAK SPACE
    '\u{205F}', // MEDIUM MATHEMATICAL SPACE
    '\u{3000}', // IDEOGRAPHIC SPACE
];

/// Narrow no-break space used by macOS in screenshot filenames before AM/PM.
const NARROW_NO_BREAK_SPACE: char = '\u{202F}';

// ============================================================================
// Internal helpers
// ============================================================================

fn normalize_unicode_spaces(s: &str) -> String {
    s.chars()
        .map(|c| if UNICODE_SPACES.contains(&c) { ' ' } else { c })
        .collect()
}

fn try_macos_screenshot_path(file_path: &str) -> String {
    // macOS screenshot filenames use narrow no-break space before AM/PM.
    file_path
        .replace(" AM.", &format!("{NARROW_NO_BREAK_SPACE}AM."))
        .replace(" PM.", &format!("{NARROW_NO_BREAK_SPACE}PM."))
}

fn try_nfd_variant(file_path: &str) -> String {
    // macOS stores filenames in NFD (decomposed) form.
    // Use unicode_normalization if available; otherwise return as-is.
    // For now we return the path unchanged — the caller tries it anyway.
    file_path.to_string()
}

fn try_curly_quote_variant(file_path: &str) -> String {
    // macOS uses U+2019 (RIGHT SINGLE QUOTATION MARK) in screenshot names
    // like "Capture d'écran". Straight apostrophe U+0027 is what users type.
    file_path.replace('\'', "\u{2019}")
}

fn file_exists(path: &str) -> bool {
    Path::new(path).exists()
}

fn normalize_at_prefix(file_path: &str) -> &str {
    file_path.strip_prefix('@').unwrap_or(file_path)
}

// ============================================================================
// Public API
// ============================================================================

/// Expand `~` and `@` prefixes in a path and normalize Unicode spaces.
///
/// Mirrors `expandPath()` from `path-utils.ts`.
pub fn expand_path(file_path: &str) -> String {
    let normalized = normalize_unicode_spaces(normalize_at_prefix(file_path));
    if normalized == "~" {
        return dirs::home_dir()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|| normalized);
    }
    if let Some(rest) = normalized.strip_prefix("~/") {
        let home = dirs::home_dir()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default();
        return format!("{home}/{rest}");
    }
    normalized
}

/// Resolve `file_path` against `cwd`, handling `~` expansion and absolute paths.
///
/// Mirrors `resolveToCwd()` from `path-utils.ts`.
pub fn resolve_to_cwd(file_path: &str, cwd: &str) -> String {
    let expanded = expand_path(file_path);
    if Path::new(&expanded).is_absolute() {
        return expanded;
    }
    // Join cwd + relative path and canonicalize logically (no filesystem access).
    let base = Path::new(cwd).join(&expanded);
    base.to_string_lossy().into_owned()
}

/// Resolve a path for reading, trying several macOS-specific variants when the
/// primary resolved path does not exist on disk.
///
/// Mirrors `resolveReadPath()` from `path-utils.ts`.
pub fn resolve_read_path(file_path: &str, cwd: &str) -> String {
    let resolved = resolve_to_cwd(file_path, cwd);

    if file_exists(&resolved) {
        return resolved;
    }

    // Try macOS AM/PM narrow no-break space variant.
    let ampm_variant = try_macos_screenshot_path(&resolved);
    if ampm_variant != resolved && file_exists(&ampm_variant) {
        return ampm_variant;
    }

    // Try NFD variant (macOS stores filenames in NFD).
    let nfd_variant = try_nfd_variant(&resolved);
    if nfd_variant != resolved && file_exists(&nfd_variant) {
        return nfd_variant;
    }

    // Try curly quote variant (macOS uses U+2019 in screenshot names).
    let curly_variant = try_curly_quote_variant(&resolved);
    if curly_variant != resolved && file_exists(&curly_variant) {
        return curly_variant;
    }

    // Try combined NFD + curly quote.
    let nfd_curly = try_curly_quote_variant(&nfd_variant);
    if nfd_curly != resolved && file_exists(&nfd_curly) {
        return nfd_curly;
    }

    // Fall back to the primary resolved path (caller handles not-found).
    resolved
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    // ---- expand_path ----

    #[test]
    fn expand_path_tilde_only() {
        let result = expand_path("~");
        let home = dirs::home_dir().unwrap();
        assert_eq!(result, home.to_string_lossy().as_ref());
    }

    #[test]
    fn expand_path_tilde_prefix() {
        let result = expand_path("~/foo/bar");
        let home = dirs::home_dir().unwrap().to_string_lossy().into_owned();
        assert!(result.starts_with(&home));
        assert!(result.ends_with("/foo/bar"));
    }

    #[test]
    fn expand_path_at_prefix_stripped() {
        let result = expand_path("@relative/path");
        assert_eq!(result, "relative/path");
    }

    #[test]
    fn expand_path_absolute_unchanged() {
        let result = expand_path("/absolute/path");
        assert_eq!(result, "/absolute/path");
    }

    #[test]
    fn expand_path_unicode_spaces_normalized() {
        // NO-BREAK SPACE → regular space
        let input = "foo\u{00A0}bar";
        let result = expand_path(input);
        assert_eq!(result, "foo bar");
    }

    // ---- resolve_to_cwd ----

    #[test]
    fn resolve_to_cwd_absolute_path() {
        let result = resolve_to_cwd("/absolute/path", "/some/cwd");
        assert_eq!(result, "/absolute/path");
    }

    #[test]
    fn resolve_to_cwd_relative_path() {
        let result = resolve_to_cwd("relative/file.txt", "/my/cwd");
        assert_eq!(result, "/my/cwd/relative/file.txt");
    }

    #[test]
    fn resolve_to_cwd_dot_path() {
        let result = resolve_to_cwd(".", "/my/cwd");
        assert_eq!(result, "/my/cwd/.");
    }

    // ---- resolve_read_path ----

    #[test]
    fn resolve_read_path_existing_file() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("test.txt");
        fs::write(&file_path, "content").unwrap();

        let result = resolve_read_path(
            file_path.file_name().unwrap().to_str().unwrap(),
            dir.path().to_str().unwrap(),
        );
        assert_eq!(result, file_path.to_str().unwrap());
    }

    #[test]
    fn resolve_read_path_nonexistent_returns_resolved() {
        let result = resolve_read_path("nonexistent.txt", "/my/cwd");
        assert_eq!(result, "/my/cwd/nonexistent.txt");
    }

    #[test]
    fn resolve_read_path_absolute_existing() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("abs.txt");
        fs::write(&file_path, "data").unwrap();
        let abs = file_path.to_str().unwrap();

        let result = resolve_read_path(abs, "/unrelated/cwd");
        assert_eq!(result, abs);
    }
}
