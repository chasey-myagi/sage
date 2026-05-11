//! Diagnostic types for resource loading.
//!
//! Translated from pi-mono `packages/coding-agent/src/core/diagnostics.ts`.

use serde::{Deserialize, Serialize};

/// Type of resource that caused a collision.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ResourceType {
    Extension,
    Skill,
    Prompt,
    Theme,
}

/// Records a naming collision between two loaded resources.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceCollision {
    pub resource_type: ResourceType,
    /// Skill name, command/tool/flag name, prompt name, or theme name.
    pub name: String,
    pub winner_path: String,
    pub loser_path: String,
    /// E.g. `"npm:foo"`, `"git:..."`, `"local"`.
    pub winner_source: Option<String>,
    pub loser_source: Option<String>,
}

/// Severity of a diagnostic entry.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum DiagnosticType {
    Warning,
    Error,
    Collision,
}

/// A single diagnostic message from resource loading.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceDiagnostic {
    #[serde(rename = "type")]
    pub diagnostic_type: DiagnosticType,
    pub message: String,
    pub path: Option<String>,
    pub collision: Option<ResourceCollision>,
}

impl ResourceDiagnostic {
    pub fn warning(message: impl Into<String>, path: Option<String>) -> Self {
        Self {
            diagnostic_type: DiagnosticType::Warning,
            message: message.into(),
            path,
            collision: None,
        }
    }

    pub fn error(message: impl Into<String>, path: Option<String>) -> Self {
        Self {
            diagnostic_type: DiagnosticType::Error,
            message: message.into(),
            path,
            collision: None,
        }
    }

    pub fn collision(collision: ResourceCollision) -> Self {
        let message = format!(
            "Name collision: '{}' between '{}' and '{}'",
            collision.name, collision.winner_path, collision.loser_path
        );
        Self {
            diagnostic_type: DiagnosticType::Collision,
            message,
            path: None,
            collision: Some(collision),
        }
    }
}
