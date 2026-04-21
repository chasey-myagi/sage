// WebSearch tool definition metadata.

pub const TOOL_NAME: &str = "web_search";

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::tools::{ToolName, all_tool_descriptors};

    #[test]
    fn tool_name_constant_matches_enum() {
        let descs = all_tool_descriptors();
        let desc = descs
            .get(&ToolName::WebSearch)
            .expect("web_search descriptor must exist");
        assert_eq!(desc.name.to_string(), TOOL_NAME);
    }

    #[test]
    fn web_search_is_not_mutating() {
        let descs = all_tool_descriptors();
        assert!(!descs[&ToolName::WebSearch].mutating);
    }

    #[test]
    fn web_search_schema_requires_query() {
        let descs = all_tool_descriptors();
        let schema = &descs[&ToolName::WebSearch].parameters;
        let required = schema["required"].as_array().unwrap();
        assert!(required.iter().any(|v| v.as_str() == Some("query")));
    }

    #[test]
    fn web_search_schema_has_domain_filters() {
        let descs = all_tool_descriptors();
        let schema = &descs[&ToolName::WebSearch].parameters;
        assert!(schema["properties"]["allowed_domains"].is_object());
        assert!(schema["properties"]["blocked_domains"].is_object());
    }
}
