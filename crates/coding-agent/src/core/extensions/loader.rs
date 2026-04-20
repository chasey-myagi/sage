//! Extension loader.
//!
//! Translated from pi-mono `packages/coding-agent/src/core/extensions/loader.ts`.
//!
//! In the TypeScript version, extensions are loaded dynamically as TypeScript
//! modules via jiti. In Rust, extensions are registered at compile time as
//! closures / trait objects. This module provides the runtime state factory
//! and a path-resolution utility.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use super::types::{
    ExtensionRuntimeState, LoadExtensionsResult, SourceInfo, SourceOrigin, SourceScope,
};

// ============================================================================
// Runtime factory
// ============================================================================

/// Create a fresh `ExtensionRuntimeState` with no registered flags or pending
/// provider registrations. Mirrors `createExtensionRuntime()` in TypeScript.
pub fn create_extension_runtime() -> ExtensionRuntimeState {
    ExtensionRuntimeState {
        flag_values: HashMap::new(),
        pending_provider_registrations: Vec::new(),
    }
}

// ============================================================================
// Path utilities
// ============================================================================

/// Expand `~` in a path string (like `expandPath` in TypeScript).
pub fn expand_path(p: &str) -> PathBuf {
    let normalized = p.trim();
    if normalized == "~" {
        return dirs::home_dir().unwrap_or_else(|| PathBuf::from("~"));
    }
    if let Some(rest) = normalized.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest);
        }
    }
    PathBuf::from(normalized)
}

/// Resolve an extension path relative to `cwd` (mirrors `resolvePath`).
pub fn resolve_path(ext_path: &str, cwd: &Path) -> PathBuf {
    let expanded = expand_path(ext_path);
    if expanded.is_absolute() {
        expanded
    } else {
        cwd.join(expanded)
    }
}

/// Check whether a file name looks like a TypeScript/JavaScript extension file.
pub fn is_extension_file(name: &str) -> bool {
    name.ends_with(".ts") || name.ends_with(".js")
}

// ============================================================================
// Synthetic source info helper
// ============================================================================

pub fn create_synthetic_source_info(
    path: &str,
    source: &str,
    scope: Option<SourceScope>,
    origin: Option<SourceOrigin>,
    base_dir: Option<String>,
) -> SourceInfo {
    SourceInfo {
        path: path.to_string(),
        source: source.to_string(),
        scope: scope.unwrap_or(SourceScope::Temporary),
        origin: origin.unwrap_or(SourceOrigin::TopLevel),
        base_dir,
    }
}

// ============================================================================
// Extension discovery (filesystem scan)
// ============================================================================

/// Discover extension entry points from a directory.
///
/// Mirrors `resolveExtensionEntries` + `discoverExtensionsInDir` from TypeScript.
/// Returns resolved paths to extension files.
pub fn discover_extensions_in_dir(dir: &Path) -> Vec<PathBuf> {
    if !dir.exists() {
        return Vec::new();
    }

    let mut discovered = Vec::new();

    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return Vec::new(),
    };

    for entry in entries.flatten() {
        let entry_path = entry.path();
        let name = entry.file_name();
        let name_str = name.to_string_lossy();

        if entry_path.is_file() && is_extension_file(&name_str) {
            discovered.push(entry_path);
            continue;
        }

        if entry_path.is_dir() {
            // Check for index.ts / index.js
            let index_ts = entry_path.join("index.ts");
            let index_js = entry_path.join("index.js");
            if index_ts.exists() {
                discovered.push(index_ts);
            } else if index_js.exists() {
                discovered.push(index_js);
            }
        }
    }

    discovered
}

/// Discover and load extensions from standard locations.
///
/// Mirrors `discoverAndLoadExtensions` from TypeScript (minus the actual dynamic loading
/// which is TypeScript-specific). Returns the paths that would be loaded.
pub fn discover_extension_paths(
    configured_paths: &[String],
    cwd: &Path,
    agent_dir: &Path,
) -> Vec<PathBuf> {
    let mut all_paths: Vec<PathBuf> = Vec::new();
    let mut seen = std::collections::HashSet::new();

    let mut add_paths = |paths: Vec<PathBuf>| {
        for p in paths {
            if seen.insert(p.clone()) {
                all_paths.push(p);
            }
        }
    };

    // 1. Project-local extensions: cwd/.pi/extensions/
    let local_ext_dir = cwd.join(".pi").join("extensions");
    add_paths(discover_extensions_in_dir(&local_ext_dir));

    // 2. Global extensions: agentDir/extensions/
    let global_ext_dir = agent_dir.join("extensions");
    add_paths(discover_extensions_in_dir(&global_ext_dir));

    // 3. Explicitly configured paths
    for p in configured_paths {
        let resolved = resolve_path(p, cwd);
        if resolved.is_dir() {
            add_paths(discover_extensions_in_dir(&resolved));
        } else {
            add_paths(vec![resolved]);
        }
    }

    all_paths
}

/// Create an empty `LoadExtensionsResult` with the given runtime state.
/// Used when no dynamic extension loading is needed (pure Rust mode).
pub fn load_extensions_empty(runtime: ExtensionRuntimeState) -> LoadExtensionsResult {
    LoadExtensionsResult {
        extensions: Vec::new(),
        errors: Vec::new(),
        runtime,
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    // ── extensions-discovery.test.ts (path resolution + is_extension_file) ──

    #[test]
    fn test_is_extension_file_ts() {
        assert!(is_extension_file("foo.ts"));
        assert!(is_extension_file("bar.ts"));
    }

    #[test]
    fn test_is_extension_file_js() {
        assert!(is_extension_file("foo.js"));
    }

    #[test]
    fn test_is_extension_file_rejects_other() {
        assert!(!is_extension_file("foo.rs"));
        assert!(!is_extension_file("foo.json"));
        assert!(!is_extension_file("foo"));
        assert!(!is_extension_file("foobar")); // no extension
    }

    #[test]
    fn test_expand_path_home() {
        let result = expand_path("~");
        let home = dirs::home_dir().unwrap_or_default();
        assert_eq!(result, home);
    }

    #[test]
    fn test_expand_path_home_subdir() {
        let result = expand_path("~/projects");
        let home = dirs::home_dir().unwrap_or_default();
        assert_eq!(result, home.join("projects"));
    }

    #[test]
    fn test_expand_path_absolute_unchanged() {
        let abs = PathBuf::from("/usr/local/bin/my-ext.ts");
        let result = expand_path("/usr/local/bin/my-ext.ts");
        assert_eq!(result, abs);
    }

    #[test]
    fn test_resolve_path_absolute() {
        let cwd = Path::new("/tmp/project");
        let result = resolve_path("/absolute/ext.ts", cwd);
        assert_eq!(result, PathBuf::from("/absolute/ext.ts"));
    }

    #[test]
    fn test_resolve_path_relative() {
        let cwd = Path::new("/tmp/project");
        let result = resolve_path("relative/ext.ts", cwd);
        assert_eq!(result, PathBuf::from("/tmp/project/relative/ext.ts"));
    }

    #[test]
    fn test_discover_extensions_in_dir_empty() {
        let dir = TempDir::new().unwrap();
        let found = discover_extensions_in_dir(dir.path());
        assert!(found.is_empty());
    }

    #[test]
    fn test_discover_extensions_in_dir_nonexistent() {
        let path = PathBuf::from("/nonexistent-dir-for-sage-test-xyz");
        let found = discover_extensions_in_dir(&path);
        assert!(found.is_empty());
    }

    #[test]
    fn test_discover_extensions_in_dir_finds_ts_files() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("foo.ts"), "// ext").unwrap();
        fs::write(dir.path().join("bar.ts"), "// ext").unwrap();
        fs::write(dir.path().join("ignored.rs"), "// not ext").unwrap();

        let mut found: Vec<String> = discover_extensions_in_dir(dir.path())
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
            .collect();
        found.sort();

        assert_eq!(found, vec!["bar.ts", "foo.ts"]);
    }

    #[test]
    fn test_discover_extensions_in_dir_finds_js_files() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("ext.js"), "// ext").unwrap();

        let found = discover_extensions_in_dir(dir.path());
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].file_name().unwrap().to_string_lossy(), "ext.js");
    }

    #[test]
    fn test_discover_extensions_in_dir_prefers_index_ts_over_js() {
        let dir = TempDir::new().unwrap();
        let sub = dir.path().join("my-ext");
        fs::create_dir(&sub).unwrap();
        fs::write(sub.join("index.ts"), "// ts").unwrap();
        fs::write(sub.join("index.js"), "// js").unwrap();

        let found = discover_extensions_in_dir(dir.path());
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].file_name().unwrap().to_string_lossy(), "index.ts");
    }

    #[test]
    fn test_discover_extensions_in_dir_finds_subdir_index_ts() {
        let dir = TempDir::new().unwrap();
        let sub = dir.path().join("my-extension");
        fs::create_dir(&sub).unwrap();
        fs::write(sub.join("index.ts"), "// ext").unwrap();

        let found = discover_extensions_in_dir(dir.path());
        assert_eq!(found.len(), 1);
        let p = &found[0];
        assert!(p.to_string_lossy().contains("my-extension"));
        assert_eq!(p.file_name().unwrap().to_string_lossy(), "index.ts");
    }

    #[test]
    fn test_discover_extensions_in_dir_ignores_subdir_without_index() {
        let dir = TempDir::new().unwrap();
        let sub = dir.path().join("not-an-extension");
        fs::create_dir(&sub).unwrap();
        fs::write(sub.join("helper.ts"), "// helper").unwrap();

        let found = discover_extensions_in_dir(dir.path());
        assert!(found.is_empty());
    }

    #[test]
    fn test_discover_extensions_in_dir_mixed_direct_and_subdir() {
        let dir = TempDir::new().unwrap();
        // Direct file
        fs::write(dir.path().join("direct.ts"), "// ext").unwrap();
        // Subdirectory with index
        let sub = dir.path().join("with-index");
        fs::create_dir(&sub).unwrap();
        fs::write(sub.join("index.ts"), "// ext").unwrap();

        let found = discover_extensions_in_dir(dir.path());
        assert_eq!(found.len(), 2);
    }

    #[test]
    fn test_create_extension_runtime_is_empty() {
        let rt = create_extension_runtime();
        assert!(rt.flag_values.is_empty());
        assert!(rt.pending_provider_registrations.is_empty());
    }

    #[test]
    fn test_load_extensions_empty_result() {
        let rt = create_extension_runtime();
        let result = load_extensions_empty(rt);
        assert!(result.extensions.is_empty());
        assert!(result.errors.is_empty());
    }
}
