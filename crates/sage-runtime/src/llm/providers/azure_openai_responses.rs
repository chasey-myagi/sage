// Azure OpenAI Responses API Provider
// Wraps the OpenAI Responses protocol with Azure-specific auth, URL construction,
// and deployment name mapping.
//
// Pi-mono reference: providers/azure-openai-responses.ts
//
// Key differences from standard OpenAI Responses:
// - Auth: `api-key` header instead of `Authorization: Bearer`
// - URL: user-configured base URL (resource name → URL) with `/responses` suffix
// - Deployment name: maps model ID → deployment name via AZURE_OPENAI_DEPLOYMENT_NAME_MAP
// - No `store` field in request body
// - Base URL resolved from: AZURE_OPENAI_BASE_URL > AZURE_OPENAI_RESOURCE_NAME > model.base_url

use crate::llm::keys;
use crate::llm::registry::{ApiProvider, StreamOptions};
use crate::llm::types::*;
use async_trait::async_trait;
use reqwest::Client;
use serde_json::Value;
use std::collections::HashMap;

use super::openai_responses;

// ---------------------------------------------------------------------------
// Provider struct
// ---------------------------------------------------------------------------

/// Provider for the Azure OpenAI Responses API.
///
/// Uses the same protocol as OpenAI Responses but with Azure-specific
/// authentication and URL construction.
pub struct AzureOpenAiResponsesProvider {
    client: Client,
}

impl AzureOpenAiResponsesProvider {
    pub fn new() -> Self {
        Self {
            client: Client::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// Azure-specific configuration
// ---------------------------------------------------------------------------

const DEFAULT_AZURE_API_VERSION: &str = "v1";

/// Parses AZURE_OPENAI_DEPLOYMENT_NAME_MAP env var.
/// Format: "model-id=deployment-name,model-id2=deployment-name2"
fn parse_deployment_name_map() -> HashMap<String, String> {
    let mut map = HashMap::new();
    let value = match std::env::var("AZURE_OPENAI_DEPLOYMENT_NAME_MAP") {
        Ok(v) => v,
        Err(_) => return map,
    };
    for entry in value.split(',') {
        let trimmed = entry.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Some((model_id, deployment_name)) = trimmed.split_once('=') {
            let model_id = model_id.trim();
            let deployment_name = deployment_name.trim();
            if !model_id.is_empty() && !deployment_name.is_empty() {
                map.insert(model_id.to_string(), deployment_name.to_string());
            }
        }
    }
    map
}

/// Resolves the deployment name for a model.
/// Priority: AZURE_OPENAI_DEPLOYMENT_NAME_MAP > model.id (passthrough)
///
/// NOTE: Re-parses the env var each call. This is intentional — the cost is
/// microseconds compared to the hundreds-of-ms HTTP API call, and caching
/// (OnceLock) breaks testability since tests modify env vars between calls.
fn resolve_deployment_name(model: &Model) -> String {
    parse_deployment_name_map()
        .get(&model.id)
        .cloned()
        .unwrap_or_else(|| model.id.clone())
}

/// Resolves the Azure base URL.
/// Priority: AZURE_OPENAI_BASE_URL > AZURE_OPENAI_RESOURCE_NAME > model.base_url
fn resolve_azure_base_url(model: &Model) -> Result<String, String> {
    // 1. Explicit base URL from env
    if let Ok(url) = std::env::var("AZURE_OPENAI_BASE_URL") {
        let url = url.trim().to_string();
        if !url.is_empty() {
            return Ok(normalize_base_url(&url));
        }
    }

    // 2. Resource name → construct URL
    if let Ok(resource_name) = std::env::var("AZURE_OPENAI_RESOURCE_NAME") {
        let resource_name = resource_name.trim().to_string();
        if !resource_name.is_empty() {
            return Ok(format!(
                "https://{}.openai.azure.com/openai/v1",
                resource_name
            ));
        }
    }

    // 3. Fall back to model.base_url
    if !model.base_url.is_empty() {
        return Ok(normalize_base_url(&model.base_url));
    }

    Err(
        "Azure OpenAI base URL is required. Set AZURE_OPENAI_BASE_URL or \
         AZURE_OPENAI_RESOURCE_NAME, or configure model.base_url."
            .to_string(),
    )
}

fn normalize_base_url(url: &str) -> String {
    url.trim_end_matches('/').to_string()
}

fn resolve_api_version() -> String {
    std::env::var("AZURE_OPENAI_API_VERSION")
        .unwrap_or_else(|_| DEFAULT_AZURE_API_VERSION.to_string())
}

/// Build the Azure-specific request body.
/// Same as OpenAI Responses but uses deployment name as model, no `store` field.
fn build_azure_request_body(
    model: &Model,
    context: &LlmContext,
    tools: &[LlmTool],
    options: &StreamOptions,
    deployment_name: &str,
) -> Value {
    // Start with the standard OpenAI Responses body
    let mut body = openai_responses::build_request_body(model, context, tools, options);

    // Replace model with deployment name
    body["model"] = serde_json::json!(deployment_name);

    // Azure doesn't use `store` or `prompt_cache_retention` fields.
    // Pi-mono: Azure passes prompt_cache_key but NOT prompt_cache_retention.
    if let Some(obj) = body.as_object_mut() {
        obj.remove("store");
        obj.remove("prompt_cache_retention");
    }

    body
}

// ---------------------------------------------------------------------------
// ApiProvider implementation
// ---------------------------------------------------------------------------

#[async_trait]
impl ApiProvider for AzureOpenAiResponsesProvider {
    fn api(&self) -> &str {
        api::AZURE_OPENAI_RESPONSES
    }

    async fn stream(
        &self,
        model: &Model,
        context: &LlmContext,
        tools: &[LlmTool],
        options: &StreamOptions,
    ) -> Vec<AssistantMessageEvent> {
        // Resolve API key
        let api_key = if let Some(ref key) = options.api_key {
            key.clone()
        } else {
            match keys::resolve_api_key_from_env(&model.api_key_env) {
                Ok(key) => key,
                Err(_) => {
                    // Fall back to AZURE_OPENAI_API_KEY
                    match std::env::var("AZURE_OPENAI_API_KEY") {
                        Ok(key) if !key.is_empty() => key,
                        _ => {
                            return vec![AssistantMessageEvent::Error(
                                "Azure OpenAI API key is required. Set AZURE_OPENAI_API_KEY \
                                 environment variable or pass it as an argument."
                                    .to_string(),
                            )];
                        }
                    }
                }
            }
        };

        // Resolve Azure-specific configuration
        let base_url = match resolve_azure_base_url(model) {
            Ok(url) => url,
            Err(e) => {
                return vec![AssistantMessageEvent::Error(e)];
            }
        };

        let deployment_name = resolve_deployment_name(model);
        let api_version = resolve_api_version();

        // Azure OpenAI SDK adds api-version as query parameter; we do the same
        // since we build HTTP requests directly.
        let url = format!("{}/responses?api-version={}", base_url, api_version);
        let body = build_azure_request_body(model, context, tools, options, &deployment_name);

        // Build request with Azure-style auth header
        let mut request = self
            .client
            .post(&url)
            .header("api-key", &api_key)
            .header("Content-Type", "application/json");

        // Apply model-level headers
        for (k, v) in &model.headers {
            request = request.header(k.as_str(), v.as_str());
        }
        // Apply per-request headers
        for (k, v) in &options.headers {
            request = request.header(k.as_str(), v.as_str());
        }

        let response = match request.json(&body).send().await {
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
                "API error {status}: {body_text}"
            ))];
        }

        // Reuse OpenAI Responses SSE parsing
        openai_responses::parse_sse_stream(response).await
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;

    // ========================================================================
    // parse_deployment_name_map
    // ========================================================================

    #[test]
    #[serial]
    fn test_parse_deployment_name_map_empty() {
        unsafe { std::env::remove_var("AZURE_OPENAI_DEPLOYMENT_NAME_MAP") };
        let map = parse_deployment_name_map();
        assert!(map.is_empty());
    }

    #[test]
    #[serial]
    fn test_parse_deployment_name_map_single() {
        unsafe { std::env::set_var("AZURE_OPENAI_DEPLOYMENT_NAME_MAP", "gpt-4o=my-gpt4o-deploy") };
        let map = parse_deployment_name_map();
        assert_eq!(map.get("gpt-4o").unwrap(), "my-gpt4o-deploy");
        unsafe { std::env::remove_var("AZURE_OPENAI_DEPLOYMENT_NAME_MAP") };
    }

    #[test]
    #[serial]
    fn test_parse_deployment_name_map_multiple() {
        unsafe {
            std::env::set_var(
                "AZURE_OPENAI_DEPLOYMENT_NAME_MAP",
                "gpt-4o=deploy-4o, gpt-5=deploy-5, o3=deploy-o3",
            )
        };
        let map = parse_deployment_name_map();
        assert_eq!(map.len(), 3);
        assert_eq!(map.get("gpt-4o").unwrap(), "deploy-4o");
        assert_eq!(map.get("gpt-5").unwrap(), "deploy-5");
        assert_eq!(map.get("o3").unwrap(), "deploy-o3");
        unsafe { std::env::remove_var("AZURE_OPENAI_DEPLOYMENT_NAME_MAP") };
    }

    #[test]
    #[serial]
    fn test_parse_deployment_name_map_ignores_malformed() {
        unsafe {
            std::env::set_var(
                "AZURE_OPENAI_DEPLOYMENT_NAME_MAP",
                "good=value, bad_entry, =no_key, no_value=, ,",
            )
        };
        let map = parse_deployment_name_map();
        assert_eq!(map.len(), 1);
        assert_eq!(map.get("good").unwrap(), "value");
        unsafe { std::env::remove_var("AZURE_OPENAI_DEPLOYMENT_NAME_MAP") };
    }

    // ========================================================================
    // resolve_deployment_name
    // ========================================================================

    #[test]
    #[serial]
    fn test_resolve_deployment_name_passthrough() {
        unsafe { std::env::remove_var("AZURE_OPENAI_DEPLOYMENT_NAME_MAP") };
        let model = crate::test_helpers::test_model();
        let name = resolve_deployment_name(&model);
        assert_eq!(name, model.id);
    }

    #[test]
    #[serial]
    fn test_resolve_deployment_name_mapped() {
        let model = crate::test_helpers::test_model();
        unsafe {
            std::env::set_var(
                "AZURE_OPENAI_DEPLOYMENT_NAME_MAP",
                &format!("{}=custom-deploy", model.id),
            )
        };
        let name = resolve_deployment_name(&model);
        assert_eq!(name, "custom-deploy");
        unsafe { std::env::remove_var("AZURE_OPENAI_DEPLOYMENT_NAME_MAP") };
    }

    // ========================================================================
    // resolve_azure_base_url
    // ========================================================================

    #[test]
    #[serial]
    fn test_resolve_base_url_from_env() {
        unsafe {
            std::env::set_var(
                "AZURE_OPENAI_BASE_URL",
                "https://my-resource.openai.azure.com/v1/",
            )
        };
        unsafe { std::env::remove_var("AZURE_OPENAI_RESOURCE_NAME") };
        let model = crate::test_helpers::test_model();
        let url = resolve_azure_base_url(&model).unwrap();
        assert_eq!(url, "https://my-resource.openai.azure.com/v1");
        unsafe { std::env::remove_var("AZURE_OPENAI_BASE_URL") };
    }

    #[test]
    #[serial]
    fn test_resolve_base_url_from_resource_name() {
        unsafe { std::env::remove_var("AZURE_OPENAI_BASE_URL") };
        unsafe { std::env::set_var("AZURE_OPENAI_RESOURCE_NAME", "my-resource") };
        let model = crate::test_helpers::test_model();
        let url = resolve_azure_base_url(&model).unwrap();
        assert_eq!(url, "https://my-resource.openai.azure.com/openai/v1");
        unsafe { std::env::remove_var("AZURE_OPENAI_RESOURCE_NAME") };
    }

    #[test]
    #[serial]
    fn test_resolve_base_url_from_model() {
        unsafe { std::env::remove_var("AZURE_OPENAI_BASE_URL") };
        unsafe { std::env::remove_var("AZURE_OPENAI_RESOURCE_NAME") };
        let mut model = crate::test_helpers::test_model();
        model.base_url = "https://custom.azure.com/openai".into();
        let url = resolve_azure_base_url(&model).unwrap();
        assert_eq!(url, "https://custom.azure.com/openai");
    }

    #[test]
    #[serial]
    fn test_resolve_base_url_error_when_missing() {
        unsafe { std::env::remove_var("AZURE_OPENAI_BASE_URL") };
        unsafe { std::env::remove_var("AZURE_OPENAI_RESOURCE_NAME") };
        let mut model = crate::test_helpers::test_model();
        model.base_url = String::new();
        let result = resolve_azure_base_url(&model);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("AZURE_OPENAI_BASE_URL"));
    }

    // ========================================================================
    // resolve_api_version
    // ========================================================================

    #[test]
    #[serial]
    fn test_resolve_api_version_default() {
        unsafe { std::env::remove_var("AZURE_OPENAI_API_VERSION") };
        assert_eq!(resolve_api_version(), "v1");
    }

    #[test]
    #[serial]
    fn test_resolve_api_version_custom() {
        unsafe { std::env::set_var("AZURE_OPENAI_API_VERSION", "2024-06-01") };
        assert_eq!(resolve_api_version(), "2024-06-01");
        unsafe { std::env::remove_var("AZURE_OPENAI_API_VERSION") };
    }

    // ========================================================================
    // normalize_base_url
    // ========================================================================

    #[test]
    fn test_normalize_base_url_strips_trailing_slash() {
        assert_eq!(
            normalize_base_url("https://example.com/v1/"),
            "https://example.com/v1"
        );
    }

    #[test]
    fn test_normalize_base_url_no_trailing_slash() {
        assert_eq!(
            normalize_base_url("https://example.com/v1"),
            "https://example.com/v1"
        );
    }

    // ========================================================================
    // Provider identity
    // ========================================================================

    #[test]
    fn test_provider_api_identifier() {
        let provider = AzureOpenAiResponsesProvider::new();
        assert_eq!(provider.api(), "azure-openai-responses");
    }

    // ========================================================================
    // build_azure_request_body — no store field
    // ========================================================================

    #[test]
    fn test_azure_request_body_no_store_field() {
        let model = crate::test_helpers::test_model();
        let context = crate::test_helpers::test_context();
        let options = StreamOptions::default();
        let body = build_azure_request_body(&model, &context, &[], &options, "my-deployment");
        assert!(
            body.get("store").is_none(),
            "Azure body should not have 'store' field"
        );
        assert_eq!(body["model"], "my-deployment");
    }

    #[test]
    fn test_azure_request_body_uses_deployment_name() {
        let model = crate::test_helpers::test_model();
        let context = crate::test_helpers::test_context();
        let options = StreamOptions::default();
        let body = build_azure_request_body(&model, &context, &[], &options, "custom-deploy-name");
        assert_eq!(body["model"], "custom-deploy-name");
    }
}
