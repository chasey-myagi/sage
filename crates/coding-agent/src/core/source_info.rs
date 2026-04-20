//! Source info types and constructors.
//!
//! Translated from pi-mono `packages/coding-agent/src/core/source-info.ts`.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SourceScope {
    User,
    Project,
    Temporary,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum SourceOrigin {
    Package,
    TopLevel,
}

/// Metadata about where a resource (skill, extension, prompt, theme) was loaded from.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceInfo {
    pub path: String,
    pub source: String,
    pub scope: SourceScope,
    pub origin: SourceOrigin,
    pub base_dir: Option<String>,
}

/// Create a `SourceInfo` for a dynamically-created (synthetic) resource.
///
/// Mirrors `createSyntheticSourceInfo()` from TypeScript.
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
