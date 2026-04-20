//! Fast deterministic hash — Rust port of pi-mono `utils/hash.ts`.
//!
//! Implements `short_hash`, a compact MurmurHash2-inspired mixing function that
//! produces a short alphanumeric string suitable for use in identifiers.  The
//! algorithm is a 1:1 translation of the TypeScript `shortHash` function so
//! that hashes produced by both sides match exactly.
//!
//! Note: JavaScript `Math.imul` performs **wrapping 32-bit integer
//! multiplication**.  Rust's `u32::wrapping_mul` is the exact equivalent.
//! JavaScript's `>>> 0` truncates to an unsigned 32-bit integer — this is
//! already guaranteed for `u32` in Rust.  JavaScript `>>> n` (unsigned right
//! shift) maps to Rust `u32 >> n` since `u32` is always unsigned.

/// Fast deterministic hash to shorten long strings.
///
/// Produces a compact lowercase alphanumeric string (base-36 encoding of two
/// 32-bit hash words concatenated: `h2_str + h1_str`).
///
/// ```rust
/// use ai::utils::hash::short_hash;
/// let h = short_hash("hello world");
/// assert!(!h.is_empty());
/// // same input always produces same output
/// assert_eq!(short_hash("hello"), short_hash("hello"));
/// ```
pub fn short_hash(s: &str) -> String {
    // Initial seeds from pi-mono `shortHash`.
    let mut h1: u32 = 0xdeadbeef;
    let mut h2: u32 = 0x41c6ce57;

    // Iterate over UTF-16 code units, matching JavaScript's `charCodeAt`.
    for ch in s.encode_utf16() {
        let ch = ch as u32;
        // JavaScript: h1 = Math.imul(h1 ^ ch, 2654435761)
        h1 = (h1 ^ ch).wrapping_mul(2654435761);
        // JavaScript: h2 = Math.imul(h2 ^ ch, 1597334677)
        h2 = (h2 ^ ch).wrapping_mul(1597334677);
    }

    // Final avalanche mixing — pi-mono uses sequential JS assignment:
    //   h1 = Math.imul(h1 ^ (h1 >>> 16), 2246822507) ^ Math.imul(h2 ^ (h2 >>> 13), 3266489909)
    //   h2 = Math.imul(h2 ^ (h2 >>> 16), 2246822507) ^ Math.imul(h1 ^ (h1 >>> 13), 3266489909)
    //
    // The second line uses the *new* h1 (already updated) for the h1 term.
    let new_h1 = (h1 ^ (h1 >> 16)).wrapping_mul(2246822507)
        ^ (h2 ^ (h2 >> 13)).wrapping_mul(3266489909);
    let new_h2 = (h2 ^ (h2 >> 16)).wrapping_mul(2246822507)
        ^ (new_h1 ^ (new_h1 >> 13)).wrapping_mul(3266489909);

    // JavaScript: (h2 >>> 0).toString(36) + (h1 >>> 0).toString(36)
    // (using the new values; `>>> 0` is a no-op for u32)
    format!("{}{}", to_base36(new_h2), to_base36(new_h1))
}

/// Convert a `u32` to its base-36 lowercase string representation.
///
/// Equivalent to JavaScript `(n >>> 0).toString(36)`.
fn to_base36(mut n: u32) -> String {
    if n == 0 {
        return "0".to_string();
    }
    const DIGITS: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    let mut buf = Vec::with_capacity(7);
    while n > 0 {
        buf.push(DIGITS[(n % 36) as usize]);
        n /= 36;
    }
    buf.reverse();
    String::from_utf8(buf).expect("base36 digits are ASCII")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_short_hash_is_deterministic() {
        assert_eq!(short_hash("hello"), short_hash("hello"));
        assert_eq!(short_hash(""), short_hash(""));
    }

    #[test]
    fn test_short_hash_different_inputs_differ() {
        assert_ne!(short_hash("hello"), short_hash("world"));
        assert_ne!(short_hash("abc"), short_hash("abd"));
    }

    #[test]
    fn test_short_hash_empty_string() {
        // Empty input: no character loop, seeds go straight to avalanche mixing.
        let h = short_hash("");
        assert!(!h.is_empty());
    }

    #[test]
    fn test_short_hash_is_base36() {
        let h = short_hash("test string 123 !@#");
        // base-36 output uses only lowercase letters and digits.
        assert!(
            h.chars().all(|c| c.is_ascii_digit() || c.is_ascii_lowercase()),
            "unexpected char in hash: {h}"
        );
    }

    #[test]
    fn test_short_hash_length_bound() {
        // Each base-36(u32) is at most 7 chars, combined ≤ 14.
        let h = short_hash("The quick brown fox jumps over the lazy dog");
        assert!(h.len() <= 14, "hash too long: {h}");
    }

    #[test]
    fn test_to_base36_zero() {
        assert_eq!(to_base36(0), "0");
    }

    #[test]
    fn test_to_base36_small_values() {
        // 35 → "z", 36 → "10"
        assert_eq!(to_base36(35), "z");
        assert_eq!(to_base36(36), "10");
    }

    /// Cross-validate against the TypeScript output for known inputs.
    ///
    /// Reference values produced by running the pi-mono `shortHash` function
    /// in Node.js with the same algorithm:
    ///
    /// ```js
    /// shortHash("hello")       // "1h6qa0qrowduu"
    /// shortHash("")            // "k4n83c7h0j2b"
    /// shortHash("hello world") // "n7rb4n1m39uz8"
    /// ```
    ///
    /// These pin the cross-language compatibility of the implementation.
    #[test]
    fn test_short_hash_cross_language_hello() {
        assert_eq!(short_hash("hello"), "1h6qa0qrowduu");
    }

    #[test]
    fn test_short_hash_cross_language_empty() {
        assert_eq!(short_hash(""), "k4n83c7h0j2b");
    }

    #[test]
    fn test_short_hash_cross_language_hello_world() {
        assert_eq!(short_hash("hello world"), "n7rb4n1m39uz8");
    }

    #[test]
    fn test_short_hash_cross_language_test_string() {
        // "test string 123 !@#"
        assert_eq!(short_hash("test string 123 !@#"), "lgie6b1kug5d3");
    }
}
