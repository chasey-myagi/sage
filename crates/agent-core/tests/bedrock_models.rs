// Translated from pi-mono packages/agent/test/bedrock-models.test.ts
//
// These tests verify that the agent loop works with various Amazon Bedrock models.
// All tests require real AWS credentials and BEDROCK_EXTENSIVE_MODEL_TEST=1 env var,
// so all are marked #[ignore].
//
// Run with:
//   AWS_REGION=us-east-1 BEDROCK_EXTENSIVE_MODEL_TEST=1 cargo test -p agent-core --test bedrock_models -- --ignored

// =============================================================================
// Known Issue Categories (mirrored from TS)
// =============================================================================

/// Models that require inference profile ARN (not available on-demand in us-east-1)
const REQUIRES_INFERENCE_PROFILE: &[&str] = &[
    "anthropic.claude-3-5-haiku-20241022-v1:0",
    "anthropic.claude-3-5-sonnet-20241022-v2:0",
    "anthropic.claude-3-opus-20240229-v1:0",
    "meta.llama3-1-70b-instruct-v1:0",
    "meta.llama3-1-8b-instruct-v1:0",
];

/// Models with invalid identifiers (not available in us-east-1 or don't exist)
const INVALID_MODEL_ID: &[&str] = &[
    "deepseek.v3-v1:0",
    "eu.anthropic.claude-haiku-4-5-20251001-v1:0",
    "eu.anthropic.claude-opus-4-5-20251101-v1:0",
    "eu.anthropic.claude-sonnet-4-5-20250929-v1:0",
    "qwen.qwen3-235b-a22b-2507-v1:0",
    "qwen.qwen3-coder-480b-a35b-v1:0",
];

/// Models where our maxTokens config exceeds the model's actual limit
const MAX_TOKENS_EXCEEDED: &[&str] = &[
    "us.meta.llama4-maverick-17b-instruct-v1:0",
    "us.meta.llama4-scout-17b-instruct-v1:0",
];

fn is_model_unavailable(model_id: &str) -> bool {
    REQUIRES_INFERENCE_PROFILE.contains(&model_id)
        || INVALID_MODEL_ID.contains(&model_id)
        || MAX_TOKENS_EXCEEDED.contains(&model_id)
}

fn has_bedrock_credentials() -> bool {
    // Check if AWS credentials are available via environment or config
    std::env::var("AWS_ACCESS_KEY_ID").is_ok()
        || std::env::var("AWS_PROFILE").is_ok()
        || std::path::Path::new(&format!(
            "{}/.aws/credentials",
            std::env::var("HOME").unwrap_or_default()
        ))
        .exists()
}

// =============================================================================
// Tests
// =============================================================================

/// Translated from: Amazon Bedrock Models - Agent Loop / "skipped" branch
///
/// This test validates that the bedrock test suite is correctly guarded by
/// the BEDROCK_EXTENSIVE_MODEL_TEST env var. The actual per-model tests
/// require the full bedrock provider implementation.
#[test]
#[ignore = "requires AWS credentials and BEDROCK_EXTENSIVE_MODEL_TEST=1"]
fn bedrock_skipped_without_credentials_env() {
    let should_run =
        has_bedrock_credentials() && std::env::var("BEDROCK_EXTENSIVE_MODEL_TEST").is_ok();
    if !should_run {
        // This is the expected path — skipped in TS via it.skip()
        return;
    }
    panic!("If credentials are set, use the per-model tests below");
}

/// Translated from: Model: <id> / "should handle basic text prompt"
/// Requires real Bedrock connection to a non-unavailable model.
#[tokio::test]
#[ignore = "requires AWS credentials and BEDROCK_EXTENSIVE_MODEL_TEST=1"]
async fn bedrock_basic_text_prompt_non_unavailable_model() {
    assert!(
        has_bedrock_credentials(),
        "AWS credentials required for bedrock tests"
    );
    assert!(
        std::env::var("BEDROCK_EXTENSIVE_MODEL_TEST").is_ok(),
        "BEDROCK_EXTENSIVE_MODEL_TEST=1 required"
    );
    // Actual implementation would iterate allBedrockModels and test each.
    // Not implemented here because the full Bedrock provider is not yet wired
    // into agent-core tests. Add integration when BedrockProvider is available.
    todo!("Implement when BedrockProvider is wired into agent-core integration tests")
}

/// Translated from: Model: <id> / "should handle multi-turn conversation with thinking content in history"
/// Requires real Bedrock connection.
#[tokio::test]
#[ignore = "requires AWS credentials and BEDROCK_EXTENSIVE_MODEL_TEST=1"]
async fn bedrock_multi_turn_with_thinking_in_history() {
    assert!(
        has_bedrock_credentials(),
        "AWS credentials required for bedrock tests"
    );
    todo!("Implement when BedrockProvider is wired into agent-core integration tests")
}

/// Translated from: Model: <id> / "should handle conversation with synthetic thinking signature in history"
/// Requires real Bedrock connection.
#[tokio::test]
#[ignore = "requires AWS credentials and BEDROCK_EXTENSIVE_MODEL_TEST=1"]
async fn bedrock_synthetic_thinking_signature_in_history() {
    assert!(
        has_bedrock_credentials(),
        "AWS credentials required for bedrock tests"
    );
    todo!("Implement when BedrockProvider is wired into agent-core integration tests")
}

// =============================================================================
// Helper / classification unit tests (no network needed)
// =============================================================================

/// Validate the is_model_unavailable helper covers all known categories.
#[test]
fn bedrock_model_unavailable_classification() {
    // REQUIRES_INFERENCE_PROFILE
    assert!(is_model_unavailable(
        "anthropic.claude-3-5-haiku-20241022-v1:0"
    ));
    // INVALID_MODEL_ID
    assert!(is_model_unavailable("deepseek.v3-v1:0"));
    // MAX_TOKENS_EXCEEDED
    assert!(is_model_unavailable(
        "us.meta.llama4-maverick-17b-instruct-v1:0"
    ));
    // Available model
    assert!(!is_model_unavailable(
        "us.anthropic.claude-3-7-sonnet-20250219-v1:0"
    ));
}
