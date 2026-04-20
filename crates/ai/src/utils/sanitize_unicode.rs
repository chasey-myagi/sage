//! Unicode surrogate sanitization — port of `utils/sanitize-unicode.ts`.
//!
//! Rust `String`s are UTF-8 so true unpaired surrogates (U+D800–U+DFFF) cannot
//! appear in a well-formed `str`: they are invalid Unicode scalar values and
//! would have been rejected at construction time.  This function still has two
//! useful jobs:
//!
//! 1. **Strip literal `\uD800`-style escape sequences** that arrived via JSON
//!    when they form unpaired surrogates. pi-mono's regex runs against the
//!    decoded string (where UTF-16 surrogate code units are unambiguous); we
//!    run against the serialised escape form because that's where sage sees
//!    the problem — serialising a broken `\uXXXX` escape back to a provider
//!    that subsequently parses it as UTF-16 would blow up.
//! 2. **Replace any `U+FFFD` that was left behind** by UTF-8 lossy decode
//!    (Anthropic, Gemini reject it as malformed).
//!
//! The scanner walks the string character by character; no regex look-ahead is
//! needed (the Rust `regex` crate doesn't support look-around).

/// Remove unpaired Unicode surrogate escapes from `text`.
///
/// Valid emoji and other BMP-external characters are preserved unchanged; only
/// genuinely unpaired `\uXXXX` escapes and stray `U+FFFD` are stripped.
pub fn sanitize_surrogates(text: &str) -> String {
    let bytes = text.as_bytes();
    let len = bytes.len();
    let mut out = String::with_capacity(len);
    let mut i = 0;

    while i < len {
        let b = bytes[i];

        // Fast path: ASCII byte that is not the start of a `\uXXXX` escape.
        if b != b'\\' {
            // Walk forward over a full UTF-8 scalar and copy it.
            // SAFETY: input is &str so boundaries are valid.
            let remaining = &text[i..];
            let ch = remaining.chars().next().unwrap();
            if ch != '\u{FFFD}' {
                out.push(ch);
            }
            i += ch.len_utf8();
            continue;
        }

        // `b == '\\'` — check if this starts a `\uXXXX` escape.
        if let Some(code) = parse_u_escape(bytes, i) {
            // High surrogate? Peek ahead 6 bytes for a low-surrogate escape.
            if (0xD800..=0xDBFF).contains(&code) {
                if let Some(low) = parse_u_escape(bytes, i + 6) {
                    if (0xDC00..=0xDFFF).contains(&low) {
                        // Paired — keep both, advance by 12.
                        out.push_str(&text[i..i + 12]);
                        i += 12;
                        continue;
                    }
                }
                // Unpaired high — drop it.
                i += 6;
                continue;
            }
            if (0xDC00..=0xDFFF).contains(&code) {
                // Unpaired low (any valid pair would have been consumed above).
                i += 6;
                continue;
            }
            // Non-surrogate \uXXXX — pass through.
            out.push_str(&text[i..i + 6]);
            i += 6;
            continue;
        }

        // Lone backslash — pass through.
        out.push('\\');
        i += 1;
    }

    out
}

/// Parse `\uXXXX` starting at byte `start` in `bytes`.  Returns the 16-bit
/// code-unit value, or `None` if the 6 bytes at `start` don't form a valid
/// escape.
fn parse_u_escape(bytes: &[u8], start: usize) -> Option<u32> {
    if start + 6 > bytes.len() {
        return None;
    }
    if bytes[start] != b'\\' || (bytes[start + 1] != b'u' && bytes[start + 1] != b'U') {
        return None;
    }
    let hex = &bytes[start + 2..start + 6];
    let mut n: u32 = 0;
    for &b in hex {
        let d = match b {
            b'0'..=b'9' => b - b'0',
            b'a'..=b'f' => b - b'a' + 10,
            b'A'..=b'F' => b - b'A' + 10,
            _ => return None,
        };
        n = (n << 4) | u32::from(d);
    }
    Some(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_ascii() {
        assert_eq!(sanitize_surrogates("Hello, world"), "Hello, world");
    }

    #[test]
    fn preserves_valid_emoji() {
        // 🙈 = U+1F648 — a single Rust scalar.  Must pass through unchanged.
        assert_eq!(sanitize_surrogates("Hello 🙈 World"), "Hello 🙈 World");
    }

    #[test]
    fn preserves_multi_script() {
        assert_eq!(
            sanitize_surrogates("你好 🌍 Здравствуй مرحبا"),
            "你好 🌍 Здравствуй مرحبا"
        );
    }

    #[test]
    fn strips_unpaired_high_surrogate_escape() {
        let input = r"Text \uD83D here";
        assert_eq!(sanitize_surrogates(input), "Text  here");
    }

    #[test]
    fn strips_unpaired_low_surrogate_escape() {
        let input = r"Text \uDC00 here";
        assert_eq!(sanitize_surrogates(input), "Text  here");
    }

    #[test]
    fn preserves_paired_surrogate_escape() {
        // "\uD83D\uDE48" is the monkey emoji — both halves must survive.
        let input = r"emoji \uD83D\uDE48 ok";
        assert_eq!(sanitize_surrogates(input), input);
    }

    #[test]
    fn mixed_paired_and_unpaired() {
        // \uD83D\uDE48 (paired) + \uD800 (unpaired) + ascii
        let input = r"a \uD83D\uDE48 b \uD800 c";
        assert_eq!(sanitize_surrogates(input), r"a \uD83D\uDE48 b  c");
    }

    #[test]
    fn non_surrogate_escape_passthrough() {
        // \u00E9 (é) is outside surrogate range — must be kept.
        let input = r"caf\u00E9";
        assert_eq!(sanitize_surrogates(input), input);
    }

    #[test]
    fn strips_replacement_char() {
        let input = "lost\u{FFFD}char";
        assert_eq!(sanitize_surrogates(input), "lostchar");
    }

    #[test]
    fn lone_backslash_passthrough() {
        assert_eq!(sanitize_surrogates(r"path C:\foo"), r"path C:\foo");
    }

    #[test]
    fn empty_string() {
        assert_eq!(sanitize_surrogates(""), "");
    }

    #[test]
    fn uppercase_u_escape_not_recognised_as_surrogate() {
        // JSON \u escapes are always lowercase u; uppercase \U is treated as a
        // stray sequence we don't decode.  Test confirms we don't *over*-strip.
        // Actually our parser accepts both; confirm surrogate handling still works.
        let input = r"x \UD800 y";
        // Parsed as unpaired high surrogate → dropped.
        assert_eq!(sanitize_surrogates(input), "x  y");
    }

    #[test]
    fn broken_escape_passthrough() {
        // "\uZZZZ" — malformed. We should not panic; copy through as ASCII.
        let input = r"x \uZZZZ y";
        let out = sanitize_surrogates(input);
        // Backslash + "u" + "Z" etc pass through as-is.
        assert_eq!(out, input);
    }

    // --- LinkedIn 真实场景（来自 pi-mono unicode-surrogate.test.ts）----------

    #[test]
    fn real_world_linkedin_emoji_preserved() {
        // 正确的 emoji（已经是 Rust UTF-8 scalar）应完全保留
        let input = "Mario Zechner wann? Wo? Bin grad äußersr eventuninformiert 🙈";
        assert_eq!(sanitize_surrogates(input), input);
    }

    #[test]
    fn multiple_valid_emoji_preserved() {
        let input = "🙈 👍 ❤️ 🤔 🚀";
        assert_eq!(sanitize_surrogates(input), input);
    }

    #[test]
    fn japanese_chinese_mixed_preserved() {
        let input = "こんにちは 你好 ∑∫∂√ \"curly\" 'quotes'";
        assert_eq!(sanitize_surrogates(input), input);
    }

    // --- 多个 unpaired surrogates -------------------------------------------

    #[test]
    fn multiple_unpaired_surrogates_all_stripped() {
        // 三个独立 unpaired high surrogates
        let input = r"\uD800\uD801\uD802 text";
        let out = sanitize_surrogates(input);
        // All three removed; "text" remains
        assert!(out.contains("text"), "text should be preserved");
        assert!(!out.contains("\\uD80"), "surrogates should be stripped");
    }

    #[test]
    fn alternating_paired_unpaired() {
        // paired + unpaired + paired
        // \uD83D\uDE48 (paired 🙈), \uD800 (unpaired), \uD83D\uDE48 (paired 🙈)
        let input = r"\uD83D\uDE48 between \uD800 and \uD83D\uDE48";
        let out = sanitize_surrogates(input);
        // Both paired surrogates should survive; the lone \uD800 should be stripped
        assert!(out.contains(r"\uD83D\uDE48"), "paired surrogates should be kept");
        assert!(!out.contains(r"\uD800 "), "lone high surrogate should be removed");
    }

    // --- truncated escape at end of string ----------------------------------

    #[test]
    fn truncated_escape_at_end_passthrough() {
        // "\uD8" — too short to be a complete \uXXXX escape
        let input = r"text \uD8";
        let out = sanitize_surrogates(input);
        // Should not panic; pass through as-is
        assert!(out.contains("text"));
    }

    // --- empty and single-char inputs ----------------------------------------

    #[test]
    fn single_ascii_char() {
        assert_eq!(sanitize_surrogates("A"), "A");
    }

    #[test]
    fn newline_tab_preserved() {
        let input = "line1\nline2\ttabbed";
        assert_eq!(sanitize_surrogates(input), input);
    }

    // --- mathematical / special Unicode symbols ------------------------------

    #[test]
    fn mathematical_symbols_preserved() {
        let input = "∑∫∂√ α β γ δ";
        assert_eq!(sanitize_surrogates(input), input);
    }

    // --- curly quotes and special punctuation --------------------------------

    #[test]
    fn curly_quotes_preserved() {
        let input = "\u{201C}curly\u{201D} \u{2018}quotes\u{2019}";
        assert_eq!(sanitize_surrogates(input), input);
    }

    // --- idempotency ---------------------------------------------------------

    #[test]
    fn sanitize_is_idempotent() {
        let input = r"text \uD83D\uDE48 end \uD800 more";
        let once = sanitize_surrogates(input);
        let twice = sanitize_surrogates(&once);
        assert_eq!(once, twice, "sanitize should be idempotent");
    }

    // --- large string doesn't panic -----------------------------------------

    #[test]
    fn large_string_no_panic() {
        // 100KB of ASCII + a few unpaired surrogates
        let mut big = "hello world ".repeat(8_000);
        big.push_str(r"\uD800 \uD83D\uDE48 end");
        let out = sanitize_surrogates(&big);
        assert!(out.contains("hello world"));
        // The paired surrogate escape should survive
        assert!(out.contains(r"\uD83D\uDE48"));
    }

    // --- non-surrogate \uXXXX escapes (BMP chars) ---------------------------

    #[test]
    fn bmp_unicode_escapes_passthrough() {
        // \u0041 = 'A', \u0042 = 'B' — both well within non-surrogate range
        let input = r"\u0041\u0042";
        let out = sanitize_surrogates(input);
        assert_eq!(out, input);
    }

    #[test]
    fn high_bmp_escape_passthrough() {
        // \uD7FF — just below surrogate range, should be kept
        let input = r"\uD7FF";
        let out = sanitize_surrogates(input);
        assert_eq!(out, input);
    }

    #[test]
    fn low_private_use_area_passthrough() {
        // \uE000 — just above surrogate range, should be kept
        let input = r"\uE000";
        let out = sanitize_surrogates(input);
        assert_eq!(out, input);
    }
}
