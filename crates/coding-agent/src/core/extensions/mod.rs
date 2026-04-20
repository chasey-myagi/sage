//! Extension system for lifecycle events and custom tools.
//!
//! Translated from pi-mono `packages/coding-agent/src/core/extensions/index.ts`.

pub mod loader;
pub mod runner;
pub mod types;
pub mod wrapper;

// Re-export commonly used items
pub use loader::{
    create_extension_runtime, discover_extension_paths, discover_extensions_in_dir,
    expand_path, resolve_path,
};
pub use runner::{emit_session_shutdown_event, ExtensionData, ExtensionRunner};
pub use types::{
    BeforeAgentStartEventResult, ExtensionContextSnapshot, ExtensionError, ExtensionFlag,
    ExtensionRuntimeState, FlagType, FlagValue, InputEventResult, InputSource, LoadExtensionError,
    LoadExtensionsResult, PendingProviderRegistration, ProviderConfig, ProviderModelConfig,
    RegisteredCommand, RegisteredTool, ResolvedCommand, ResourcesDiscoverResult,
    SessionBeforeCompactResult, SessionBeforeForkResult, SessionBeforeSwitchResult,
    SessionBeforeTreeResult, SessionDirectoryResult, SourceInfo, SourceOrigin, SourceScope,
    ToolDefinition, ToolInfo,
};
pub use wrapper::{wrap_registered_tool, wrap_registered_tools, WrappedTool};
