// WebSearch tool — search the web for information.
//
// Phase 1 MVP: mock implementation that returns placeholder results.
// Phase 2: integrate with Claude API web_search tool or a search provider.

use crate::types::Content;

use super::{AgentTool, ToolOutput};

fn error_output(msg: impl Into<String>) -> ToolOutput {
    ToolOutput {
        content: vec![Content::Text { text: msg.into() }],
        is_error: true,
    }
}

pub struct WebSearchTool;

#[async_trait::async_trait]
impl AgentTool for WebSearchTool {
    fn name(&self) -> &str {
        "web_search"
    }

    fn description(&self) -> &str {
        "Search the web for current information. Returns a list of relevant URLs and titles. \
         Use this when you need up-to-date information that may not be in your training data."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "The search query to use"
                },
                "allowed_domains": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Only include search results from these domains (optional)"
                },
                "blocked_domains": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Never include search results from these domains (optional)"
                }
            },
            "required": ["query"]
        })
    }

    async fn execute(&self, args: serde_json::Value) -> ToolOutput {
        let query = match args.get("query").and_then(|v| v.as_str()) {
            Some(q) if !q.is_empty() => q.to_string(),
            Some(_) => return error_output("query is empty"),
            None => return error_output("missing required parameter: query"),
        };

        let allowed_domains: Option<Vec<String>> = args
            .get("allowed_domains")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect()
            });

        let blocked_domains: Option<Vec<String>> = args
            .get("blocked_domains")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect()
            });

        if allowed_domains.is_some() && blocked_domains.is_some() {
            return error_output(
                "cannot specify both allowed_domains and blocked_domains in the same request",
            );
        }

        // Phase 1: mock implementation — returns placeholder results.
        // Phase 2: integrate with a real search provider or Claude API web_search.
        let results = serde_json::json!([
            {
                "title": format!("Search results for: {}", query),
                "url": "https://example.com",
                "snippet": "Web search integration is in Phase 2. This is a placeholder result."
            }
        ]);

        let output = serde_json::json!({
            "query": query,
            "results": results,
            "durationSeconds": 0.0,
            "note": "Phase 1 mock implementation. Real search integration coming in Phase 2."
        });

        ToolOutput {
            content: vec![Content::Text {
                text: output.to_string(),
            }],
            is_error: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_name_is_web_search() {
        let tool = WebSearchTool;
        assert_eq!(tool.name(), "web_search");
    }

    #[test]
    fn schema_has_required_query() {
        let tool = WebSearchTool;
        let schema = tool.parameters_schema();
        let required = schema["required"].as_array().unwrap();
        assert!(required.iter().any(|v| v.as_str() == Some("query")));
    }

    #[test]
    fn schema_has_optional_domain_filters() {
        let tool = WebSearchTool;
        let schema = tool.parameters_schema();
        assert!(schema["properties"]["allowed_domains"].is_object());
        assert!(schema["properties"]["blocked_domains"].is_object());
    }

    #[tokio::test]
    async fn missing_query_returns_error() {
        let tool = WebSearchTool;
        let output = tool.execute(serde_json::json!({})).await;
        assert!(output.is_error);
    }

    #[tokio::test]
    async fn empty_query_returns_error() {
        let tool = WebSearchTool;
        let output = tool.execute(serde_json::json!({"query": ""})).await;
        assert!(output.is_error);
    }

    #[tokio::test]
    async fn valid_query_returns_results() {
        let tool = WebSearchTool;
        let output = tool
            .execute(serde_json::json!({"query": "Rust programming"}))
            .await;
        assert!(!output.is_error);
        let text = match &output.content[0] {
            crate::types::Content::Text { text } => text.clone(),
            _ => panic!("expected Text content"),
        };
        let json: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(json["query"], "Rust programming");
        assert!(json["results"].is_array());
    }

    #[tokio::test]
    async fn both_domain_filters_returns_error() {
        let tool = WebSearchTool;
        let output = tool
            .execute(serde_json::json!({
                "query": "test",
                "allowed_domains": ["example.com"],
                "blocked_domains": ["bad.com"]
            }))
            .await;
        assert!(output.is_error);
    }

    #[tokio::test]
    async fn allowed_domains_filter_accepted() {
        let tool = WebSearchTool;
        let output = tool
            .execute(serde_json::json!({
                "query": "Rust docs",
                "allowed_domains": ["doc.rust-lang.org"]
            }))
            .await;
        assert!(!output.is_error);
    }
}
