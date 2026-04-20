//! Shared diff-computation utilities for the edit tool.
//!
//! Translated from pi-mono `packages/coding-agent/src/core/tools/edit-diff.ts`.
//!
//! Used by both the edit tool (for execution) and by preview rendering in the
//! TUI before the tool runs.

use std::path::Path;
use unicode_normalization::UnicodeNormalization;

// ============================================================================
// Line ending detection / normalization
// ============================================================================

/// Detect the dominant line ending in `content`.
///
/// Returns `"\r\n"` when the first `\r\n` appears before the first lone `\n`,
/// otherwise `"\n"`.
///
/// Mirrors `detectLineEnding()` from `edit-diff.ts`.
pub fn detect_line_ending(content: &str) -> &'static str {
    let crlf_idx = content.find("\r\n");
    let lf_idx = content.find('\n');
    match (crlf_idx, lf_idx) {
        (Some(c), Some(l)) if c < l => "\r\n",
        _ => "\n",
    }
}

/// Convert all line endings to `\n`.
///
/// Mirrors `normalizeToLF()` from `edit-diff.ts`.
pub fn normalize_to_lf(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n")
}

/// Restore line endings from `\n` back to `ending`.
///
/// Mirrors `restoreLineEndings()` from `edit-diff.ts`.
pub fn restore_line_endings(text: &str, ending: &str) -> String {
    if ending == "\r\n" {
        text.replace('\n', "\r\n")
    } else {
        text.to_string()
    }
}

// ============================================================================
// Unicode normalization for fuzzy matching
// ============================================================================

/// Normalize text for fuzzy matching.
///
/// Applies progressive transformations:
/// - Strip trailing whitespace from each line
/// - Normalize smart quotes to ASCII equivalents
/// - Normalize Unicode dashes/hyphens to ASCII hyphen
/// - Normalize special Unicode spaces to regular space
///
/// Mirrors `normalizeForFuzzyMatch()` from `edit-diff.ts`.
pub fn normalize_for_fuzzy_match(text: &str) -> String {
    // NFKC normalization first — matches TypeScript's text.normalize("NFKC").
    // This ensures fullwidth characters, ligatures, and decomposed accented
    // characters compare equal to their composed equivalents.
    let nfkc: String = text.nfkc().collect();
    nfkc.lines()
        .map(|line| line.trim_end())
        .collect::<Vec<_>>()
        .join("\n")
        // Smart single quotes → '
        .replace(['\u{2018}', '\u{2019}', '\u{201A}', '\u{201B}'], "'")
        // Smart double quotes → "
        .replace(['\u{201C}', '\u{201D}', '\u{201E}', '\u{201F}'], "\"")
        // Various dashes → -
        .replace(
            [
                '\u{2010}', '\u{2011}', '\u{2012}', '\u{2013}', '\u{2014}', '\u{2015}', '\u{2212}',
            ],
            "-",
        )
        // Special spaces → regular space
        .replace(
            [
                '\u{00A0}', '\u{2002}', '\u{2003}', '\u{2004}', '\u{2005}', '\u{2006}', '\u{2007}',
                '\u{2008}', '\u{2009}', '\u{200A}', '\u{202F}', '\u{205F}', '\u{3000}',
            ],
            " ",
        )
}

// ============================================================================
// BOM handling
// ============================================================================

/// Strip UTF-8 BOM if present; return `(bom, text_without_bom)`.
///
/// Mirrors `stripBom()` from `edit-diff.ts`.
pub fn strip_bom(content: &str) -> (&'static str, &str) {
    if content.starts_with('\u{FEFF}') {
        ("\u{FEFF}", &content['\u{FEFF}'.len_utf8()..])
    } else {
        ("", content)
    }
}

// ============================================================================
// Fuzzy text finding
// ============================================================================

/// Result of fuzzy text search.
#[derive(Debug, Clone)]
pub struct FuzzyMatchResult {
    /// Whether a match was found.
    pub found: bool,
    /// Index where the match starts (in the content used for replacement).
    pub index: usize,
    /// Length of the matched text (in bytes).
    pub match_length: usize,
    /// Whether fuzzy matching (rather than exact) was used.
    pub used_fuzzy_match: bool,
    /// The content to use for replacement operations.
    ///
    /// Exact match → original content.  Fuzzy match → fuzzy-normalized content.
    pub content_for_replacement: String,
}

/// Find `old_text` in `content`, trying exact match first, then fuzzy match.
///
/// When fuzzy matching is used, `content_for_replacement` is the
/// fuzzy-normalized version of `content` (trailing whitespace stripped, Unicode
/// quotes/dashes normalized to ASCII).
///
/// Mirrors `fuzzyFindText()` from `edit-diff.ts`.
pub fn fuzzy_find_text(content: &str, old_text: &str) -> FuzzyMatchResult {
    // 1. Try exact match first.
    if let Some(exact_index) = content.find(old_text) {
        return FuzzyMatchResult {
            found: true,
            index: exact_index,
            match_length: old_text.len(),
            used_fuzzy_match: false,
            content_for_replacement: content.to_string(),
        };
    }

    // 2. Try fuzzy match — work entirely in normalized space.
    let fuzzy_content = normalize_for_fuzzy_match(content);
    let fuzzy_old_text = normalize_for_fuzzy_match(old_text);
    if let Some(fuzzy_index) = fuzzy_content.find(&fuzzy_old_text) {
        return FuzzyMatchResult {
            found: true,
            index: fuzzy_index,
            match_length: fuzzy_old_text.len(),
            used_fuzzy_match: true,
            content_for_replacement: fuzzy_content,
        };
    }

    FuzzyMatchResult {
        found: false,
        index: 0,
        match_length: 0,
        used_fuzzy_match: false,
        content_for_replacement: content.to_string(),
    }
}

// ============================================================================
// Diff generation
// ============================================================================

/// Result of a successful diff computation.
#[derive(Debug, Clone)]
pub struct EditDiffResult {
    /// Unified-style diff string with line numbers.
    pub diff: String,
    /// Line number of the first change in the new file (1-indexed).
    pub first_changed_line: Option<usize>,
}

/// Error from a failed diff / edit preview.
#[derive(Debug, Clone)]
pub struct EditDiffError {
    pub error: String,
}

/// Generate a unified diff string with line numbers and context.
///
/// Returns both the diff string and the first changed line number (1-indexed,
/// in the new file).
///
/// Mirrors `generateDiffString()` from `edit-diff.ts`.
pub fn generate_diff_string(
    old_content: &str,
    new_content: &str,
    context_lines: Option<usize>,
) -> EditDiffResult {
    let ctx = context_lines.unwrap_or(4);

    // Simple line-based diff using longest-common-subsequence (LCS).
    let old_lines: Vec<&str> = old_content.split('\n').collect();
    let new_lines: Vec<&str> = new_content.split('\n').collect();

    let changes = compute_line_changes(&old_lines, &new_lines);
    let max_line = old_lines.len().max(new_lines.len());
    let line_num_width = format!("{max_line}").len();

    let mut output: Vec<String> = Vec::new();
    let mut first_changed_line: Option<usize> = None;

    let mut old_line = 1usize;
    let mut new_line = 1usize;
    let mut i = 0usize;

    while i < changes.len() {
        let change = &changes[i];
        match change {
            Change::Equal(lines) => {
                // Determine how many context lines to show.
                let prev_is_change = i > 0 && !matches!(changes[i - 1], Change::Equal(_));
                let next_is_change =
                    i + 1 < changes.len() && !matches!(changes[i + 1], Change::Equal(_));

                if prev_is_change || next_is_change {
                    let skip_start = if !prev_is_change {
                        lines.len().saturating_sub(ctx)
                    } else {
                        0
                    };
                    let skip_end = if !next_is_change && lines.len() > ctx {
                        lines.len() - ctx
                    } else {
                        0
                    };

                    if skip_start > 0 {
                        output.push(format!(" {pad} ...", pad = " ".repeat(line_num_width)));
                        old_line += skip_start;
                        new_line += skip_start;
                    }

                    let end_idx = lines.len() - skip_end;
                    for line in &lines[skip_start..end_idx] {
                        output.push(format!(
                            " {ln:>width$} {line}",
                            ln = old_line,
                            width = line_num_width,
                        ));
                        old_line += 1;
                        new_line += 1;
                    }

                    if skip_end > 0 {
                        output.push(format!(" {pad} ...", pad = " ".repeat(line_num_width)));
                        old_line += skip_end;
                        new_line += skip_end;
                    }
                } else {
                    old_line += lines.len();
                    new_line += lines.len();
                }
            }
            Change::Added(lines) => {
                if first_changed_line.is_none() {
                    first_changed_line = Some(new_line);
                }
                for line in lines {
                    output.push(format!(
                        "+{ln:>width$} {line}",
                        ln = new_line,
                        width = line_num_width
                    ));
                    new_line += 1;
                }
            }
            Change::Removed(lines) => {
                if first_changed_line.is_none() {
                    first_changed_line = Some(new_line);
                }
                for line in lines {
                    output.push(format!(
                        "-{ln:>width$} {line}",
                        ln = old_line,
                        width = line_num_width
                    ));
                    old_line += 1;
                }
            }
        }
        i += 1;
    }

    EditDiffResult {
        diff: output.join("\n"),
        first_changed_line,
    }
}

// ============================================================================
// Simple LCS-based line diff
// ============================================================================

#[derive(Debug)]
enum Change<'a> {
    Equal(Vec<&'a str>),
    Added(Vec<&'a str>),
    Removed(Vec<&'a str>),
}

fn compute_line_changes<'a>(old: &[&'a str], new: &[&'a str]) -> Vec<Change<'a>> {
    // Myers diff / LCS: use patience-diff approximation via DP.
    let n = old.len();
    let m = new.len();

    // Build LCS table.
    let mut dp = vec![vec![0usize; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            dp[i][j] = if old[i] == new[j] {
                dp[i + 1][j + 1] + 1
            } else {
                dp[i + 1][j].max(dp[i][j + 1])
            };
        }
    }

    // Backtrack to produce the change list.
    let mut changes: Vec<Change> = Vec::new();
    let mut i = 0;
    let mut j = 0;

    while i < n || j < m {
        if i < n && j < m && old[i] == new[j] {
            // Equal
            match changes.last_mut() {
                Some(Change::Equal(v)) => v.push(old[i]),
                _ => changes.push(Change::Equal(vec![old[i]])),
            }
            i += 1;
            j += 1;
        } else if j < m && (i >= n || dp[i][j + 1] >= dp[i + 1][j]) {
            // Added
            match changes.last_mut() {
                Some(Change::Added(v)) => v.push(new[j]),
                _ => changes.push(Change::Added(vec![new[j]])),
            }
            j += 1;
        } else {
            // Removed
            match changes.last_mut() {
                Some(Change::Removed(v)) => v.push(old[i]),
                _ => changes.push(Change::Removed(vec![old[i]])),
            }
            i += 1;
        }
    }

    changes
}

// ============================================================================
// compute_edit_diff (preview, no file write)
// ============================================================================

/// Compute the diff for an edit operation **without** applying it.
///
/// Used for preview rendering in the TUI before the tool executes.
///
/// Mirrors `computeEditDiff()` from `edit-diff.ts`.
pub fn compute_edit_diff(
    path: &str,
    old_text: &str,
    new_text: &str,
    cwd: &str,
) -> Result<EditDiffResult, EditDiffError> {
    use crate::core::tools::path_utils::resolve_to_cwd;
    use std::fs;

    let absolute_path = resolve_to_cwd(path, cwd);

    // Check if file exists and is readable.
    if !Path::new(&absolute_path).exists() {
        return Err(EditDiffError {
            error: format!("File not found: {path}"),
        });
    }

    let raw_content = fs::read_to_string(&absolute_path).map_err(|e| EditDiffError {
        error: e.to_string(),
    })?;

    // Strip BOM.
    let (_, content) = strip_bom(&raw_content);

    let normalized_content = normalize_to_lf(content);
    let normalized_old_text = normalize_to_lf(old_text);
    let normalized_new_text = normalize_to_lf(new_text);

    let match_result = fuzzy_find_text(&normalized_content, &normalized_old_text);
    if !match_result.found {
        return Err(EditDiffError {
            error: format!(
                "Could not find the exact text in {path}. The old text must match exactly \
                 including all whitespace and newlines."
            ),
        });
    }

    // Count occurrences to detect ambiguity.
    let fuzzy_content = normalize_for_fuzzy_match(&normalized_content);
    let fuzzy_old = normalize_for_fuzzy_match(&normalized_old_text);
    let occurrences = fuzzy_content.matches(&fuzzy_old).count();
    if occurrences > 1 {
        return Err(EditDiffError {
            error: format!(
                "Found {occurrences} occurrences of the text in {path}. \
                 The text must be unique. Please provide more context to make it unique."
            ),
        });
    }

    let base = &match_result.content_for_replacement;
    let new_content = format!(
        "{}{}{}",
        &base[..match_result.index],
        normalized_new_text,
        &base[match_result.index + match_result.match_length..],
    );

    if *base == new_content {
        return Err(EditDiffError {
            error: format!(
                "No changes would be made to {path}. The replacement produces identical content."
            ),
        });
    }

    Ok(generate_diff_string(base, &new_content, None))
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // ---- detect_line_ending ----

    #[test]
    fn detect_lf_only() {
        assert_eq!(detect_line_ending("line1\nline2"), "\n");
    }

    #[test]
    fn detect_crlf() {
        assert_eq!(detect_line_ending("line1\r\nline2"), "\r\n");
    }

    #[test]
    fn detect_empty_is_lf() {
        assert_eq!(detect_line_ending(""), "\n");
    }

    // ---- normalize_to_lf ----

    #[test]
    fn normalize_crlf_to_lf() {
        assert_eq!(normalize_to_lf("a\r\nb\r\nc"), "a\nb\nc");
    }

    #[test]
    fn normalize_bare_cr_to_lf() {
        assert_eq!(normalize_to_lf("a\rb"), "a\nb");
    }

    // ---- restore_line_endings ----

    #[test]
    fn restore_crlf() {
        assert_eq!(restore_line_endings("a\nb", "\r\n"), "a\r\nb");
    }

    #[test]
    fn restore_lf_unchanged() {
        assert_eq!(restore_line_endings("a\nb", "\n"), "a\nb");
    }

    // ---- normalize_for_fuzzy_match ----

    #[test]
    fn fuzzy_strips_trailing_whitespace() {
        assert_eq!(
            normalize_for_fuzzy_match("line1   \nline2\t"),
            "line1\nline2"
        );
    }

    #[test]
    fn fuzzy_normalizes_smart_quotes() {
        // LEFT SINGLE QUOTATION MARK → '
        assert_eq!(
            normalize_for_fuzzy_match("\u{2018}hello\u{2019}"),
            "'hello'"
        );
    }

    #[test]
    fn fuzzy_normalizes_em_dash() {
        assert_eq!(normalize_for_fuzzy_match("em\u{2014}dash"), "em-dash");
    }

    // ---- strip_bom ----

    #[test]
    fn strip_bom_present() {
        let content = "\u{FEFF}hello";
        let (bom, text) = strip_bom(content);
        assert_eq!(bom, "\u{FEFF}");
        assert_eq!(text, "hello");
    }

    #[test]
    fn strip_bom_absent() {
        let content = "hello";
        let (bom, text) = strip_bom(content);
        assert_eq!(bom, "");
        assert_eq!(text, "hello");
    }

    // ---- fuzzy_find_text ----

    #[test]
    fn fuzzy_exact_match() {
        let result = fuzzy_find_text("hello world", "world");
        assert!(result.found);
        assert!(!result.used_fuzzy_match);
        assert_eq!(result.index, 6);
        assert_eq!(result.match_length, 5);
    }

    #[test]
    fn fuzzy_no_match() {
        let result = fuzzy_find_text("hello world", "xyz");
        assert!(!result.found);
    }

    #[test]
    fn fuzzy_match_with_smart_quotes() {
        // Content uses straight quotes; old_text uses smart quotes → fuzzy match
        let content = "say 'hello'";
        let old_text = "say \u{2018}hello\u{2019}";
        let result = fuzzy_find_text(content, old_text);
        assert!(result.found);
        assert!(result.used_fuzzy_match);
    }

    // ---- generate_diff_string ----

    #[test]
    fn diff_single_line_change() {
        let old = "line1\nline2\nline3";
        let new = "line1\nLINE2\nline3";
        let result = generate_diff_string(old, new, None);
        assert!(result.diff.contains("-"));
        assert!(result.diff.contains("+"));
        assert!(result.first_changed_line.is_some());
    }

    #[test]
    fn diff_no_change_empty_diff() {
        let content = "same content";
        let result = generate_diff_string(content, content, None);
        // No +/- lines — only context (or empty).
        assert!(!result.diff.contains('+'));
        assert!(!result.diff.contains('-'));
        assert!(result.first_changed_line.is_none());
    }

    #[test]
    fn diff_added_lines() {
        let old = "line1\nline3";
        let new = "line1\nline2\nline3";
        let result = generate_diff_string(old, new, None);
        assert!(result.diff.contains('+'));
        assert!(result.first_changed_line == Some(2));
    }

    #[test]
    fn diff_removed_lines() {
        let old = "line1\nline2\nline3";
        let new = "line1\nline3";
        let result = generate_diff_string(old, new, None);
        assert!(result.diff.contains('-'));
    }

    // ---- compute_edit_diff ----

    #[test]
    fn compute_diff_file_not_found() {
        let result = compute_edit_diff("/nonexistent/file.txt", "old", "new", "/cwd");
        assert!(result.is_err());
        assert!(result.unwrap_err().error.contains("not found"));
    }

    #[test]
    fn compute_diff_text_not_found() {
        use std::io::Write;
        use tempfile::NamedTempFile;

        let mut file = NamedTempFile::new().unwrap();
        writeln!(file, "hello world").unwrap();
        let path = file.path().to_str().unwrap().to_string();

        let result = compute_edit_diff(&path, "xyz", "abc", "/");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.error.contains("Could not find") || err.error.contains("not found"));
    }

    #[test]
    fn compute_diff_success() {
        use std::io::Write;
        use tempfile::NamedTempFile;

        let mut file = NamedTempFile::new().unwrap();
        writeln!(file, "foo\nbar\nbaz").unwrap();
        let path = file.path().to_str().unwrap().to_string();

        let result = compute_edit_diff(&path, "bar", "BAR", "/");
        assert!(result.is_ok());
        let diff = result.unwrap();
        assert!(diff.diff.contains('+') || diff.diff.contains('-'));
    }
}
