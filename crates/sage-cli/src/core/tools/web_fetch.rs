// WebFetch tool definition metadata.

pub const TOOL_NAME: &str = "web_fetch";

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::tools::{ToolName, all_tool_descriptors};

    #[test]
    fn tool_name_constant_matches_enum() {
        let descs = all_tool_descriptors();
        let desc = descs
            .get(&ToolName::WebFetch)
            .expect("web_fetch descriptor must exist");
        assert_eq!(desc.name.to_string(), TOOL_NAME);
    }

    #[test]
    fn web_fetch_is_not_mutating() {
        let descs = all_tool_descriptors();
        assert!(!descs[&ToolName::WebFetch].mutating);
    }

    #[test]
    fn web_fetch_schema_requires_url() {
        let descs = all_tool_descriptors();
        let schema = &descs[&ToolName::WebFetch].parameters;
        let required = schema["required"].as_array().unwrap();
        assert!(required.iter().any(|v| v.as_str() == Some("url")));
    }
}
