// Bun-equivalent runtime registration helpers.
//
// In pi-mono, `bun/register-bedrock.ts` intercepts module loading at runtime.
// The Rust port replaces that mechanism with an explicit credential discovery
// function that callers invoke before constructing an AWS client.

pub mod register_bedrock;

pub use register_bedrock::{discover_aws_credentials, AwsCredentials, CredentialError};
