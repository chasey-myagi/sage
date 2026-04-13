// Google Vertex AI Provider — streams completions via Vertex AI's SSE API.
//
// Endpoint:
//   https://{location}-aiplatform.googleapis.com/v1/projects/{project}/locations/{location}/publishers/google/models/{model}:streamGenerateContent?alt=sse
//
// Auth: Vertex API key appended as `&key={key}`.
//
// NOTE: ADC (Application Default Credentials) is not yet implemented. Pi-mono
// uses the @google/genai SDK which handles OAuth2 token refresh automatically;
// a Rust equivalent would require `google-cloud-auth` or manual metadata server
// integration. For now, only API key auth is supported.
//
// Reuses request/response format from google.rs (same wire format).

use crate::llm::registry::{ApiProvider, StreamOptions};
use crate::llm::types::*;
use async_trait::async_trait;
use reqwest::Client;

use super::google::{build_google_request_body, read_google_sse_stream};

const API_VERSION: &str = "v1";

// ---------------------------------------------------------------------------
// GoogleVertexProvider
// ---------------------------------------------------------------------------

#[derive(Default)]
pub struct GoogleVertexProvider {
    client: Client,
}

impl GoogleVertexProvider {
    pub fn new() -> Self {
        Self::default()
    }
}

// ---------------------------------------------------------------------------
// Auth & config resolution
// ---------------------------------------------------------------------------

/// Check if a string looks like a placeholder API key (e.g. `<your-key>`).
/// Matches pi-mono's `/^<[^>]+>$/` — requires at least one non-`>` char inside.
fn is_placeholder_api_key(key: &str) -> bool {
    key.starts_with('<')
        && key.ends_with('>')
        && key.len() > 2
        && key[1..key.len() - 1].chars().all(|c| c != '>')
}

/// Resolve Vertex AI API key from options or GOOGLE_CLOUD_API_KEY env var.
/// Returns `None` if no valid key is available.
pub(crate) fn resolve_api_key(options_key: Option<&str>) -> Option<String> {
    let key = options_key
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .or_else(|| {
            std::env::var("GOOGLE_CLOUD_API_KEY")
                .ok()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
        });

    key.filter(|k| !is_placeholder_api_key(k))
}

/// Resolve the GCP project ID from environment variables.
pub(crate) fn resolve_project() -> Result<String, String> {
    if let Ok(p) = std::env::var("GOOGLE_CLOUD_PROJECT") {
        if !p.is_empty() {
            return Ok(p);
        }
    }
    if let Ok(p) = std::env::var("GCLOUD_PROJECT") {
        if !p.is_empty() {
            return Ok(p);
        }
    }
    Err(
        "Vertex AI requires a project ID. Set GOOGLE_CLOUD_PROJECT or GCLOUD_PROJECT."
            .into(),
    )
}

/// Resolve the GCP location from environment variables.
pub(crate) fn resolve_location() -> Result<String, String> {
    if let Ok(l) = std::env::var("GOOGLE_CLOUD_LOCATION") {
        if !l.is_empty() {
            return Ok(l);
        }
    }
    Err("Vertex AI requires a location. Set GOOGLE_CLOUD_LOCATION.".into())
}

/// Build the Vertex AI streaming URL.
///
/// Format: `{base}/v1/projects/{project}/locations/{location}/publishers/google/models/{model}:streamGenerateContent?alt=sse[&key={key}]`
///
/// The API key is appended as a query parameter. Google API keys are
/// alphanumeric with hyphens/underscores, so URL-encoding is not needed.
pub(crate) fn build_url(
    base_url: &str,
    location: &str,
    project: &str,
    model_id: &str,
    api_key: Option<&str>,
) -> String {
    let base = if base_url.is_empty() {
        format!("https://{location}-aiplatform.googleapis.com")
    } else {
        base_url.trim_end_matches('/').to_string()
    };

    let path = format!(
        "{base}/{API_VERSION}/projects/{project}/locations/{location}/publishers/google/models/{model_id}:streamGenerateContent?alt=sse"
    );

    match api_key {
        Some(key) => format!("{path}&key={key}"),
        None => path,
    }
}

// ---------------------------------------------------------------------------
// ApiProvider implementation
// ---------------------------------------------------------------------------

#[async_trait]
impl ApiProvider for GoogleVertexProvider {
    fn api(&self) -> &str {
        "google-vertex"
    }

    async fn stream(
        &self,
        model: &Model,
        context: &LlmContext,
        tools: &[LlmTool],
        options: &StreamOptions,
    ) -> Vec<AssistantMessageEvent> {
        // Resolve API key (only auth method currently supported)
        let api_key = match resolve_api_key(options.api_key.as_deref()) {
            Some(key) => key,
            None => {
                return vec![AssistantMessageEvent::Error(
                    "Vertex AI API key not found. Set GOOGLE_CLOUD_API_KEY. \
                     (ADC/OAuth2 auth is not yet supported.)"
                        .into(),
                )];
            }
        };

        // Resolve project and location from environment
        let project = match resolve_project() {
            Ok(p) => p,
            Err(e) => {
                return vec![AssistantMessageEvent::Error(e)];
            }
        };

        let location = match resolve_location() {
            Ok(l) => l,
            Err(e) => {
                return vec![AssistantMessageEvent::Error(e)];
            }
        };

        // Build URL
        let url = build_url(
            &model.base_url,
            &location,
            &project,
            &model.id,
            Some(&api_key),
        );

        // Build request body (shared with Google AI)
        let body = build_google_request_body(model, context, tools, options);

        // Build the HTTP request
        let mut req = self
            .client
            .post(&url)
            .header("Content-Type", "application/json");

        // Extra headers from model config + options
        for (k, v) in &model.headers {
            req = req.header(k.as_str(), v.as_str());
        }
        for (k, v) in &options.headers {
            req = req.header(k.as_str(), v.as_str());
        }

        let response = match req.json(&body).send().await {
            Ok(resp) => resp,
            Err(e) => {
                return vec![AssistantMessageEvent::Error(format!(
                    "HTTP request failed: {e}"
                ))];
            }
        };

        if !response.status().is_success() {
            let status = response.status();
            let body_text = response.text().await.unwrap_or_default();
            return vec![AssistantMessageEvent::Error(format!(
                "Vertex AI error {status}: {body_text}"
            ))];
        }

        // Parse SSE stream (shared with Google AI)
        read_google_sse_stream(response).await
    }
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::registry::StreamOptions;
    use serde_json::json;

    // -----------------------------------------------------------------------
    // GoogleVertexProvider — constructor & api()
    // -----------------------------------------------------------------------

    #[test]
    fn test_vertex_provider_api_name() {
        let provider = GoogleVertexProvider::new();
        assert_eq!(provider.api(), "google-vertex");
    }

    #[test]
    fn test_vertex_provider_default() {
        let provider = GoogleVertexProvider::default();
        assert_eq!(provider.api(), "google-vertex");
    }

    // -----------------------------------------------------------------------
    // resolve_api_key
    // -----------------------------------------------------------------------

    #[test]
    fn test_resolve_api_key_from_options() {
        let key = resolve_api_key(Some("my-vertex-key"));
        assert_eq!(key, Some("my-vertex-key".into()));
    }

    #[test]
    fn test_resolve_api_key_empty_options_falls_through() {
        unsafe { std::env::remove_var("GOOGLE_CLOUD_API_KEY") };
        let key = resolve_api_key(Some(""));
        assert_eq!(key, None);
    }

    #[test]
    fn test_resolve_api_key_whitespace_only() {
        unsafe { std::env::remove_var("GOOGLE_CLOUD_API_KEY") };
        let key = resolve_api_key(Some("   "));
        assert_eq!(key, None);
    }

    #[test]
    fn test_resolve_api_key_none_no_env() {
        unsafe { std::env::remove_var("GOOGLE_CLOUD_API_KEY") };
        let key = resolve_api_key(None);
        assert_eq!(key, None);
    }

    #[test]
    fn test_resolve_api_key_placeholder_rejected() {
        let key = resolve_api_key(Some("<your-api-key>"));
        assert_eq!(key, None);
    }

    // -----------------------------------------------------------------------
    // is_placeholder_api_key — matches pi-mono's /^<[^>]+>$/
    // -----------------------------------------------------------------------

    #[test]
    fn test_placeholder_key_valid_cases() {
        assert!(is_placeholder_api_key("<key>"));
        assert!(is_placeholder_api_key("<YOUR_API_KEY>"));
        assert!(is_placeholder_api_key("<your-key-here>"));
    }

    #[test]
    fn test_placeholder_key_invalid_cases() {
        assert!(!is_placeholder_api_key("real-key-abc123"));
        assert!(!is_placeholder_api_key("<partial"));
        assert!(!is_placeholder_api_key("partial>"));
        // Empty angle brackets — pi-mono's regex requires [^>]+ (at least one char)
        assert!(!is_placeholder_api_key("<>"));
    }

    // -----------------------------------------------------------------------
    // resolve_project
    // -----------------------------------------------------------------------

    #[test]
    fn test_resolve_project_from_env() {
        unsafe { std::env::set_var("GOOGLE_CLOUD_PROJECT", "env-project") };
        let project = resolve_project();
        assert_eq!(project.unwrap(), "env-project");
        unsafe { std::env::remove_var("GOOGLE_CLOUD_PROJECT") };
    }

    #[test]
    fn test_resolve_project_fallback_gcloud_project() {
        unsafe { std::env::remove_var("GOOGLE_CLOUD_PROJECT") };
        unsafe { std::env::set_var("GCLOUD_PROJECT", "gcloud-proj") };
        let project = resolve_project();
        assert_eq!(project.unwrap(), "gcloud-proj");
        unsafe { std::env::remove_var("GCLOUD_PROJECT") };
    }

    #[test]
    fn test_resolve_project_missing_returns_error() {
        unsafe { std::env::remove_var("GOOGLE_CLOUD_PROJECT") };
        unsafe { std::env::remove_var("GCLOUD_PROJECT") };
        let result = resolve_project();
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("project ID"));
    }

    // -----------------------------------------------------------------------
    // resolve_location
    // -----------------------------------------------------------------------

    #[test]
    fn test_resolve_location_from_env() {
        unsafe { std::env::set_var("GOOGLE_CLOUD_LOCATION", "europe-west1") };
        let loc = resolve_location();
        assert_eq!(loc.unwrap(), "europe-west1");
        unsafe { std::env::remove_var("GOOGLE_CLOUD_LOCATION") };
    }

    #[test]
    fn test_resolve_location_missing_returns_error() {
        unsafe { std::env::remove_var("GOOGLE_CLOUD_LOCATION") };
        let result = resolve_location();
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("location"));
    }

    // -----------------------------------------------------------------------
    // build_url
    // -----------------------------------------------------------------------

    #[test]
    fn test_build_url_with_api_key() {
        let url = build_url(
            "",
            "us-central1",
            "my-project",
            "gemini-2.5-pro",
            Some("test-key"),
        );
        assert_eq!(
            url,
            "https://us-central1-aiplatform.googleapis.com/v1/projects/my-project/locations/us-central1/publishers/google/models/gemini-2.5-pro:streamGenerateContent?alt=sse&key=test-key"
        );
    }

    #[test]
    fn test_build_url_without_api_key() {
        let url = build_url(
            "",
            "europe-west4",
            "prod-project",
            "gemini-2.0-flash",
            None,
        );
        assert_eq!(
            url,
            "https://europe-west4-aiplatform.googleapis.com/v1/projects/prod-project/locations/europe-west4/publishers/google/models/gemini-2.0-flash:streamGenerateContent?alt=sse"
        );
    }

    #[test]
    fn test_build_url_custom_base_url() {
        let url = build_url(
            "https://custom-proxy.example.com/",
            "us-east1",
            "proj",
            "gemini-2.5-flash",
            None,
        );
        assert!(url.starts_with("https://custom-proxy.example.com/v1/projects/"));
        assert!(!url.contains("aiplatform.googleapis.com"));
    }

    #[test]
    fn test_build_url_base_url_trailing_slash_stripped() {
        let url1 = build_url(
            "https://proxy.example.com/",
            "loc",
            "proj",
            "model",
            None,
        );
        let url2 = build_url(
            "https://proxy.example.com",
            "loc",
            "proj",
            "model",
            None,
        );
        assert_eq!(url1, url2);
    }

    // -----------------------------------------------------------------------
    // build_google_request_body (shared function, tested via Vertex models)
    // -----------------------------------------------------------------------

    fn make_test_model(id: &str, reasoning: bool) -> Model {
        Model {
            id: id.into(),
            name: "test".into(),
            api: api::GOOGLE_VERTEX.into(),
            provider: provider::GOOGLE_VERTEX.into(),
            base_url: String::new(),
            api_key_env: String::new(),
            reasoning,
            input: vec![InputType::Text],
            max_tokens: 8192,
            context_window: 1048576,
            cost: ModelCost {
                input_per_million: 0.0,
                output_per_million: 0.0,
                cache_read_per_million: 0.0,
                cache_write_per_million: 0.0,
            },
            headers: vec![],
            compat: None,
        }
    }

    #[test]
    fn test_build_request_body_basic() {
        let model = make_test_model("gemini-2.5-pro", false);
        let context = LlmContext {
            messages: vec![LlmMessage::User {
                content: vec![LlmContent::Text("Hello".into())],
            }],
            system_prompt: "You are helpful.".into(),
            max_tokens: 4096,
            temperature: None,
        };
        let options = StreamOptions::default();

        let body = build_google_request_body(&model, &context, &[], &options);

        assert!(body["contents"].is_array());
        assert_eq!(body["contents"].as_array().unwrap().len(), 1);
        assert_eq!(
            body["systemInstruction"]["parts"][0]["text"],
            "You are helpful."
        );
        assert_eq!(body["generationConfig"]["maxOutputTokens"], 4096);
        assert!(body.get("tools").is_none());
    }

    #[test]
    fn test_build_request_body_with_tools() {
        let model = make_test_model("gemini-2.0-flash", false);
        let context = LlmContext {
            messages: vec![LlmMessage::User {
                content: vec![LlmContent::Text("Run ls".into())],
            }],
            system_prompt: String::new(),
            max_tokens: 4096,
            temperature: None,
        };
        let tools = vec![LlmTool {
            name: "bash".into(),
            description: "Run a command".into(),
            parameters: json!({"type": "object", "properties": {"command": {"type": "string"}}}),
        }];
        let options = StreamOptions::default();

        let body = build_google_request_body(&model, &context, &tools, &options);
        assert!(body["tools"].is_array());
        let decls = body["tools"][0]["functionDeclarations"]
            .as_array()
            .unwrap();
        assert_eq!(decls.len(), 1);
        assert_eq!(decls[0]["name"], "bash");
    }

    #[test]
    fn test_build_request_body_with_temperature() {
        let model = make_test_model("gemini-2.5-flash", false);
        let context = LlmContext {
            messages: vec![],
            system_prompt: String::new(),
            max_tokens: 4096,
            temperature: Some(0.7),
        };
        let options = StreamOptions::default();

        let body = build_google_request_body(&model, &context, &[], &options);
        let temp = body["generationConfig"]["temperature"]
            .as_f64()
            .unwrap();
        assert!((temp - 0.7).abs() < 0.001, "expected ~0.7, got {temp}");
    }

    #[test]
    fn test_build_request_body_thinking_enabled_gemini3() {
        let model = make_test_model("gemini-3-pro-preview", true);
        let context = LlmContext {
            messages: vec![],
            system_prompt: String::new(),
            max_tokens: 65536,
            temperature: None,
        };
        let options = StreamOptions {
            thinking_enabled: Some(true),
            reasoning: Some(ReasoningLevel::High),
            ..Default::default()
        };

        let body = build_google_request_body(&model, &context, &[], &options);
        let tc = &body["generationConfig"]["thinkingConfig"];
        assert_eq!(tc["includeThoughts"], true);
        assert_eq!(tc["thinkingLevel"], "HIGH");
    }

    #[test]
    fn test_build_request_body_thinking_enabled_gemini25() {
        let model = make_test_model("gemini-2.5-pro", true);
        let context = LlmContext {
            messages: vec![],
            system_prompt: String::new(),
            max_tokens: 65536,
            temperature: None,
        };
        let options = StreamOptions {
            thinking_enabled: Some(true),
            reasoning: Some(ReasoningLevel::Medium),
            ..Default::default()
        };

        let body = build_google_request_body(&model, &context, &[], &options);
        let tc = &body["generationConfig"]["thinkingConfig"];
        assert_eq!(tc["includeThoughts"], true);
        assert_eq!(tc["thinkingBudget"], 8192);
    }

    #[test]
    fn test_build_request_body_thinking_disabled_reasoning_model() {
        let model = make_test_model("gemini-2.5-flash", true);
        let context = LlmContext {
            messages: vec![],
            system_prompt: String::new(),
            max_tokens: 65536,
            temperature: None,
        };
        let options = StreamOptions::default(); // thinking not enabled

        let body = build_google_request_body(&model, &context, &[], &options);
        let tc = &body["generationConfig"]["thinkingConfig"];
        assert_eq!(tc["thinkingBudget"], 0);
    }

    #[test]
    fn test_build_request_body_no_system_prompt() {
        let model = make_test_model("gemini-2.0-flash", false);
        let context = LlmContext {
            messages: vec![LlmMessage::User {
                content: vec![LlmContent::Text("hi".into())],
            }],
            system_prompt: String::new(),
            max_tokens: 4096,
            temperature: None,
        };
        let options = StreamOptions::default();

        let body = build_google_request_body(&model, &context, &[], &options);
        assert!(body.get("systemInstruction").is_none());
    }

    #[test]
    fn test_build_request_body_options_max_tokens_override() {
        let model = make_test_model("gemini-2.0-flash", false);
        let context = LlmContext {
            messages: vec![],
            system_prompt: String::new(),
            max_tokens: 4096,
            temperature: None,
        };
        let options = StreamOptions {
            max_tokens: Some(2048),
            ..Default::default()
        };

        let body = build_google_request_body(&model, &context, &[], &options);
        assert_eq!(body["generationConfig"]["maxOutputTokens"], 2048);
    }

    #[test]
    fn test_build_request_body_options_temperature_overrides_context() {
        let model = make_test_model("gemini-2.0-flash", false);
        let context = LlmContext {
            messages: vec![],
            system_prompt: String::new(),
            max_tokens: 4096,
            temperature: Some(0.5),
        };
        let options = StreamOptions {
            temperature: Some(0.9),
            ..Default::default()
        };

        let body = build_google_request_body(&model, &context, &[], &options);
        let temp = body["generationConfig"]["temperature"]
            .as_f64()
            .unwrap();
        assert!((temp - 0.9).abs() < 0.001, "expected ~0.9, got {temp}");
    }

    #[test]
    fn test_build_request_body_custom_thinking_budget() {
        let model = make_test_model("gemini-2.5-pro", true);
        let context = LlmContext {
            messages: vec![],
            system_prompt: String::new(),
            max_tokens: 65536,
            temperature: None,
        };
        let options = StreamOptions {
            thinking_enabled: Some(true),
            reasoning: Some(ReasoningLevel::High),
            thinking_budget_tokens: Some(16384),
            ..Default::default()
        };

        let body = build_google_request_body(&model, &context, &[], &options);
        let tc = &body["generationConfig"]["thinkingConfig"];
        assert_eq!(tc["thinkingBudget"], 16384);
    }
}
