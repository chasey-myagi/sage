//! Utility helpers — Rust counterparts of `packages/ai/src/utils/*.ts`.
//!
//! These modules are 1:1 translations of pi-mono's utility helpers, adapted
//! to sage's type system where applicable:
//!
//! | pi-mono (`utils/*.ts`)     | sage (`utils/*.rs`)        | Status                              |
//! |----------------------------|----------------------------|-------------------------------------|
//! | `overflow.ts`              | `overflow.rs`              | Port — regex patterns + API         |
//! | `sanitize-unicode.ts`      | `sanitize_unicode.rs`      | Port — unpaired surrogate strip     |
//! | `validation.ts`            | `validation.rs`            | Port — tool-arg JSON Schema check   |
//! | `hash.ts`                  | `hash.rs`                  | Port — `short_hash` (MurmurHash2)   |
//! | `json-parse.ts`            | `json_parse.rs`            | Port — streaming JSON repair        |
//! | `event-stream.ts`          | `event_stream.rs`          | Port — SSE bytes → Stream<Event>    |
//! | `typebox-helpers.ts`       | `typebox_helpers.rs`       | Port — schema builders + validate   |

pub mod event_stream;
pub mod hash;
pub mod json_parse;
pub mod oauth;
pub mod overflow;
pub mod sanitize_unicode;
pub mod typebox_helpers;
pub mod validation;
