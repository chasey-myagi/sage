//! Permission decision engine.
//!
//! Translated from pi-mono `utils/permissions/permissions.ts`.
//!
//! Provides the core data types for permission rules and decisions, plus
//! helper functions for querying the permission context.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use super::mode::PermissionMode;
use super::parser::{
    PermissionBehavior, PermissionRuleValue, permission_rule_value_from_str,
    permission_rule_value_to_string,
};

// ============================================================================
// Rule source
// ============================================================================

/// Where a permission rule originated from.
///
/// Mirrors `PermissionRuleSource` from `types/permissions.ts`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PermissionRuleSource {
    UserSettings,
    ProjectSettings,
    LocalSettings,
    FlagSettings,
    PolicySettings,
    CliArg,
    Command,
    Session,
}

impl std::fmt::Display for PermissionRuleSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            PermissionRuleSource::UserSettings => "user settings",
            PermissionRuleSource::ProjectSettings => "project settings",
            PermissionRuleSource::LocalSettings => "local settings",
            PermissionRuleSource::FlagSettings => "flag settings",
            PermissionRuleSource::PolicySettings => "policy settings",
            PermissionRuleSource::CliArg => "CLI argument",
            PermissionRuleSource::Command => "command",
            PermissionRuleSource::Session => "session",
        };
        write!(f, "{s}")
    }
}

// ============================================================================
// Permission rule
// ============================================================================

/// A single permission rule with its source and behavior.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PermissionRule {
    pub source: PermissionRuleSource,
    pub rule_behavior: PermissionBehavior,
    pub rule_value: PermissionRuleValue,
}

// ============================================================================
// Decision types
// ============================================================================

/// The reason behind a permission decision.
#[derive(Debug, Clone)]
pub enum PermissionDecisionReason {
    /// Matched an explicit rule.
    Rule(PermissionRule),
    /// Triggered by a hook.
    Hook {
        hook_name: String,
        reason: Option<String>,
    },
    /// Determined by the current permission mode.
    Mode(PermissionMode),
    /// Blocked by a safety check.
    SafetyCheck(String),
    /// Other / unspecified reason.
    Other(String),
}

/// The outcome of a permission check.
#[derive(Debug, Clone)]
pub enum PermissionDecision {
    /// Tool call is allowed without user interaction.
    Allow {
        reason: Option<PermissionDecisionReason>,
    },
    /// Tool call is denied without offering the user a choice.
    Deny {
        reason: Option<PermissionDecisionReason>,
        message: String,
    },
    /// User must be asked before the tool call proceeds.
    Ask {
        reason: Option<PermissionDecisionReason>,
        message: String,
    },
}

impl PermissionDecision {
    pub fn behavior(&self) -> PermissionBehavior {
        match self {
            PermissionDecision::Allow { .. } => PermissionBehavior::Allow,
            PermissionDecision::Deny { .. } => PermissionBehavior::Deny,
            PermissionDecision::Ask { .. } => PermissionBehavior::Ask,
        }
    }

    pub fn is_allow(&self) -> bool {
        matches!(self, PermissionDecision::Allow { .. })
    }

    pub fn is_deny(&self) -> bool {
        matches!(self, PermissionDecision::Deny { .. })
    }
}

// ============================================================================
// Tool permission context
// ============================================================================

/// Type alias for per-source rule lists (raw serialized strings).
pub type RulesBySource = HashMap<PermissionRuleSource, Vec<String>>;

/// Runtime context holding the current permission mode and rule sets.
///
/// Mirrors `ToolPermissionContext` from `types/permissions.ts`.
#[derive(Debug, Clone, Default)]
pub struct ToolPermissionContext {
    pub mode: PermissionMode,
    /// Rules that always allow a tool to run.
    pub always_allow_rules: RulesBySource,
    /// Rules that always deny a tool from running.
    pub always_deny_rules: RulesBySource,
    /// Rules that always ask the user before running.
    pub always_ask_rules: RulesBySource,
    /// Whether bypass-permissions mode is available in this session.
    pub is_bypass_permissions_mode_available: bool,
    /// Mode that was active before entering plan mode (for restoration).
    pub pre_plan_mode: Option<PermissionMode>,
}

impl ToolPermissionContext {
    pub fn new(mode: PermissionMode) -> Self {
        Self {
            mode,
            ..Default::default()
        }
    }

    /// Add an allow rule for a given source.
    pub fn add_allow_rule(&mut self, source: PermissionRuleSource, rule: String) {
        self.always_allow_rules
            .entry(source)
            .or_default()
            .push(rule);
    }

    /// Add a deny rule for a given source.
    pub fn add_deny_rule(&mut self, source: PermissionRuleSource, rule: String) {
        self.always_deny_rules.entry(source).or_default().push(rule);
    }

    /// Add an ask rule for a given source.
    pub fn add_ask_rule(&mut self, source: PermissionRuleSource, rule: String) {
        self.always_ask_rules.entry(source).or_default().push(rule);
    }
}

// ============================================================================
// Rule accessors
// ============================================================================

/// Ordered list of all rule sources, matching `PERMISSION_RULE_SOURCES` from TS.
pub const PERMISSION_RULE_SOURCES: &[PermissionRuleSource] = &[
    PermissionRuleSource::UserSettings,
    PermissionRuleSource::ProjectSettings,
    PermissionRuleSource::LocalSettings,
    PermissionRuleSource::FlagSettings,
    PermissionRuleSource::PolicySettings,
    PermissionRuleSource::CliArg,
    PermissionRuleSource::Command,
    PermissionRuleSource::Session,
];

fn rules_from_source_map(map: &RulesBySource, behavior: PermissionBehavior) -> Vec<PermissionRule> {
    PERMISSION_RULE_SOURCES
        .iter()
        .flat_map(|source| {
            map.get(source)
                .into_iter()
                .flatten()
                .map(move |rule_str| PermissionRule {
                    source: source.clone(),
                    rule_behavior: behavior,
                    rule_value: permission_rule_value_from_str(rule_str),
                })
        })
        .collect()
}

/// Get all allow rules from the permission context.
pub fn get_allow_rules(ctx: &ToolPermissionContext) -> Vec<PermissionRule> {
    rules_from_source_map(&ctx.always_allow_rules, PermissionBehavior::Allow)
}

/// Get all deny rules from the permission context.
pub fn get_deny_rules(ctx: &ToolPermissionContext) -> Vec<PermissionRule> {
    rules_from_source_map(&ctx.always_deny_rules, PermissionBehavior::Deny)
}

/// Get all ask rules from the permission context.
pub fn get_ask_rules(ctx: &ToolPermissionContext) -> Vec<PermissionRule> {
    rules_from_source_map(&ctx.always_ask_rules, PermissionBehavior::Ask)
}

// ============================================================================
// Rule matching
// ============================================================================

/// Check if a tool matches a rule exactly (no content filter required).
///
/// Matches when the rule has no `rule_content` and the tool name equals the
/// rule's tool name. Also handles MCP server-level permissions (`mcp__server`).
fn tool_matches_rule(tool_name: &str, rule: &PermissionRule) -> bool {
    // Rule must have no content to match an entire tool.
    if rule.rule_value.rule_content.is_some() {
        return false;
    }

    if rule.rule_value.tool_name == tool_name {
        return true;
    }

    // MCP server-level permission: rule "mcp__server1" matches "mcp__server1__tool1".
    // Also supports wildcard: "mcp__server1__*" matches all tools from server1.
    if let (Some(rule_info), Some(tool_info)) = (
        mcp_info_from_name(&rule.rule_value.tool_name),
        mcp_info_from_name(tool_name),
    ) {
        return rule_info.server_name == tool_info.server_name
            && rule_info.tool_name.as_deref().is_none_or(|t| t == "*");
    }

    false
}

/// Minimal MCP tool name info extracted from `mcp__<server>__<tool>` format.
struct McpInfo {
    server_name: String,
    tool_name: Option<String>,
}

fn mcp_info_from_name(name: &str) -> Option<McpInfo> {
    if !name.starts_with("mcp__") {
        return None;
    }
    let rest = &name["mcp__".len()..];
    if let Some((server, tool)) = rest.split_once("__") {
        Some(McpInfo {
            server_name: server.to_owned(),
            tool_name: Some(tool.to_owned()),
        })
    } else {
        Some(McpInfo {
            server_name: rest.to_owned(),
            tool_name: None,
        })
    }
}

/// Find an allow rule that matches the given tool name.
pub fn find_allow_rule_for_tool(
    ctx: &ToolPermissionContext,
    tool_name: &str,
) -> Option<PermissionRule> {
    get_allow_rules(ctx)
        .into_iter()
        .find(|rule| tool_matches_rule(tool_name, rule))
}

/// Find a deny rule that matches the given tool name.
pub fn find_deny_rule_for_tool(
    ctx: &ToolPermissionContext,
    tool_name: &str,
) -> Option<PermissionRule> {
    get_deny_rules(ctx)
        .into_iter()
        .find(|rule| tool_matches_rule(tool_name, rule))
}

/// Find an ask rule that matches the given tool name.
pub fn find_ask_rule_for_tool(
    ctx: &ToolPermissionContext,
    tool_name: &str,
) -> Option<PermissionRule> {
    get_ask_rules(ctx)
        .into_iter()
        .find(|rule| tool_matches_rule(tool_name, rule))
}

/// Build a map of `rule_content → PermissionRule` for all allow rules targeting
/// a specific tool.  Used to do content-level permission checks.
pub fn get_allow_rule_contents_for_tool(
    ctx: &ToolPermissionContext,
    tool_name: &str,
) -> HashMap<String, PermissionRule> {
    get_allow_rules(ctx)
        .into_iter()
        .filter(|rule| {
            rule.rule_value.tool_name == tool_name && rule.rule_value.rule_content.is_some()
        })
        .filter_map(|rule| {
            rule.rule_value
                .rule_content
                .clone()
                .map(|content| (content, rule))
        })
        .collect()
}

// ============================================================================
// Permission request message
// ============================================================================

/// Build a human-readable message describing why permission is being requested.
///
/// Mirrors `createPermissionRequestMessage` from `permissions.ts`.
pub fn create_permission_request_message(
    tool_name: &str,
    reason: Option<&PermissionDecisionReason>,
) -> String {
    let Some(reason) = reason else {
        return format!(
            "Claude requested permissions to use {tool_name}, but you haven't granted it yet."
        );
    };

    match reason {
        PermissionDecisionReason::Rule(rule) => {
            let rule_str = permission_rule_value_to_string(&rule.rule_value);
            format!(
                "Permission rule '{rule_str}' from {} requires approval for this {tool_name} command",
                rule.source
            )
        }
        PermissionDecisionReason::Hook { hook_name, reason } => {
            if let Some(r) = reason {
                format!("Hook '{hook_name}' blocked this action: {r}")
            } else {
                format!("Hook '{hook_name}' requires approval for this {tool_name} command")
            }
        }
        PermissionDecisionReason::Mode(mode) => {
            format!(
                "Current permission mode ({}) requires approval for this {tool_name} command",
                mode.title()
            )
        }
        PermissionDecisionReason::SafetyCheck(msg) | PermissionDecisionReason::Other(msg) => {
            msg.clone()
        }
    }
}

// ============================================================================
// Basic permission check
// ============================================================================

/// Perform a basic rule-based permission check for a tool.
///
/// Checks deny rules first, then ask rules, then allow rules.
/// Returns `Ask` if no rule matches (prompts user by default).
///
/// This is a simplified version of the full `getUserPermission` flow from TS,
/// which also handles classifiers, hooks, and async approval flows.
pub fn check_tool_permission(ctx: &ToolPermissionContext, tool_name: &str) -> PermissionDecision {
    // dontAsk mode denies instead of asking.
    if ctx.mode == PermissionMode::DontAsk {
        let message = format!("Permission denied: {tool_name} (dontAsk mode)");
        return PermissionDecision::Deny {
            reason: None,
            message,
        };
    }

    // Plan mode: only read-only tools are allowed.
    // Note: EnterPlanMode / ExitPlanMode are handled directly by the harness and never
    // reach ToolAdapter, so they must not appear here.
    if ctx.mode == PermissionMode::Plan {
        let read_only = matches!(
            tool_name,
            "read" | "grep" | "find" | "ls" | "web_fetch" | "web_search"
        );
        if !read_only {
            return PermissionDecision::Deny {
                reason: Some(PermissionDecisionReason::Mode(PermissionMode::Plan)),
                message: format!("{tool_name} is not allowed in plan mode"),
            };
        }
        return PermissionDecision::Allow { reason: None };
    }

    // Deny rules win even over bypassPermissions (CC step 1a).
    if let Some(rule) = find_deny_rule_for_tool(ctx, tool_name) {
        let message = create_permission_request_message(
            tool_name,
            Some(&PermissionDecisionReason::Rule(rule.clone())),
        );
        return PermissionDecision::Deny {
            reason: Some(PermissionDecisionReason::Rule(rule)),
            message,
        };
    }

    // bypassPermissions skips all remaining checks (CC step 2a).
    if ctx.mode == PermissionMode::BypassPermissions {
        return PermissionDecision::Allow { reason: None };
    }

    // Ask rules have higher priority than allow rules (CC step 1b before step 2b).
    if let Some(rule) = find_ask_rule_for_tool(ctx, tool_name) {
        let message = create_permission_request_message(
            tool_name,
            Some(&PermissionDecisionReason::Rule(rule.clone())),
        );
        return PermissionDecision::Ask {
            reason: Some(PermissionDecisionReason::Rule(rule)),
            message,
        };
    }

    // Explicit allow rule.
    if let Some(rule) = find_allow_rule_for_tool(ctx, tool_name) {
        return PermissionDecision::Allow {
            reason: Some(PermissionDecisionReason::Rule(rule)),
        };
    }

    // Default: ask the user.
    let message = create_permission_request_message(tool_name, None);
    PermissionDecision::Ask {
        reason: None,
        message,
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn make_ctx_with_allow(tool: &str) -> ToolPermissionContext {
        let mut ctx = ToolPermissionContext::default();
        ctx.add_allow_rule(PermissionRuleSource::Session, tool.to_owned());
        ctx
    }

    fn make_ctx_with_deny(tool: &str) -> ToolPermissionContext {
        let mut ctx = ToolPermissionContext::default();
        ctx.add_deny_rule(PermissionRuleSource::Session, tool.to_owned());
        ctx
    }

    #[test]
    fn allow_rule_permits_tool() {
        let ctx = make_ctx_with_allow("Bash");
        assert!(check_tool_permission(&ctx, "Bash").is_allow());
    }

    #[test]
    fn deny_rule_blocks_tool() {
        let ctx = make_ctx_with_deny("Bash");
        assert!(check_tool_permission(&ctx, "Bash").is_deny());
    }

    #[test]
    fn no_rule_defaults_to_ask() {
        let ctx = ToolPermissionContext::default();
        assert!(matches!(
            check_tool_permission(&ctx, "Bash"),
            PermissionDecision::Ask { .. }
        ));
    }

    #[test]
    fn bypass_mode_always_allows() {
        let mut ctx = ToolPermissionContext::default();
        ctx.mode = PermissionMode::BypassPermissions;
        assert!(check_tool_permission(&ctx, "Bash").is_allow());
    }

    #[test]
    fn dont_ask_mode_denies_instead_of_asking() {
        let mut ctx = ToolPermissionContext::default();
        ctx.mode = PermissionMode::DontAsk;
        assert!(check_tool_permission(&ctx, "Bash").is_deny());
    }

    #[test]
    fn deny_takes_priority_over_allow() {
        let mut ctx = ToolPermissionContext::default();
        ctx.add_allow_rule(PermissionRuleSource::Session, "Bash".to_owned());
        ctx.add_deny_rule(PermissionRuleSource::Session, "Bash".to_owned());
        assert!(check_tool_permission(&ctx, "Bash").is_deny());
    }

    #[test]
    fn deny_rule_blocks_even_in_bypass_mode() {
        let mut ctx = ToolPermissionContext::default();
        ctx.mode = PermissionMode::BypassPermissions;
        ctx.add_deny_rule(PermissionRuleSource::Session, "Bash".to_owned());
        assert!(check_tool_permission(&ctx, "Bash").is_deny());
    }

    #[test]
    fn plan_mode_denies_bash() {
        let mut ctx = ToolPermissionContext::default();
        ctx.mode = PermissionMode::Plan;
        assert!(check_tool_permission(&ctx, "Bash").is_deny());
    }

    #[test]
    fn plan_mode_denies_write() {
        let mut ctx = ToolPermissionContext::default();
        ctx.mode = PermissionMode::Plan;
        assert!(check_tool_permission(&ctx, "write").is_deny());
    }

    #[test]
    fn plan_mode_denies_edit() {
        let mut ctx = ToolPermissionContext::default();
        ctx.mode = PermissionMode::Plan;
        assert!(check_tool_permission(&ctx, "edit").is_deny());
    }

    #[test]
    fn plan_mode_allows_read() {
        let mut ctx = ToolPermissionContext::default();
        ctx.mode = PermissionMode::Plan;
        assert!(check_tool_permission(&ctx, "read").is_allow());
    }

    #[test]
    fn plan_mode_allows_grep() {
        let mut ctx = ToolPermissionContext::default();
        ctx.mode = PermissionMode::Plan;
        assert!(check_tool_permission(&ctx, "grep").is_allow());
    }

    #[test]
    fn plan_mode_deny_has_mode_reason() {
        let mut ctx = ToolPermissionContext::default();
        ctx.mode = PermissionMode::Plan;
        let decision = check_tool_permission(&ctx, "Bash");
        assert!(matches!(
            decision,
            PermissionDecision::Deny {
                reason: Some(PermissionDecisionReason::Mode(PermissionMode::Plan)),
                ..
            }
        ));
    }

    #[test]
    fn ask_rule_takes_priority_over_allow_rule() {
        let mut ctx = ToolPermissionContext::default();
        ctx.add_allow_rule(PermissionRuleSource::Session, "Bash".to_owned());
        ctx.add_ask_rule(PermissionRuleSource::Session, "Bash".to_owned());
        assert!(matches!(
            check_tool_permission(&ctx, "Bash"),
            PermissionDecision::Ask { .. }
        ));
    }

    #[test]
    fn get_allow_rules_collects_all_sources() {
        let mut ctx = ToolPermissionContext::default();
        ctx.add_allow_rule(PermissionRuleSource::UserSettings, "Read".to_owned());
        ctx.add_allow_rule(PermissionRuleSource::Session, "Bash".to_owned());
        let rules = get_allow_rules(&ctx);
        assert_eq!(rules.len(), 2);
    }

    #[test]
    fn mcp_server_rule_matches_tool() {
        let mut ctx = ToolPermissionContext::default();
        ctx.add_allow_rule(PermissionRuleSource::Session, "mcp__server1".to_owned());
        // "mcp__server1" should match "mcp__server1__write"
        assert!(find_allow_rule_for_tool(&ctx, "mcp__server1__write").is_some());
    }

    #[test]
    fn request_message_default() {
        let msg = create_permission_request_message("Bash", None);
        assert!(msg.contains("Bash"));
        assert!(msg.contains("haven't granted"));
    }

    #[test]
    fn request_message_rule_reason() {
        let rule = PermissionRule {
            source: PermissionRuleSource::UserSettings,
            rule_behavior: PermissionBehavior::Ask,
            rule_value: PermissionRuleValue {
                tool_name: "Bash".to_string(),
                rule_content: None,
            },
        };
        let msg =
            create_permission_request_message("Bash", Some(&PermissionDecisionReason::Rule(rule)));
        assert!(msg.contains("Bash"));
        assert!(msg.contains("user settings"));
    }

    // ── Plan mode whitelist: exhaustive coverage of all 6 allowed tools ──────

    #[test]
    fn plan_mode_allows_find() {
        let mut ctx = ToolPermissionContext::default();
        ctx.mode = PermissionMode::Plan;
        assert!(check_tool_permission(&ctx, "find").is_allow());
    }

    #[test]
    fn plan_mode_allows_ls() {
        let mut ctx = ToolPermissionContext::default();
        ctx.mode = PermissionMode::Plan;
        assert!(check_tool_permission(&ctx, "ls").is_allow());
    }

    #[test]
    fn plan_mode_allows_web_fetch() {
        let mut ctx = ToolPermissionContext::default();
        ctx.mode = PermissionMode::Plan;
        assert!(check_tool_permission(&ctx, "web_fetch").is_allow());
    }

    #[test]
    fn plan_mode_allows_web_search() {
        let mut ctx = ToolPermissionContext::default();
        ctx.mode = PermissionMode::Plan;
        assert!(check_tool_permission(&ctx, "web_search").is_allow());
    }

    #[test]
    fn plan_mode_denies_notebook_edit() {
        let mut ctx = ToolPermissionContext::default();
        ctx.mode = PermissionMode::Plan;
        assert!(check_tool_permission(&ctx, "notebook_edit").is_deny());
    }

    #[test]
    fn plan_mode_denies_glob() {
        // glob is not in the read-only whitelist even though it is read-only in spirit.
        let mut ctx = ToolPermissionContext::default();
        ctx.mode = PermissionMode::Plan;
        assert!(check_tool_permission(&ctx, "glob").is_deny());
    }

    #[test]
    fn plan_mode_whitelist_exhaustive() {
        // All 6 whitelisted tools must be allowed; everything else must be denied.
        let allowed = ["read", "grep", "find", "ls", "web_fetch", "web_search"];
        let denied = ["bash", "write", "edit", "glob", "notebook_edit", "mcp__server__tool"];

        let mut ctx = ToolPermissionContext::default();
        ctx.mode = PermissionMode::Plan;

        for tool in &allowed {
            assert!(
                check_tool_permission(&ctx, tool).is_allow(),
                "expected {tool} to be allowed in Plan mode"
            );
        }
        for tool in &denied {
            assert!(
                check_tool_permission(&ctx, tool).is_deny(),
                "expected {tool} to be denied in Plan mode"
            );
        }
    }

    #[test]
    fn permission_rule_source_display() {
        assert_eq!(PermissionRuleSource::UserSettings.to_string(), "user settings");
        assert_eq!(PermissionRuleSource::ProjectSettings.to_string(), "project settings");
        assert_eq!(PermissionRuleSource::CliArg.to_string(), "CLI argument");
        assert_eq!(PermissionRuleSource::Session.to_string(), "session");
    }

    #[test]
    fn mcp_wildcard_prefix_matches_any_subcommand() {
        // "mcp__myserver" should match "mcp__myserver__anything"
        let mut ctx = ToolPermissionContext::default();
        ctx.add_allow_rule(PermissionRuleSource::Session, "mcp__myserver".to_owned());
        assert!(find_allow_rule_for_tool(&ctx, "mcp__myserver__read").is_some());
        assert!(find_allow_rule_for_tool(&ctx, "mcp__myserver__write").is_some());
        assert!(find_allow_rule_for_tool(&ctx, "mcp__other__read").is_none());
    }
}
