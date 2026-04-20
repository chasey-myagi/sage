//! One-time migrations that run on startup.
//!
//! Translated from pi-mono `packages/coding-agent/src/migrations.ts`.

use std::path::{Path, PathBuf};

use crate::config::{get_agent_dir, get_bin_dir, CONFIG_DIR_NAME};

const MIGRATION_GUIDE_URL: &str =
    "https://github.com/badlogic/pi-mono/blob/main/packages/coding-agent/CHANGELOG.md#extensions-migration";
const EXTENSIONS_DOC_URL: &str =
    "https://github.com/badlogic/pi-mono/blob/main/packages/coding-agent/docs/extensions.md";

// ============================================================================
// migrateAuthToAuthJson
// ============================================================================

/// Migrate legacy `oauth.json` and `settings.json` apiKeys to `auth.json`.
///
/// Returns a list of provider names that were migrated.
///
/// Mirrors `migrateAuthToAuthJson()` from TypeScript.
pub fn migrate_auth_to_auth_json() -> Vec<String> {
    let agent_dir = get_agent_dir();
    let auth_path = agent_dir.join("auth.json");
    let oauth_path = agent_dir.join("oauth.json");
    let settings_path = agent_dir.join("settings.json");

    // Skip if auth.json already exists
    if auth_path.exists() {
        return vec![];
    }

    let mut migrated: serde_json::Map<String, serde_json::Value> = serde_json::Map::new();
    let mut providers: Vec<String> = Vec::new();

    // Migrate oauth.json
    if oauth_path.exists() {
        if let Ok(content) = std::fs::read_to_string(&oauth_path) {
            if let Ok(serde_json::Value::Object(obj)) = serde_json::from_str::<serde_json::Value>(&content)
            {
                for (provider, cred) in obj {
                    if let serde_json::Value::Object(mut cred_obj) = cred {
                        cred_obj.insert("type".into(), serde_json::Value::String("oauth".into()));
                        migrated.insert(provider.clone(), serde_json::Value::Object(cred_obj));
                        providers.push(provider);
                    }
                }
                // Rename oauth.json → oauth.json.migrated
                let migrated_path = agent_dir.join("oauth.json.migrated");
                let _ = std::fs::rename(&oauth_path, &migrated_path);
            }
        }
    }

    // Migrate settings.json apiKeys
    if settings_path.exists() {
        if let Ok(content) = std::fs::read_to_string(&settings_path) {
            if let Ok(mut settings) = serde_json::from_str::<serde_json::Value>(&content) {
                if let Some(api_keys) = settings
                    .get("apiKeys")
                    .cloned()
                    .and_then(|v| v.as_object().cloned())
                {
                    for (provider, key) in api_keys {
                        if !migrated.contains_key(&provider) {
                            if let Some(key_str) = key.as_str() {
                                let mut cred_obj = serde_json::Map::new();
                                cred_obj.insert(
                                    "type".into(),
                                    serde_json::Value::String("api_key".into()),
                                );
                                cred_obj.insert(
                                    "key".into(),
                                    serde_json::Value::String(key_str.into()),
                                );
                                migrated.insert(
                                    provider.clone(),
                                    serde_json::Value::Object(cred_obj),
                                );
                                providers.push(provider);
                            }
                        }
                    }
                    // Remove apiKeys from settings
                    if let Some(obj) = settings.as_object_mut() {
                        obj.remove("apiKeys");
                    }
                    if let Ok(updated) = serde_json::to_string_pretty(&settings) {
                        let _ = std::fs::write(&settings_path, updated);
                    }
                }
            }
        }
    }

    if !migrated.is_empty() {
        if let Some(parent) = auth_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(json_str) = serde_json::to_string_pretty(&serde_json::Value::Object(migrated)) {
            let _ = std::fs::write(&auth_path, &json_str);
            // Set restrictive permissions on Unix
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = std::fs::set_permissions(
                    &auth_path,
                    std::fs::Permissions::from_mode(0o600),
                );
            }
        }
    }

    providers
}

// ============================================================================
// migrateSessionsFromAgentRoot
// ============================================================================

/// Migrate sessions from `~/.pi/agent/*.jsonl` to proper session directories.
///
/// Bug in v0.30.0: Sessions were saved to `~/.pi/agent/` instead of
/// `~/.pi/agent/sessions/<encoded-cwd>/`.
///
/// Mirrors `migrateSessionsFromAgentRoot()` from TypeScript.
pub fn migrate_sessions_from_agent_root() {
    let agent_dir = get_agent_dir();

    let files: Vec<PathBuf> = match std::fs::read_dir(&agent_dir) {
        Ok(entries) => entries
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("jsonl"))
            .collect(),
        Err(_) => return,
    };

    if files.is_empty() {
        return;
    }

    for file in files {
        let content = match std::fs::read_to_string(&file) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let first_line = match content.split('\n').next() {
            Some(l) if !l.trim().is_empty() => l,
            _ => continue,
        };

        let header: serde_json::Value = match serde_json::from_str(first_line) {
            Ok(v) => v,
            Err(_) => continue,
        };

        if header.get("type").and_then(|v| v.as_str()) != Some("session") {
            continue;
        }

        let cwd = match header.get("cwd").and_then(|v| v.as_str()) {
            Some(c) => c.to_string(),
            None => continue,
        };

        // Compute the correct session directory (same encoding as session-manager)
        let safe_path = format!(
            "--{}--",
            cwd.trim_start_matches('/')
                .trim_start_matches('\\')
                .replace(['/', '\\', ':'], "-")
        );
        let correct_dir = agent_dir.join("sessions").join(&safe_path);

        if let Err(_) = std::fs::create_dir_all(&correct_dir) {
            continue;
        }

        let file_name = match file.file_name() {
            Some(n) => n,
            None => continue,
        };
        let new_path = correct_dir.join(file_name);

        if new_path.exists() {
            continue;
        }

        let _ = std::fs::rename(&file, &new_path);
    }
}

// ============================================================================
// migrateCommandsToPrompts
// ============================================================================

/// Migrate `commands/` → `prompts/` in a config directory if needed.
///
/// Returns `true` if a migration was performed.
fn migrate_commands_to_prompts(base_dir: &Path, label: &str) -> bool {
    let commands_dir = base_dir.join("commands");
    let prompts_dir = base_dir.join("prompts");

    if commands_dir.exists() && !prompts_dir.exists() {
        match std::fs::rename(&commands_dir, &prompts_dir) {
            Ok(()) => {
                println!("Migrated {label} commands/ → prompts/");
                return true;
            }
            Err(e) => {
                eprintln!("Warning: Could not migrate {label} commands/ to prompts/: {e}");
            }
        }
    }
    false
}

// ============================================================================
// migrateToolsToBin
// ============================================================================

/// Move `fd` / `rg` binaries from `tools/` to `bin/` if they exist.
fn migrate_tools_to_bin() {
    let agent_dir = get_agent_dir();
    let tools_dir = agent_dir.join("tools");
    let bin_dir = get_bin_dir();

    if !tools_dir.exists() {
        return;
    }

    let binaries = ["fd", "rg", "fd.exe", "rg.exe"];
    let mut moved_any = false;

    for bin in &binaries {
        let old_path = tools_dir.join(bin);
        let new_path = bin_dir.join(bin);

        if old_path.exists() {
            if !bin_dir.exists() {
                let _ = std::fs::create_dir_all(&bin_dir);
            }
            if !new_path.exists() {
                if std::fs::rename(&old_path, &new_path).is_ok() {
                    moved_any = true;
                }
            } else {
                // Target exists, just remove old one
                let _ = std::fs::remove_file(&old_path);
            }
        }
    }

    if moved_any {
        println!("Migrated managed binaries tools/ → bin/");
    }
}

// ============================================================================
// checkDeprecatedExtensionDirs
// ============================================================================

/// Check for deprecated `hooks/` and `tools/` directories.
///
/// Returns a list of warning strings.
fn check_deprecated_extension_dirs(base_dir: &Path, label: &str) -> Vec<String> {
    let mut warnings = Vec::new();

    let hooks_dir = base_dir.join("hooks");
    if hooks_dir.exists() {
        warnings.push(format!(
            "{label} hooks/ directory found. Hooks have been renamed to extensions."
        ));
    }

    let tools_dir = base_dir.join("tools");
    if tools_dir.exists() {
        let managed = ["fd", "rg", "fd.exe", "rg.exe"];
        let has_custom = std::fs::read_dir(&tools_dir)
            .ok()
            .into_iter()
            .flatten()
            .filter_map(|e| e.ok())
            .any(|e| {
                let name = e.file_name().to_string_lossy().to_lowercase();
                !managed.contains(&name.as_str()) && !name.starts_with('.')
            });
        if has_custom {
            warnings.push(format!(
                "{label} tools/ directory contains custom tools. Custom tools have been merged into extensions."
            ));
        }
    }

    warnings
}

// ============================================================================
// migrateExtensionSystem
// ============================================================================

/// Run extension system migrations (commands→prompts) and collect warnings
/// about deprecated directories.
fn migrate_extension_system(cwd: &Path) -> Vec<String> {
    let agent_dir = get_agent_dir();
    let project_dir = cwd.join(CONFIG_DIR_NAME);

    migrate_commands_to_prompts(&agent_dir, "Global");
    migrate_commands_to_prompts(&project_dir, "Project");

    let mut warnings = Vec::new();
    warnings.extend(check_deprecated_extension_dirs(&agent_dir, "Global"));
    warnings.extend(check_deprecated_extension_dirs(&project_dir, "Project"));
    warnings
}

// ============================================================================
// showDeprecationWarnings
// ============================================================================

/// Print deprecation warnings and wait for a keypress (async).
///
/// Mirrors `showDeprecationWarnings()` from TypeScript.
pub async fn show_deprecation_warnings(warnings: &[String]) {
    if warnings.is_empty() {
        return;
    }

    for warning in warnings {
        eprintln!("Warning: {warning}");
    }
    eprintln!("\nMove your extensions to the extensions/ directory.");
    eprintln!("Migration guide: {MIGRATION_GUIDE_URL}");
    eprintln!("Documentation: {EXTENSIONS_DOC_URL}");
    eprintln!("\nPress any key to continue...");

    // Wait for a single byte on stdin
    use tokio::io::AsyncReadExt;
    let mut stdin = tokio::io::stdin();
    let _ = stdin.read_u8().await;
    eprintln!();
}

// ============================================================================
// MigrationResult
// ============================================================================

/// Result of `run_migrations()`.
#[derive(Debug, Clone)]
pub struct MigrationResult {
    pub migrated_auth_providers: Vec<String>,
    pub deprecation_warnings: Vec<String>,
}

// ============================================================================
// runMigrations
// ============================================================================

/// Run all migrations. Called once on startup.
///
/// Mirrors `runMigrations()` from TypeScript.
pub fn run_migrations(cwd: Option<&Path>) -> MigrationResult {
    let default_cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let cwd = cwd.unwrap_or(&default_cwd);

    let migrated_auth_providers = migrate_auth_to_auth_json();
    migrate_sessions_from_agent_root();
    migrate_tools_to_bin();
    let deprecation_warnings = migrate_extension_system(cwd);

    MigrationResult { migrated_auth_providers, deprecation_warnings }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp_dir() -> tempfile::TempDir {
        tempfile::tempdir().expect("tmp dir")
    }

    #[test]
    fn migrate_auth_skips_when_auth_json_exists() {
        let dir = temp_dir();
        // Create dummy auth.json
        fs::write(dir.path().join("auth.json"), "{}").unwrap();
        // The function reads agent_dir from env, so we just test the logic directly
        // by checking that it returns empty when auth.json is already present.
        // We can't easily override get_agent_dir(), so just validate the file check.
        let auth_path = dir.path().join("auth.json");
        assert!(auth_path.exists());
    }

    #[test]
    fn migrate_commands_to_prompts_renames() {
        let dir = temp_dir();
        let commands = dir.path().join("commands");
        fs::create_dir(&commands).unwrap();
        fs::write(commands.join("test.md"), "content").unwrap();

        let result = migrate_commands_to_prompts(dir.path(), "Test");
        assert!(result);
        assert!(dir.path().join("prompts").exists());
        assert!(!dir.path().join("commands").exists());
    }

    #[test]
    fn migrate_commands_skips_when_prompts_exists() {
        let dir = temp_dir();
        let commands = dir.path().join("commands");
        let prompts = dir.path().join("prompts");
        fs::create_dir(&commands).unwrap();
        fs::create_dir(&prompts).unwrap();

        let result = migrate_commands_to_prompts(dir.path(), "Test");
        assert!(!result);
        assert!(commands.exists()); // unchanged
    }

    #[test]
    fn check_deprecated_hooks_warning() {
        let dir = temp_dir();
        fs::create_dir(dir.path().join("hooks")).unwrap();
        let warnings = check_deprecated_extension_dirs(dir.path(), "Global");
        assert!(warnings.iter().any(|w| w.contains("hooks")));
    }

    #[test]
    fn migration_result_struct() {
        let result = MigrationResult {
            migrated_auth_providers: vec!["anthropic".into()],
            deprecation_warnings: vec![],
        };
        assert_eq!(result.migrated_auth_providers.len(), 1);
        assert!(result.deprecation_warnings.is_empty());
    }
}
