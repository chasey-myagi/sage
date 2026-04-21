//! Permission rule loader.
//!
//! Translated from pi-mono `utils/permissions/permissionsLoader.ts`.
//!
//! Loads permission rules from settings files and provides CRUD helpers
//! for persisting rule changes back to disk.

use std::path::Path;

use serde::{Deserialize, Serialize};

use super::engine::{PermissionRule, PermissionRuleSource};
use super::parser::{
    PermissionBehavior, PermissionRuleValue, permission_rule_value_from_str,
    permission_rule_value_to_string,
};

// ============================================================================
// Permissions settings schema
// ============================================================================

/// The `permissions` block inside a settings JSON file.
///
/// Mirrors the `permissions` field of `SettingsJson` from `settings/types.ts`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PermissionsJson {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allow: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deny: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ask: Option<Vec<String>>,
}

impl PermissionsJson {
    /// Iterate over all rules of the given behavior.
    pub fn rules_for(&self, behavior: PermissionBehavior) -> &[String] {
        match behavior {
            PermissionBehavior::Allow => self.allow.as_deref().unwrap_or(&[]),
            PermissionBehavior::Deny => self.deny.as_deref().unwrap_or(&[]),
            PermissionBehavior::Ask => self.ask.as_deref().unwrap_or(&[]),
        }
    }

    /// Mutably borrow the rule list for the given behavior, creating it if absent.
    pub fn rules_for_mut(&mut self, behavior: PermissionBehavior) -> &mut Vec<String> {
        match behavior {
            PermissionBehavior::Allow => self.allow.get_or_insert_with(Vec::new),
            PermissionBehavior::Deny => self.deny.get_or_insert_with(Vec::new),
            PermissionBehavior::Ask => self.ask.get_or_insert_with(Vec::new),
        }
    }
}

// ============================================================================
// Rule extraction
// ============================================================================

const SUPPORTED_BEHAVIORS: &[PermissionBehavior] = &[
    PermissionBehavior::Allow,
    PermissionBehavior::Deny,
    PermissionBehavior::Ask,
];

/// Extract `PermissionRule` objects from a `PermissionsJson` block.
pub fn permissions_json_to_rules(
    permissions: &PermissionsJson,
    source: PermissionRuleSource,
) -> Vec<PermissionRule> {
    let mut rules = Vec::new();
    for behavior in SUPPORTED_BEHAVIORS {
        for rule_str in permissions.rules_for(behavior.clone()) {
            rules.push(PermissionRule {
                source: source.clone(),
                rule_behavior: behavior.clone(),
                rule_value: permission_rule_value_from_str(rule_str),
            });
        }
    }
    rules
}

// ============================================================================
// File-based loading
// ============================================================================

/// Load a `PermissionsJson` from a settings file at `path`.
///
/// Returns `None` if the file does not exist, cannot be read, or does not
/// contain a `permissions` key.
pub fn load_permissions_from_file(path: &Path) -> Option<PermissionsJson> {
    let content = std::fs::read_to_string(path).ok()?;
    let value: serde_json::Value = serde_json::from_str(&content).ok()?;
    let permissions_value = value.get("permissions")?;
    serde_json::from_value(permissions_value.clone()).ok()
}

/// Write updated `PermissionsJson` back to a settings file, preserving all
/// other fields already present in the file.
///
/// Returns `false` if the file could not be read or written.
pub fn save_permissions_to_file(path: &Path, permissions: &PermissionsJson) -> bool {
    // Read existing content (or start with an empty object for new files).
    let existing_content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) if path.exists() => return false,
        Err(_) => "{}".to_string(),
    };
    let mut obj: serde_json::Map<String, serde_json::Value> =
        serde_json::from_str(&existing_content)
            .ok()
            .and_then(|v: serde_json::Value| v.as_object().cloned())
            .unwrap_or_default();

    let permissions_value = match serde_json::to_value(permissions) {
        Ok(v) => v,
        Err(_) => return false,
    };
    obj.insert("permissions".to_owned(), permissions_value);

    let json = match serde_json::to_string_pretty(&serde_json::Value::Object(obj)) {
        Ok(s) => s,
        Err(_) => return false,
    };

    // Ensure parent directory exists.
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    std::fs::write(path, json).is_ok()
}

// ============================================================================
// Rule CRUD
// ============================================================================

/// Add permission rules to a settings file, skipping duplicates.
///
/// Mirrors `addPermissionRulesToSettings` from `permissionsLoader.ts`.
/// Returns `true` on success (including when there is nothing to add).
pub fn add_permission_rules_to_file(
    path: &Path,
    rule_values: &[PermissionRuleValue],
    behavior: PermissionBehavior,
) -> bool {
    if rule_values.is_empty() {
        return true;
    }

    let mut permissions = load_permissions_from_file(path).unwrap_or_default();
    let existing_rules = permissions.rules_for(behavior.clone());

    // Normalize existing entries to their canonical form so legacy names match.
    let existing_set: std::collections::HashSet<String> = existing_rules
        .iter()
        .map(|raw| permission_rule_value_to_string(&permission_rule_value_from_str(raw)))
        .collect();

    let new_rules: Vec<String> = rule_values
        .iter()
        .map(permission_rule_value_to_string)
        .filter(|s| !existing_set.contains(s))
        .collect();

    if new_rules.is_empty() {
        return true;
    }

    permissions.rules_for_mut(behavior).extend(new_rules);

    save_permissions_to_file(path, &permissions)
}

/// Delete a permission rule from a settings file.
///
/// Mirrors `deletePermissionRuleFromSettings` from `permissionsLoader.ts`.
/// Returns `true` if the rule was found and removed.
pub fn delete_permission_rule_from_file(
    path: &Path,
    rule_value: &PermissionRuleValue,
    behavior: PermissionBehavior,
) -> bool {
    let mut permissions = match load_permissions_from_file(path) {
        Some(p) => p,
        None => return false,
    };

    let target = permission_rule_value_to_string(rule_value);
    let rules = permissions.rules_for(behavior.clone());

    let new_rules: Vec<String> = rules
        .iter()
        .filter(|raw| {
            let normalized = permission_rule_value_to_string(&permission_rule_value_from_str(raw));
            normalized != target
        })
        .cloned()
        .collect();

    if new_rules.len() == rules.len() {
        // Rule not found.
        return false;
    }

    *permissions.rules_for_mut(behavior) = new_rules;
    save_permissions_to_file(path, &permissions)
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn write_settings(dir: &TempDir, name: &str, content: &str) -> std::path::PathBuf {
        let path = dir.path().join(name);
        std::fs::write(&path, content).unwrap();
        path
    }

    #[test]
    fn load_permissions_from_valid_file() {
        let tmp = TempDir::new().unwrap();
        let path = write_settings(
            &tmp,
            "settings.json",
            r#"{"permissions":{"allow":["Bash","Read"],"deny":["Write"]}}"#,
        );
        let perms = load_permissions_from_file(&path).unwrap();
        assert_eq!(perms.allow.as_deref().unwrap(), &["Bash", "Read"]);
        assert_eq!(perms.deny.as_deref().unwrap(), &["Write"]);
        assert!(perms.ask.is_none());
    }

    #[test]
    fn load_permissions_missing_key_returns_none() {
        let tmp = TempDir::new().unwrap();
        let path = write_settings(&tmp, "settings.json", r#"{"theme":"dark"}"#);
        assert!(load_permissions_from_file(&path).is_none());
    }

    #[test]
    fn load_permissions_missing_file_returns_none() {
        assert!(
            load_permissions_from_file(std::path::Path::new("/nonexistent/path.json")).is_none()
        );
    }

    #[test]
    fn permissions_json_to_rules_extracts_all_behaviors() {
        let perms = PermissionsJson {
            allow: Some(vec!["Bash".to_string()]),
            deny: Some(vec!["Write".to_string()]),
            ask: Some(vec!["Edit".to_string()]),
        };
        let rules = permissions_json_to_rules(&perms, PermissionRuleSource::ProjectSettings);
        assert_eq!(rules.len(), 3);
        let behaviors: Vec<_> = rules.iter().map(|r| r.rule_behavior.clone()).collect();
        assert!(behaviors.contains(&PermissionBehavior::Allow));
        assert!(behaviors.contains(&PermissionBehavior::Deny));
        assert!(behaviors.contains(&PermissionBehavior::Ask));
    }

    #[test]
    fn add_rules_creates_new_file_entries() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("settings.json");
        std::fs::write(&path, r#"{}"#).unwrap();

        let values = vec![PermissionRuleValue {
            tool_name: "Bash".to_string(),
            rule_content: None,
        }];
        assert!(add_permission_rules_to_file(
            &path,
            &values,
            PermissionBehavior::Allow
        ));

        let perms = load_permissions_from_file(&path).unwrap();
        assert_eq!(perms.allow.as_deref().unwrap(), &["Bash"]);
    }

    #[test]
    fn add_rules_skips_duplicates() {
        let tmp = TempDir::new().unwrap();
        let path = write_settings(
            &tmp,
            "settings.json",
            r#"{"permissions":{"allow":["Bash"]}}"#,
        );
        let values = vec![PermissionRuleValue {
            tool_name: "Bash".to_string(),
            rule_content: None,
        }];
        assert!(add_permission_rules_to_file(
            &path,
            &values,
            PermissionBehavior::Allow
        ));

        let perms = load_permissions_from_file(&path).unwrap();
        // Should still only have one "Bash" entry.
        assert_eq!(perms.allow.as_deref().unwrap().len(), 1);
    }

    #[test]
    fn delete_rule_removes_entry() {
        let tmp = TempDir::new().unwrap();
        let path = write_settings(
            &tmp,
            "settings.json",
            r#"{"permissions":{"allow":["Bash","Read"]}}"#,
        );
        let rv = PermissionRuleValue {
            tool_name: "Bash".to_string(),
            rule_content: None,
        };
        assert!(delete_permission_rule_from_file(
            &path,
            &rv,
            PermissionBehavior::Allow
        ));

        let perms = load_permissions_from_file(&path).unwrap();
        assert_eq!(perms.allow.as_deref().unwrap(), &["Read"]);
    }

    #[test]
    fn delete_nonexistent_rule_returns_false() {
        let tmp = TempDir::new().unwrap();
        let path = write_settings(
            &tmp,
            "settings.json",
            r#"{"permissions":{"allow":["Read"]}}"#,
        );
        let rv = PermissionRuleValue {
            tool_name: "Bash".to_string(),
            rule_content: None,
        };
        assert!(!delete_permission_rule_from_file(
            &path,
            &rv,
            PermissionBehavior::Allow
        ));
    }

    #[test]
    fn save_permissions_preserves_other_settings() {
        let tmp = TempDir::new().unwrap();
        let path = write_settings(&tmp, "settings.json", r#"{"theme":"dark"}"#);
        let perms = PermissionsJson {
            allow: Some(vec!["Bash".to_string()]),
            ..Default::default()
        };
        assert!(save_permissions_to_file(&path, &perms));

        let content = std::fs::read_to_string(&path).unwrap();
        let v: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert_eq!(v["theme"], "dark");
        assert_eq!(v["permissions"]["allow"][0], "Bash");
    }
}
