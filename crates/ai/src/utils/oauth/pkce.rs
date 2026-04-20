//! PKCE utilities — Rust counterpart of `packages/ai/src/utils/oauth/pkce.ts`.
//!
//! Generates PKCE code verifier and challenge using SHA-256.
//! Equivalent to the Web Crypto API implementation in the TypeScript original.

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use rand::RngCore;
use sha2::{Digest, Sha256};

/// PKCE verifier + challenge pair.
pub struct Pkce {
    pub verifier: String,
    pub challenge: String,
}

/// Generate a PKCE verifier (32 random bytes, base64url-encoded) and the
/// corresponding SHA-256 challenge.
///
/// Mirrors `generatePKCE()` from `pkce.ts`.
pub fn generate_pkce() -> Pkce {
    // 32 random bytes → base64url verifier
    let mut verifier_bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut verifier_bytes);
    let verifier = URL_SAFE_NO_PAD.encode(verifier_bytes);

    // SHA-256(verifier) → base64url challenge
    let mut hasher = Sha256::new();
    hasher.update(verifier.as_bytes());
    let hash = hasher.finalize();
    let challenge = URL_SAFE_NO_PAD.encode(hash);

    Pkce {
        verifier,
        challenge,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pkce_lengths_are_correct() {
        let pkce = generate_pkce();
        // 32 bytes base64url = 43 chars (no padding)
        assert_eq!(pkce.verifier.len(), 43);
        // SHA-256 = 32 bytes → 43 chars base64url
        assert_eq!(pkce.challenge.len(), 43);
    }

    #[test]
    fn pkce_verifier_and_challenge_differ() {
        let pkce = generate_pkce();
        assert_ne!(pkce.verifier, pkce.challenge);
    }

    #[test]
    fn pkce_challenge_is_sha256_of_verifier() {
        let pkce = generate_pkce();
        let mut hasher = Sha256::new();
        hasher.update(pkce.verifier.as_bytes());
        let expected = URL_SAFE_NO_PAD.encode(hasher.finalize());
        assert_eq!(pkce.challenge, expected);
    }
}
