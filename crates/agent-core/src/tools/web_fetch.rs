// WebFetch tool — fetch URL content and convert to text.
//
// Phase 1 MVP: basic HTTP fetch with simple HTML-to-text extraction.

use std::time::Instant;

use crate::types::Content;

use super::{AgentTool, ToolOutput};

fn error_output(msg: impl Into<String>) -> ToolOutput {
    ToolOutput {
        content: vec![Content::Text { text: msg.into() }],
        is_error: true,
    }
}

fn extract_text_from_html(html: &str) -> String {
    // Phase 1: strip tags with regex; Phase 2: use scraper/html2text.
    let text = html
        .replace("</p>", "\n\n")
        .replace("</P>", "\n\n")
        .replace("<br>", "\n")
        .replace("<br/>", "\n")
        .replace("<br />", "\n")
        .replace("</br>", "\n")
        .replace("</div>", "\n")
        .replace("</DIV>", "\n")
        .replace("</li>", "\n")
        .replace("</LI>", "\n")
        .replace("</tr>", "\n")
        .replace("</TR>", "\n")
        .replace("</h1>", "\n\n")
        .replace("</h2>", "\n\n")
        .replace("</h3>", "\n\n")
        .replace("</h4>", "\n\n")
        .replace("</h5>", "\n\n")
        .replace("</h6>", "\n\n");

    // Remove <script> and <style> blocks.
    let re_script = regex::Regex::new(r"(?si)<script[^>]*>.*?</script>").unwrap();
    let text = re_script.replace_all(&text, " ");
    let re_style = regex::Regex::new(r"(?si)<style[^>]*>.*?</style>").unwrap();
    let text = re_style.replace_all(&text, " ");

    // Strip remaining tags.
    let re_tags = regex::Regex::new(r"<[^>]+>").unwrap();
    let text = re_tags.replace_all(&text, "");

    // Decode common HTML entities.
    let text = text
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&nbsp;", " ");

    // Collapse excessive whitespace / blank lines.
    let re_blank = regex::Regex::new(r"\n{3,}").unwrap();
    let text = re_blank.replace_all(&text, "\n\n");
    let re_spaces = regex::Regex::new(r" {2,}").unwrap();
    let text = re_spaces.replace_all(&text, " ");

    text.trim().to_string()
}

pub struct WebFetchTool;

#[async_trait::async_trait]
impl AgentTool for WebFetchTool {
    fn name(&self) -> &str {
        "web_fetch"
    }

    fn description(&self) -> &str {
        "Fetch content from a URL and return it as plain text. \
         HTML pages are converted to readable text. \
         Use this to read documentation, web pages, or any URL-accessible content."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        // prompt field deferred to Phase 2
        serde_json::json!({
            "type": "object",
            "properties": {
                "url": {
                    "type": "string",
                    "description": "The URL to fetch content from"
                }
            },
            "required": ["url"]
        })
    }

    async fn execute(&self, args: serde_json::Value) -> ToolOutput {
        let url = match args.get("url").and_then(|v| v.as_str()) {
            Some(u) if !u.is_empty() => u.to_string(),
            Some(_) => return error_output("url is empty"),
            None => return error_output("missing required parameter: url"),
        };

        let client = match reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(60))
            .user_agent("Sage/0.1 (coding agent)")
            .build()
        {
            Ok(c) => c,
            Err(e) => return error_output(format!("failed to build HTTP client: {e}")),
        };

        let start = Instant::now();

        let response = match client.get(&url).send().await {
            Ok(r) => r,
            Err(e) => return error_output(format!("HTTP request failed: {e}")),
        };

        let status = response.status();
        let code = status.as_u16();
        let code_text = status.canonical_reason().unwrap_or("Unknown").to_string();

        let content_type = response
            .headers()
            .get("content-type")
            .and_then(|h| h.to_str().ok())
            .unwrap_or("text/plain")
            .to_string();

        let body_bytes = match response.bytes().await {
            Ok(b) => b,
            Err(e) => return error_output(format!("failed to read response body: {e}")),
        };

        let duration_ms = start.elapsed().as_millis() as u64;
        let bytes = body_bytes.len();

        let result =
            if content_type.contains("text/html") || content_type.contains("application/xhtml") {
                let html = String::from_utf8_lossy(&body_bytes);
                extract_text_from_html(&html)
            } else {
                match String::from_utf8(body_bytes.to_vec()) {
                    Ok(s) => s,
                    Err(_) => format!(
                        "[binary content, {} bytes, content-type: {}]",
                        bytes, content_type
                    ),
                }
            };

        let output = serde_json::json!({
            "bytes": bytes,
            "code": code,
            "codeText": code_text,
            "result": result,
            "durationMs": duration_ms,
            "url": url,
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
    fn tool_name_is_web_fetch() {
        let tool = WebFetchTool;
        assert_eq!(tool.name(), "web_fetch");
    }

    #[test]
    fn schema_has_required_url() {
        let tool = WebFetchTool;
        let schema = tool.parameters_schema();
        let required = schema["required"].as_array().unwrap();
        assert!(required.iter().any(|v| v.as_str() == Some("url")));
    }

    #[tokio::test]
    async fn missing_url_returns_error() {
        let tool = WebFetchTool;
        let output = tool.execute(serde_json::json!({})).await;
        assert!(output.is_error);
    }

    #[tokio::test]
    async fn empty_url_returns_error() {
        let tool = WebFetchTool;
        let output = tool.execute(serde_json::json!({"url": ""})).await;
        assert!(output.is_error);
    }

    #[test]
    fn extract_text_strips_html_tags() {
        let html = "<html><body><h1>Title</h1><p>Hello <b>world</b></p></body></html>";
        let text = extract_text_from_html(html);
        assert!(text.contains("Title"));
        assert!(text.contains("Hello"));
        assert!(text.contains("world"));
        assert!(!text.contains("<b>"));
        assert!(!text.contains("</b>"));
    }

    #[test]
    fn extract_text_removes_script_blocks() {
        let html = "<html><body><script>alert('xss')</script><p>Content</p></body></html>";
        let text = extract_text_from_html(html);
        assert!(text.contains("Content"));
        assert!(!text.contains("alert"));
        assert!(!text.contains("xss"));
    }

    #[test]
    fn extract_text_removes_style_blocks() {
        let html =
            "<html><head><style>.foo { color: red }</style></head><body><p>Text</p></body></html>";
        let text = extract_text_from_html(html);
        assert!(text.contains("Text"));
        assert!(!text.contains(".foo"));
        assert!(!text.contains("color"));
    }

    #[test]
    fn extract_text_decodes_html_entities() {
        let html = "<p>A &amp; B &lt; C &gt; D</p>";
        let text = extract_text_from_html(html);
        assert!(text.contains("A & B < C > D"));
    }
}
