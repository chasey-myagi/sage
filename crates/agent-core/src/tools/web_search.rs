// WebSearch tool — not yet implemented; returns an error for all queries.

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
        "Search the web for current information. Not yet implemented: no search provider is \
         configured. Use web_fetch to retrieve a specific URL instead."
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
        match args.get("query").and_then(|v| v.as_str()) {
            Some(q) if !q.is_empty() => {}
            Some(_) => return error_output("query is empty"),
            None => return error_output("missing required parameter: query"),
        };

        error_output(
            "web_search is not yet implemented: no search provider is configured. \
             Use web_fetch to retrieve a specific URL instead.",
        )
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
    async fn valid_query_returns_not_implemented_error() {
        let tool = WebSearchTool;
        let output = tool
            .execute(serde_json::json!({"query": "Rust programming"}))
            .await;
        assert!(output.is_error);
        let text = match &output.content[0] {
            crate::types::Content::Text { text } => text.clone(),
            _ => panic!("expected Text content"),
        };
        assert!(text.contains("not yet implemented"));
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
    async fn allowed_domains_filter_returns_not_implemented_error() {
        let tool = WebSearchTool;
        let output = tool
            .execute(serde_json::json!({
                "query": "Rust docs",
                "allowed_domains": ["doc.rust-lang.org"]
            }))
            .await;
        assert!(output.is_error);
    }
}
