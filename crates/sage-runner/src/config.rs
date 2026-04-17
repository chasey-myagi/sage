use serde::{Deserialize, Serialize};
use std::path::Path;

use sage_runtime::tools::policy::ToolPolicy;

/// Return the current user's home directory, checking `HOME` then `USERPROFILE`.
///
/// Returns `None` when neither environment variable is set (rare in practice,
/// possible in minimal container environments).
pub fn home_dir() -> Option<std::path::PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(std::path::PathBuf::from)
}

/// Expand a leading `~` in a path string to the user's home directory.
/// Returns the original string unchanged if it doesn't start with `~` or
/// if the home directory cannot be determined.
///
/// Only bare `~` and `~/…` are expanded. The `~username/…` form is returned
/// as-is.
fn expand_tilde(path: &str) -> String {
    let Some(home) = home_dir() else {
        return path.to_string();
    };
    if let Some(rest) = path.strip_prefix("~/") {
        return home.join(rest).to_string_lossy().into_owned();
    }
    if path == "~" {
        return home.to_string_lossy().into_owned();
    }
    path.to_string()
}

/// Top-level agent configuration, parsed from `config.yaml`.
///
/// Stored at `~/.sage/agents/<name>/config.yaml` and loaded by
/// [`crate::serve::load_agent_config`][sage_cli].
#[derive(Debug, Serialize, Deserialize)]
pub struct AgentConfig {
    /// Unique identifier for this agent (used as the daemon service name).
    pub name: String,
    /// Human-readable purpose description shown in `sage list`.
    pub description: String,
    /// LLM provider and model selection.
    pub llm: LlmConfig,
    /// Base system prompt. Memory and skill sections are prepended/appended at runtime.
    pub system_prompt: String,
    /// Tool access policy.
    pub tools: ToolsConfig,
    /// Operational limits (max turns, timeout).
    pub constraints: Constraints,
    /// Sandbox VM configuration. `None` ⟹ host mode (equivalent to `--dev`).
    #[serde(default)]
    pub sandbox: Option<SandboxConfig>,
    /// Memory and knowledge injection configuration.
    #[serde(default)]
    pub memory: Option<MemoryConfig>,
    /// Hook commands for agent lifecycle events (pre/post tool use, stop).
    #[serde(default)]
    pub hooks: Option<HooksConfig>,
    /// Harness evaluator configuration for `sage test`.
    #[serde(default)]
    pub harness: Option<HarnessConfig>,
    /// Wiki self-maintenance configuration (Sprint 7).
    #[serde(default)]
    pub wiki: Option<WikiConfig>,
    /// Outbound/inbound channel adapter (Sprint 8).
    /// `None` means the agent runs headless (CLI / daemon socket only).
    #[serde(default)]
    pub channel: Option<ChannelConfig>,
}

/// LLM provider and model selection.
#[derive(Debug, Serialize, Deserialize)]
pub struct LlmConfig {
    /// Provider name as recognised by the LLM registry (e.g. `"anthropic"`, `"openai"`).
    pub provider: String,
    /// Model identifier (e.g. `"claude-opus-4-6"`, `"gpt-4o"`).
    pub model: String,
    /// Maximum tokens to generate per turn.
    ///
    /// `None` ⇒ use `ProviderSpec.default_max_tokens` (Sprint 12 M1). The
    /// ProviderSpec default is guaranteed non-zero by a test invariant, so
    /// `None` never means "unlimited" — it means "defer to the per-provider
    /// default".
    #[serde(default)]
    pub max_tokens: Option<u32>,
    /// Context window size in tokens (used to size the token budget for
    /// compaction).
    ///
    /// `None` ⇒ use `ProviderSpec.default_context_window` (Sprint 12 M1,
    /// non-zero by invariant). Compaction stays enabled either way — to
    /// disable compaction, supply a custom `ContextBudget` via the engine
    /// builder rather than leaving this field off.
    #[serde(default)]
    pub context_window: Option<u32>,
    /// Override base URL for the LLM API endpoint.
    #[serde(default)]
    pub base_url: Option<String>,
    /// Override environment variable name for the API key.
    #[serde(default)]
    pub api_key_env: Option<String>,
}

// ── Provider validation (M1) ──────────────────────────────────────────────

/// Validate that the given provider string is one of the known providers
/// listed in `sage_runtime::llm::provider_specs::list_providers()`.
/// Returns `Ok(())` on success, `Err` with a helpful message listing valid providers.
///
/// Called from `load_agent_config` / `validate_agent` / `run_local_test` in
/// `sage-cli::serve` after YAML parsing. The error message lists candidates
/// sorted alphabetically so users can scan the list for the correct spelling.
pub fn validate_provider(provider: &str) -> Result<(), String> {
    if sage_runtime::llm::provider_specs::resolve_provider(provider).is_some() {
        return Ok(());
    }
    let mut valid: Vec<&str> = sage_runtime::llm::provider_specs::list_providers()
        .iter()
        .map(|s| s.id)
        .collect();
    valid.sort_unstable();
    Err(format!(
        "unknown provider '{}'; valid providers: {}",
        provider,
        valid.join(", ")
    ))
}

/// Execution mode for the sandbox.
///
/// - `Microvm` — run the agent inside a msb_krun microVM (default, production)
/// - `Host` — run the agent directly as a host process (dev/test mode, no isolation).
///   YAML key: `host`. Enabled by `--dev` flag or `mode: host` in config.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum SandboxMode {
    #[default]
    Microvm,
    Host,
}

impl SandboxMode {
    /// Return `SandboxMode::Host` when `dev` is true, otherwise return `self` unchanged.
    ///
    /// This is the semantic bridge between the CLI `--dev` flag and the runtime
    /// `SandboxMode`: `--dev` bypasses the microVM regardless of what the config says.
    pub fn with_dev_override(self, dev: bool) -> Self {
        if dev { SandboxMode::Host } else { self }
    }
}

/// Rootfs image tier for the sandbox VM.
///
/// - `Minimal` — bare-minimum rootfs (default)
/// - `Standard` — curl, python3, jq, git, rg, fd, gh pre-installed
///
/// A `Custom` tier (user-supplied OCI image path) is planned for v1.x.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum RootfsTier {
    #[default]
    Minimal,
    Standard,
}

/// Network access policy for the sandbox.
///
/// Accepts both boolean and string values in YAML for backward compatibility:
/// - `true` → `Full`, `false` → `Airgapped`
/// - `"airgapped"` / `"whitelist"` / `"full"` (string form)
#[derive(Debug, Clone, PartialEq)]
pub enum NetworkPolicy {
    /// No network access (default). Guest is completely isolated.
    Airgapped,
    /// Network restricted to specific hosts via msb_krun TSI whitelist.
    Whitelist,
    /// Unrestricted network access (debugging only).
    Full,
}

impl Default for NetworkPolicy {
    fn default() -> Self {
        NetworkPolicy::Airgapped
    }
}

impl Serialize for NetworkPolicy {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match self {
            NetworkPolicy::Airgapped => serializer.serialize_str("airgapped"),
            NetworkPolicy::Whitelist => serializer.serialize_str("whitelist"),
            NetworkPolicy::Full => serializer.serialize_str("full"),
        }
    }
}

impl<'de> Deserialize<'de> for NetworkPolicy {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Helper {
            Bool(bool),
            String(String),
        }

        match Helper::deserialize(deserializer)? {
            Helper::Bool(true) => Ok(NetworkPolicy::Full),
            Helper::Bool(false) => Ok(NetworkPolicy::Airgapped),
            Helper::String(s) => match s.as_str() {
                "airgapped" => Ok(NetworkPolicy::Airgapped),
                "whitelist" => Ok(NetworkPolicy::Whitelist),
                "full" => Ok(NetworkPolicy::Full),
                other => Err(serde::de::Error::custom(format!(
                    "unknown network policy: {other} (expected airgapped, whitelist, or full)"
                ))),
            },
        }
    }
}

/// Security hardening configuration for the guest VM.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SecurityConfig {
    /// Enable seccomp-bpf syscall filtering (Linux only).
    #[serde(default = "default_true")]
    pub seccomp: bool,
    /// Enable Landlock LSM filesystem access control (Linux only).
    #[serde(default = "default_true")]
    pub landlock: bool,
    /// Maximum file size in MB that can be written inside the sandbox.
    #[serde(default = "default_max_file_size_mb")]
    pub max_file_size_mb: u32,
    /// Maximum number of open file descriptors.
    #[serde(default = "default_max_open_files")]
    pub max_open_files: u32,
    /// tmpfs size limit in MB.
    #[serde(default = "default_tmpfs_size_mb")]
    pub tmpfs_size_mb: u32,
    /// Maximum number of processes (RLIMIT_NPROC). Prevents fork bombs.
    #[serde(default = "default_max_processes")]
    pub max_processes: u32,
}

impl Default for SecurityConfig {
    fn default() -> Self {
        Self {
            seccomp: default_true(),
            landlock: default_true(),
            max_file_size_mb: default_max_file_size_mb(),
            max_open_files: default_max_open_files(),
            tmpfs_size_mb: default_tmpfs_size_mb(),
            max_processes: default_max_processes(),
        }
    }
}

fn default_true() -> bool {
    true
}

fn default_max_file_size_mb() -> u32 {
    100
}

fn default_max_open_files() -> u32 {
    256
}

fn default_tmpfs_size_mb() -> u32 {
    512
}

fn default_max_processes() -> u32 {
    256
}

fn default_exec_timeout_secs() -> u32 {
    30
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SandboxConfig {
    /// Sandbox execution mode. `microvm` (default) runs in a VM; `host` runs on the host
    /// directly (dev/CI mode with no isolation).
    #[serde(default)]
    pub mode: SandboxMode,
    /// Rootfs image tier. `minimal` (default) is bare-bones; `standard` includes
    /// curl, python3, jq, git, rg, fd, gh.
    #[serde(default)]
    pub rootfs: RootfsTier,
    #[serde(default = "default_cpus")]
    pub cpus: u32,
    #[serde(default = "default_memory_mib")]
    pub memory_mib: u32,
    /// Network access policy. Accepts bool for backward compatibility:
    /// `true` → Full, `false` → Airgapped. String values: airgapped, whitelist, full.
    #[serde(default)]
    pub network: NetworkPolicy,
    /// Allowed hosts for whitelist network policy (ignored for other policies).
    #[serde(default)]
    pub allowed_hosts: Vec<String>,
    /// Security hardening settings (seccomp, landlock, resource limits).
    #[serde(default)]
    pub security: SecurityConfig,
    /// Per-command execution timeout in seconds.
    #[serde(default = "default_exec_timeout_secs")]
    pub exec_timeout_secs: u32,
    /// Host-side workspace directory mounted read-write into the sandbox at `/workspace`.
    /// A leading `~` is expanded to the user home directory on deserialization.
    #[serde(default, deserialize_with = "deserialize_workspace_host")]
    pub workspace_host: Option<std::path::PathBuf>,
}

fn deserialize_workspace_host<'de, D>(
    deserializer: D,
) -> Result<Option<std::path::PathBuf>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let opt: Option<String> = Option::deserialize(deserializer)?;
    Ok(opt.map(|s| std::path::PathBuf::from(expand_tilde(&s))))
}

fn default_cpus() -> u32 {
    1
}

fn default_memory_mib() -> u32 {
    512
}

/// Predefined toolset presets for common use cases.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Toolset {
    /// All 7 tools, bash allowed_binaries = ["*"].
    Coding,
    /// bash + read + grep + find + ls (no write/edit). Common ops binaries.
    Ops,
    /// bash + read. HTTP-focused binaries (curl, wget, jq, python).
    Web,
    /// read + grep + find + ls only. No bash, no write, no edit.
    Minimal,
    /// Same as Minimal — read-only access.
    Readonly,
}

/// Preset allowed binaries for ops toolset.
const OPS_BINARIES: &[&str] = &[
    "curl",
    "wget",
    "docker",
    "kubectl",
    "ssh",
    "scp",
    "systemctl",
    "journalctl",
    "ps",
    "top",
    "htop",
    "df",
    "du",
    "free",
    "netstat",
    "ss",
    "ping",
    "dig",
    "nslookup",
    "ip",
    "iptables",
    "tar",
    "gzip",
    "zip",
    "unzip",
    "jq",
    "yq",
    "awk",
    "sed",
];

/// Preset allowed binaries for web toolset.
const WEB_BINARIES: &[&str] = &["curl", "wget", "jq", "python", "python3", "node"];

fn strs_to_strings(strs: &[&str]) -> Vec<String> {
    strs.iter().map(|s| s.to_string()).collect()
}

/// An unrestricted PathToolConfig (empty allowed_paths means the tool is
/// enabled but path restriction is delegated to the ToolPolicy layer —
/// an empty list in the policy means default-deny at enforcement time).
fn unrestricted_path_tool() -> PathToolConfig {
    PathToolConfig {
        allowed_paths: vec![],
    }
}

impl Toolset {
    /// Expand a preset into a concrete ToolsConfig with default settings.
    pub fn expand(&self) -> ToolsConfig {
        match self {
            Toolset::Coding => ToolsConfig {
                toolset: None,
                bash: Some(BashToolConfig {
                    allowed_binaries: vec!["*".to_string()],
                }),
                read: Some(unrestricted_path_tool()),
                write: Some(unrestricted_path_tool()),
                edit: Some(unrestricted_path_tool()),
                grep: Some(EmptyToolConfig {}),
                find: Some(EmptyToolConfig {}),
                ls: Some(EmptyToolConfig {}),
            },
            Toolset::Ops => ToolsConfig {
                toolset: None,
                bash: Some(BashToolConfig {
                    allowed_binaries: strs_to_strings(OPS_BINARIES),
                }),
                read: Some(unrestricted_path_tool()),
                write: None,
                edit: None,
                grep: Some(EmptyToolConfig {}),
                find: Some(EmptyToolConfig {}),
                ls: Some(EmptyToolConfig {}),
            },
            Toolset::Web => ToolsConfig {
                toolset: None,
                bash: Some(BashToolConfig {
                    allowed_binaries: strs_to_strings(WEB_BINARIES),
                }),
                read: Some(unrestricted_path_tool()),
                write: None,
                edit: None,
                grep: None,
                find: None,
                ls: None,
            },
            Toolset::Minimal | Toolset::Readonly => ToolsConfig {
                toolset: None,
                bash: None,
                read: Some(unrestricted_path_tool()),
                write: None,
                edit: None,
                grep: Some(EmptyToolConfig {}),
                find: Some(EmptyToolConfig {}),
                ls: Some(EmptyToolConfig {}),
            },
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolsConfig {
    /// Optional toolset preset — provides defaults for tool fields below.
    /// Explicit tool fields override the preset.
    #[serde(default)]
    pub toolset: Option<Toolset>,
    #[serde(default)]
    pub bash: Option<BashToolConfig>,
    #[serde(default)]
    pub read: Option<PathToolConfig>,
    #[serde(default)]
    pub write: Option<PathToolConfig>,
    #[serde(default)]
    pub edit: Option<PathToolConfig>,
    #[serde(default)]
    pub grep: Option<EmptyToolConfig>,
    #[serde(default)]
    pub find: Option<EmptyToolConfig>,
    #[serde(default)]
    pub ls: Option<EmptyToolConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BashToolConfig {
    pub allowed_binaries: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PathToolConfig {
    pub allowed_paths: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmptyToolConfig {}

#[derive(Debug, Serialize, Deserialize)]
pub struct Constraints {
    pub max_turns: u32,
    pub timeout_secs: u32,
}

/// Determines how memory documents are injected into the agent context.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum MemoryInjectMode {
    /// Prepend memory content to the system prompt as a cacheable section.
    PrependSystem,
    /// Inject memory as the first user message in the conversation.
    InitialMessage,
}

/// Classifies the session type for metrics collection and behavioural branching.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum SessionType {
    /// Normal interactive session driven by a human user.
    UserDriven,
    /// Automated wiki maintenance and knowledge distillation session.
    WikiMaintenance,
    /// Craft/skill self-evaluation session.
    SkillEvaluation,
    /// Test harness evaluation run.
    HarnessRun,
}

impl SessionType {
    /// Stable PascalCase name for archive JSONL metadata.
    ///
    /// Archive wire format is distinct from config.yaml wire format: config.yaml
    /// uses snake_case (serde rename_all), archive JSONL uses this PascalCase.
    /// Both are explicit choices — don't derive archive names from `Debug`.
    ///
    /// **CAUTION: wire format invariant**
    /// These strings are the on-disk archive contract. They must NEVER be renamed —
    /// only new variants may be added. Renaming breaks the ability to read older
    /// archive files. Migration schema bumps live in the JSONL `version` field.
    pub fn archive_name(&self) -> &'static str {
        match self {
            SessionType::UserDriven => "UserDriven",
            SessionType::WikiMaintenance => "WikiMaintenance",
            SessionType::SkillEvaluation => "SkillEvaluation",
            SessionType::HarnessRun => "HarnessRun",
        }
    }
}

fn default_auto_load() -> Vec<String> {
    vec!["AGENT.md".to_string(), "memory/MEMORY.md".to_string()]
}

/// Configuration for a single hook command.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookConfig {
    /// Shell command string, executed via `/bin/sh -c <command>`.
    pub command: String,
    /// Per-hook execution timeout in seconds. Overrides any executor default.
    #[serde(default)]
    pub timeout_secs: Option<u32>,
}

/// Event-keyed hook configuration for an agent.
///
/// Each field maps an agent lifecycle event to an ordered list of hook commands.
/// An absent field deserializes as an empty `Vec` (no hooks for that event).
/// `hooks: {}` deserializes as `Some(HooksConfig { .. empty .. })`.
/// Omitting the `hooks` field entirely deserializes as `None`.
///
/// YAML keys are snake_case (e.g. `session_start`). The runtime `HookEvent`
/// enum exposes PascalCase names (e.g. `"SessionStart"`) via `HookEvent::name()`
/// which are passed to hook scripts — the two naming conventions are deliberate
/// (YAML reads human, script payloads read machine).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct HooksConfig {
    // ── Existing (Sprint 4) ────────────────────────────────────────────────
    /// Run before each tool call. Exit 2 blocks the call; stderr → steering message.
    #[serde(default)]
    pub pre_tool_use: Vec<HookConfig>,
    /// Run after each tool call completes.
    #[serde(default)]
    pub post_tool_use: Vec<HookConfig>,
    /// Run when the agent is about to stop. Exit 2 injects stderr as feedback and
    /// restarts the agent loop (the Harness mechanism).
    #[serde(default)]
    pub stop: Vec<HookConfig>,

    // ── New in Sprint 6 ────────────────────────────────────────────────────
    /// Run when an agent session starts (before the first turn).
    #[serde(default)]
    pub session_start: Vec<HookConfig>,
    /// Run when an agent session ends (after stop, regardless of restart).
    #[serde(default)]
    pub session_end: Vec<HookConfig>,
    /// Run when the user submits a new prompt (before agent processing).
    #[serde(default)]
    pub user_prompt_submit: Vec<HookConfig>,
    /// Run before context compaction.
    #[serde(default)]
    pub pre_compact: Vec<HookConfig>,
    /// Run after context compaction completes.
    #[serde(default)]
    pub post_compact: Vec<HookConfig>,
}

/// Harness evaluator configuration for `sage test`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HarnessConfig {
    /// Path to the evaluator script or inline shell expression.
    /// Run via `/bin/sh -c <evaluator>`.
    pub evaluator: String,
    /// Evaluator timeout in seconds.
    #[serde(default)]
    pub timeout_secs: Option<u32>,
}

/// Wiki self-maintenance configuration (Sprint 7).
///
/// When enabled, the daemon monitors archived sessions under
/// `<workspace>/raw/sessions/` and triggers a `WikiMaintenance` session
/// once the unprocessed-session count crosses `trigger_sessions`, subject
/// to the `cooldown_secs` rate limit.
///
/// Defaults are conservative (`enabled: false`) so that opting in is an
/// explicit YAML edit — otherwise an agent could silently burn LLM tokens
/// on background maintenance without the operator knowing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WikiConfig {
    /// Number of unprocessed archived sessions required to trigger a
    /// `WikiMaintenance` run. Defaults to 3.
    #[serde(default = "default_wiki_trigger_sessions")]
    pub trigger_sessions: u32,

    /// Minimum number of seconds between two consecutive `WikiMaintenance`
    /// runs. Defaults to 1800 (30 minutes).
    #[serde(default = "default_wiki_cooldown_secs")]
    pub cooldown_secs: u64,

    /// Whether wiki self-maintenance is enabled. Defaults to `false` —
    /// opt-in to avoid unexpected LLM spend.
    #[serde(default)]
    pub enabled: bool,
}

fn default_wiki_trigger_sessions() -> u32 {
    3
}
fn default_wiki_cooldown_secs() -> u64 {
    1800
}

/// Channel adapter configuration (Sprint 8).
///
/// A channel plugs the agent into an external messaging platform (Feishu,
/// Slack, …). The variant selects the platform; fields inside carry the
/// platform-specific credentials and webhook server settings.
///
/// Serialized form uses an explicit `type:` tag so future variants can be
/// added without breaking existing YAML:
///
/// ```yaml
/// channel:
///   type: feishu
///   app_id: cli_xxx
///   app_secret: ${FEISHU_APP_SECRET}
///   verification_token: optional
///   webhook_port: 3400
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ChannelConfig {
    /// Feishu / Lark channel.
    Feishu {
        /// App ID from the Feishu developer console.
        app_id: String,
        /// App secret. Supports `${ENV_VAR}` expansion at load time so
        /// secrets don't have to live in the YAML verbatim.
        app_secret: String,
        /// Verification token used to validate inbound webhook signatures
        /// (HMAC-SHA256).
        ///
        /// **PRODUCTION: always set this.** `None` disables signature
        /// checking and the webhook will accept any payload — dev/testing
        /// only. The handler logs a warning on every unverified request so
        /// the risk is visible in logs.
        #[serde(default)]
        verification_token: Option<String>,
        /// Local port the webhook HTTP server binds to.
        #[serde(default = "default_webhook_port")]
        webhook_port: u16,
    },
}

fn default_webhook_port() -> u16 {
    3400
}

impl Default for WikiConfig {
    fn default() -> Self {
        Self {
            trigger_sessions: default_wiki_trigger_sessions(),
            cooldown_secs: default_wiki_cooldown_secs(),
            enabled: false,
        }
    }
}

/// Memory and knowledge injection configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryConfig {
    /// Files to auto-load from the agent workspace and inject into context.
    /// Paths are relative to the workspace root. Defaults to `["AGENT.md",
    /// "memory/MEMORY.md"]`.
    #[serde(default = "default_auto_load")]
    pub auto_load: Vec<String>,
    /// How the loaded content is injected into the agent context.
    pub inject_as: MemoryInjectMode,
    /// Optional session type classification.
    #[serde(default)]
    pub session_type: Option<SessionType>,
}

impl ToolsConfig {
    /// Resolve the effective ToolsConfig by merging toolset defaults with
    /// explicit overrides. Explicit tool fields take precedence over the preset.
    fn resolved(&self) -> ToolsConfig {
        let preset = match &self.toolset {
            Some(ts) => ts.expand(),
            None => return self.clone(),
        };

        // For each tool field: use explicit (self) if Some, otherwise use preset
        ToolsConfig {
            toolset: None,
            bash: self.bash.clone().or(preset.bash),
            read: self.read.clone().or(preset.read),
            write: self.write.clone().or(preset.write),
            edit: self.edit.clone().or(preset.edit),
            grep: self.grep.clone().or(preset.grep),
            find: self.find.clone().or(preset.find),
            ls: self.ls.clone().or(preset.ls),
        }
    }

    /// Derive sandbox policy from tool configuration.
    pub fn to_policy(&self) -> ToolPolicy {
        let resolved = self.resolved();
        let mut allowed_binaries = Vec::new();
        let mut allowed_read_paths = Vec::new();
        let mut allowed_write_paths = Vec::new();

        if let Some(bash) = &resolved.bash {
            allowed_binaries.extend(bash.allowed_binaries.clone());
        }
        // grep/find/ls require their standard binaries
        if resolved.grep.is_some() {
            allowed_binaries.push("grep".into());
        }
        if resolved.find.is_some() {
            allowed_binaries.push("find".into());
        }
        if resolved.ls.is_some() {
            allowed_binaries.push("ls".into());
        }

        if let Some(read) = &resolved.read {
            allowed_read_paths.extend(read.allowed_paths.iter().map(|p| expand_tilde(p)));
        }
        if let Some(write) = &resolved.write {
            allowed_write_paths.extend(write.allowed_paths.iter().map(|p| expand_tilde(p)));
        }
        if let Some(edit) = &resolved.edit {
            // edit implies both read and write
            allowed_read_paths.extend(edit.allowed_paths.iter().map(|p| expand_tilde(p)));
            allowed_write_paths.extend(edit.allowed_paths.iter().map(|p| expand_tilde(p)));
        }

        ToolPolicy {
            allowed_binaries,
            allowed_read_paths,
            allowed_write_paths,
        }
    }

    /// Returns the list of enabled tool names.
    pub fn tool_names(&self) -> Vec<String> {
        let resolved = self.resolved();
        let mut names = Vec::new();
        if resolved.bash.is_some() {
            names.push("bash".into());
        }
        if resolved.read.is_some() {
            names.push("read".into());
        }
        if resolved.write.is_some() {
            names.push("write".into());
        }
        if resolved.edit.is_some() {
            names.push("edit".into());
        }
        if resolved.grep.is_some() {
            names.push("grep".into());
        }
        if resolved.find.is_some() {
            names.push("find".into());
        }
        if resolved.ls.is_some() {
            names.push("ls".into());
        }
        names
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FEISHU_YAML: &str = r#"
name: feishu-assistant
description: "飞书助手"
llm:
  provider: anthropic
  model: claude-haiku-4-5-20251001
  max_tokens: 4096
system_prompt: |
  你是飞书助手。
tools:
  bash:
    allowed_binaries: [feishu]
  read:
    allowed_paths: ["~/Documents/"]
constraints:
  max_turns: 10
  timeout_secs: 120
"#;

    #[test]
    fn parse_feishu_config() {
        let config: AgentConfig = serde_yaml::from_str(FEISHU_YAML).unwrap();
        assert_eq!(config.name, "feishu-assistant");
        assert_eq!(config.description, "飞书助手");
        assert_eq!(config.llm.provider, "anthropic");
        assert_eq!(config.llm.model, "claude-haiku-4-5-20251001");
        assert_eq!(config.llm.max_tokens, Some(4096));
        assert!(config.system_prompt.contains("飞书助手"));
        assert!(config.tools.bash.is_some());
        assert!(config.tools.read.is_some());
        assert!(config.tools.write.is_none());
        assert!(config.tools.edit.is_none());
        assert!(config.tools.grep.is_none());
        assert!(config.tools.find.is_none());
        assert!(config.tools.ls.is_none());
        assert_eq!(config.constraints.max_turns, 10);
        assert_eq!(config.constraints.timeout_secs, 120);
    }

    #[test]
    fn parse_all_tools_config() {
        let yaml = r#"
name: full-agent
description: "all tools enabled"
llm:
  provider: openai
  model: gpt-4
  max_tokens: 2048
system_prompt: "test"
tools:
  bash:
    allowed_binaries: [python, node]
  read:
    allowed_paths: ["/home/user/src"]
  write:
    allowed_paths: ["/home/user/src", "/tmp"]
  edit:
    allowed_paths: ["/home/user/src"]
  grep: {}
  find: {}
  ls: {}
constraints:
  max_turns: 20
  timeout_secs: 300
"#;
        let config: AgentConfig = serde_yaml::from_str(yaml).unwrap();
        assert!(config.tools.bash.is_some());
        assert!(config.tools.read.is_some());
        assert!(config.tools.write.is_some());
        assert!(config.tools.edit.is_some());
        assert!(config.tools.grep.is_some());
        assert!(config.tools.find.is_some());
        assert!(config.tools.ls.is_some());
    }

    #[test]
    fn parse_minimal_tools_config() {
        let yaml = r#"
name: minimal
description: "no tools"
llm:
  provider: anthropic
  model: claude-sonnet-4-20250514
  max_tokens: 1024
system_prompt: "test"
tools: {}
constraints:
  max_turns: 5
  timeout_secs: 60
"#;
        let config: AgentConfig = serde_yaml::from_str(yaml).unwrap();
        assert!(config.tools.bash.is_none());
        assert!(config.tools.read.is_none());
        assert!(config.tools.write.is_none());
    }

    #[test]
    fn to_policy_bash_only() {
        let yaml = r#"
name: t
description: "t"
llm: { provider: a, model: b, max_tokens: 1 }
system_prompt: "t"
tools:
  bash:
    allowed_binaries: [python, cargo]
constraints: { max_turns: 1, timeout_secs: 1 }
"#;
        let config: AgentConfig = serde_yaml::from_str(yaml).unwrap();
        let policy = config.tools.to_policy();
        assert_eq!(policy.allowed_binaries, vec!["python", "cargo"]);
        assert!(policy.allowed_read_paths.is_empty());
        assert!(policy.allowed_write_paths.is_empty());
    }

    #[test]
    fn to_policy_grep_find_ls_add_binaries() {
        let yaml = r#"
name: t
description: "t"
llm: { provider: a, model: b, max_tokens: 1 }
system_prompt: "t"
tools:
  grep: {}
  find: {}
  ls: {}
constraints: { max_turns: 1, timeout_secs: 1 }
"#;
        let config: AgentConfig = serde_yaml::from_str(yaml).unwrap();
        let policy = config.tools.to_policy();
        assert!(policy.allowed_binaries.contains(&"grep".to_string()));
        assert!(policy.allowed_binaries.contains(&"find".to_string()));
        assert!(policy.allowed_binaries.contains(&"ls".to_string()));
    }

    #[test]
    fn to_policy_read_write_paths() {
        let yaml = r#"
name: t
description: "t"
llm: { provider: a, model: b, max_tokens: 1 }
system_prompt: "t"
tools:
  read:
    allowed_paths: ["/home/user/docs"]
  write:
    allowed_paths: ["/tmp"]
constraints: { max_turns: 1, timeout_secs: 1 }
"#;
        let config: AgentConfig = serde_yaml::from_str(yaml).unwrap();
        let policy = config.tools.to_policy();
        assert_eq!(policy.allowed_read_paths, vec!["/home/user/docs"]);
        assert_eq!(policy.allowed_write_paths, vec!["/tmp"]);
    }

    #[test]
    fn to_policy_edit_implies_read_and_write() {
        let yaml = r#"
name: t
description: "t"
llm: { provider: a, model: b, max_tokens: 1 }
system_prompt: "t"
tools:
  edit:
    allowed_paths: ["/src"]
constraints: { max_turns: 1, timeout_secs: 1 }
"#;
        let config: AgentConfig = serde_yaml::from_str(yaml).unwrap();
        let policy = config.tools.to_policy();
        assert!(policy.allowed_read_paths.contains(&"/src".to_string()));
        assert!(policy.allowed_write_paths.contains(&"/src".to_string()));
    }

    #[test]
    fn to_policy_combined() {
        let yaml = r#"
name: t
description: "t"
llm: { provider: a, model: b, max_tokens: 1 }
system_prompt: "t"
tools:
  bash:
    allowed_binaries: [feishu]
  read:
    allowed_paths: ["/docs"]
  edit:
    allowed_paths: ["/src"]
  grep: {}
constraints: { max_turns: 1, timeout_secs: 1 }
"#;
        let config: AgentConfig = serde_yaml::from_str(yaml).unwrap();
        let policy = config.tools.to_policy();
        // bash binaries + grep binary
        assert!(policy.allowed_binaries.contains(&"feishu".to_string()));
        assert!(policy.allowed_binaries.contains(&"grep".to_string()));
        // read paths: from read + edit
        assert!(policy.allowed_read_paths.contains(&"/docs".to_string()));
        assert!(policy.allowed_read_paths.contains(&"/src".to_string()));
        // write paths: from edit only
        assert!(policy.allowed_write_paths.contains(&"/src".to_string()));
        assert!(!policy.allowed_write_paths.contains(&"/docs".to_string()));
    }

    #[test]
    fn to_policy_empty_tools() {
        let tools = ToolsConfig {
            toolset: None,
            bash: None,
            read: None,
            write: None,
            edit: None,
            grep: None,
            find: None,
            ls: None,
        };
        let policy = tools.to_policy();
        assert!(policy.allowed_binaries.is_empty());
        assert!(policy.allowed_read_paths.is_empty());
        assert!(policy.allowed_write_paths.is_empty());
    }

    #[test]
    fn to_policy_expands_tilde_in_paths() {
        let yaml = r#"
name: t
description: "t"
llm: { provider: a, model: b, max_tokens: 1 }
system_prompt: "t"
tools:
  read:
    allowed_paths: ["~/Documents"]
  write:
    allowed_paths: ["~/output"]
constraints: { max_turns: 1, timeout_secs: 1 }
"#;
        let config: AgentConfig = serde_yaml::from_str(yaml).unwrap();
        let policy = config.tools.to_policy();
        // After expansion, paths should NOT start with "~/"
        for p in &policy.allowed_read_paths {
            assert!(!p.starts_with("~/"), "tilde should be expanded, got: {p}");
        }
        for p in &policy.allowed_write_paths {
            assert!(!p.starts_with("~/"), "tilde should be expanded, got: {p}");
        }
        // Paths should end with the original suffix
        assert!(
            policy.allowed_read_paths[0].ends_with("/Documents")
                || policy.allowed_read_paths[0].ends_with("\\Documents"),
            "expanded path should end with Documents, got: {}",
            policy.allowed_read_paths[0]
        );
    }

    #[test]
    fn to_policy_preserves_absolute_paths() {
        let yaml = r#"
name: t
description: "t"
llm: { provider: a, model: b, max_tokens: 1 }
system_prompt: "t"
tools:
  read:
    allowed_paths: ["/workspace/src"]
constraints: { max_turns: 1, timeout_secs: 1 }
"#;
        let config: AgentConfig = serde_yaml::from_str(yaml).unwrap();
        let policy = config.tools.to_policy();
        assert_eq!(policy.allowed_read_paths[0], "/workspace/src");
    }

    // ========================================================================
    // Phase 5: LlmConfig extensions (base_url, api_key_env)
    // ========================================================================

    #[test]
    fn parse_llm_config_with_base_url_and_api_key_env() {
        let yaml = r#"
name: custom-endpoint
description: "custom LLM endpoint"
llm:
  provider: qwen
  model: qwen-plus
  max_tokens: 4096
  base_url: "https://custom.api.example.com/v1"
  api_key_env: "MY_CUSTOM_KEY"
system_prompt: "test"
tools: {}
constraints: { max_turns: 10, timeout_secs: 120 }
"#;
        let config: AgentConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(
            config.llm.base_url.as_deref(),
            Some("https://custom.api.example.com/v1")
        );
        assert_eq!(config.llm.api_key_env.as_deref(), Some("MY_CUSTOM_KEY"));
    }

    #[test]
    fn parse_llm_config_without_optional_fields() {
        // base_url and api_key_env should default to None when omitted
        let yaml = r#"
name: default
description: "uses defaults"
llm:
  provider: deepseek
  model: deepseek-chat
  max_tokens: 2048
system_prompt: "test"
tools: {}
constraints: { max_turns: 5, timeout_secs: 60 }
"#;
        let config: AgentConfig = serde_yaml::from_str(yaml).unwrap();
        assert!(config.llm.base_url.is_none());
        assert!(config.llm.api_key_env.is_none());
    }

    // ========================================================================
    // Phase 5: SandboxConfig
    // ========================================================================

    #[test]
    fn parse_sandbox_config_full() {
        let yaml = r#"
name: sandbox-agent
description: "with sandbox config"
llm: { provider: qwen, model: qwen-plus, max_tokens: 4096 }
system_prompt: "test"
tools: {}
constraints: { max_turns: 10, timeout_secs: 120 }
sandbox:
  cpus: 2
  memory_mib: 1024
  network: true
"#;
        let config: AgentConfig = serde_yaml::from_str(yaml).unwrap();
        let sb = config.sandbox.unwrap();
        assert_eq!(sb.cpus, 2);
        assert_eq!(sb.memory_mib, 1024);
        assert_eq!(sb.network, NetworkPolicy::Full);
    }

    #[test]
    fn parse_sandbox_config_defaults() {
        let yaml = r#"
name: sandbox-defaults
description: "sandbox with defaults"
llm: { provider: qwen, model: qwen-plus, max_tokens: 4096 }
system_prompt: "test"
tools: {}
constraints: { max_turns: 10, timeout_secs: 120 }
sandbox: {}
"#;
        let config: AgentConfig = serde_yaml::from_str(yaml).unwrap();
        let sb = config.sandbox.unwrap();
        assert_eq!(sb.cpus, 1);
        assert_eq!(sb.memory_mib, 512);
        assert_eq!(sb.network, NetworkPolicy::Airgapped);
    }

    #[test]
    fn parse_no_sandbox_config() {
        // sandbox field omitted entirely — should be None
        let yaml = r#"
name: no-sandbox
description: "no sandbox config"
llm: { provider: qwen, model: qwen-plus, max_tokens: 4096 }
system_prompt: "test"
tools: {}
constraints: { max_turns: 10, timeout_secs: 120 }
"#;
        let config: AgentConfig = serde_yaml::from_str(yaml).unwrap();
        assert!(config.sandbox.is_none());
    }

    // ========================================================================
    // Phase 5: tool_names() helper
    // ========================================================================

    #[test]
    fn tool_names_all_enabled() {
        let yaml = r#"
name: t
description: "t"
llm: { provider: a, model: b, max_tokens: 1 }
system_prompt: "t"
tools:
  bash: { allowed_binaries: [python] }
  read: { allowed_paths: ["/"] }
  write: { allowed_paths: ["/"] }
  edit: { allowed_paths: ["/"] }
  grep: {}
  find: {}
  ls: {}
constraints: { max_turns: 1, timeout_secs: 1 }
"#;
        let config: AgentConfig = serde_yaml::from_str(yaml).unwrap();
        let names = config.tools.tool_names();
        assert_eq!(names.len(), 7);
        assert!(names.contains(&"bash".to_string()));
        assert!(names.contains(&"read".to_string()));
        assert!(names.contains(&"write".to_string()));
        assert!(names.contains(&"edit".to_string()));
        assert!(names.contains(&"grep".to_string()));
        assert!(names.contains(&"find".to_string()));
        assert!(names.contains(&"ls".to_string()));
    }

    #[test]
    fn tool_names_partial() {
        let yaml = r#"
name: t
description: "t"
llm: { provider: a, model: b, max_tokens: 1 }
system_prompt: "t"
tools:
  bash: { allowed_binaries: [cargo] }
  grep: {}
constraints: { max_turns: 1, timeout_secs: 1 }
"#;
        let config: AgentConfig = serde_yaml::from_str(yaml).unwrap();
        let names = config.tools.tool_names();
        assert_eq!(names.len(), 2);
        assert!(names.contains(&"bash".to_string()));
        assert!(names.contains(&"grep".to_string()));
    }

    #[test]
    fn tool_names_empty() {
        let tools = ToolsConfig {
            toolset: None,
            bash: None,
            read: None,
            write: None,
            edit: None,
            grep: None,
            find: None,
            ls: None,
        };
        assert!(tools.tool_names().is_empty());
    }

    // ========================================================================
    // Phase 5: backward compatibility — existing YAML still parses
    // ========================================================================

    #[test]
    fn existing_feishu_yaml_still_parses_with_new_fields() {
        // The original FEISHU_YAML from Phase 0 must still work
        let config: AgentConfig = serde_yaml::from_str(FEISHU_YAML).unwrap();
        assert_eq!(config.name, "feishu-assistant");
        // New optional fields should be None
        assert!(config.llm.base_url.is_none());
        assert!(config.llm.api_key_env.is_none());
        assert!(config.sandbox.is_none());
    }

    // ========================================================================
    // Phase 5: YAML config examples
    // ========================================================================

    #[test]
    fn parse_coding_assistant_yaml() {
        let yaml = r#"
name: coding-assistant
description: "Qwen-based coding assistant"
llm:
  provider: qwen
  model: qwen-plus
  max_tokens: 8192
  base_url: "https://dashscope.aliyuncs.com/compatible-mode/v1"
system_prompt: |
  你是一个代码助手。帮助用户编写、调试和优化代码。
  工作目录是 /workspace，请在该目录下操作。
tools:
  bash:
    allowed_binaries: [python, python3, pip, cargo, rustc, node, npm, git]
  read:
    allowed_paths: ["/workspace"]
  write:
    allowed_paths: ["/workspace"]
  edit:
    allowed_paths: ["/workspace"]
  grep: {}
  find: {}
  ls: {}
constraints:
  max_turns: 30
  timeout_secs: 600
sandbox:
  cpus: 2
  memory_mib: 2048
  network: true
"#;
        let config: AgentConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.name, "coding-assistant");
        assert_eq!(config.llm.provider, "qwen");
        assert_eq!(config.llm.model, "qwen-plus");
        assert_eq!(config.llm.max_tokens, Some(8192));
        assert_eq!(
            config.llm.base_url.as_deref(),
            Some("https://dashscope.aliyuncs.com/compatible-mode/v1")
        );
        assert!(config.llm.api_key_env.is_none()); // uses default DASHSCOPE_API_KEY
        let sb = config.sandbox.unwrap();
        assert_eq!(sb.cpus, 2);
        assert_eq!(sb.memory_mib, 2048);
        assert_eq!(sb.network, NetworkPolicy::Full);
        assert_eq!(config.tools.tool_names().len(), 7);
    }

    #[test]
    fn parse_deepseek_coder_yaml() {
        let yaml = r#"
name: deepseek-coder
description: "DeepSeek V3 coding agent"
llm:
  provider: deepseek
  model: deepseek-chat
  max_tokens: 8192
  api_key_env: "DEEPSEEK_API_KEY"
system_prompt: |
  You are a coding assistant powered by DeepSeek.
tools:
  bash:
    allowed_binaries: [python3, cargo, node, git]
  read:
    allowed_paths: ["/workspace"]
  write:
    allowed_paths: ["/workspace"]
  edit:
    allowed_paths: ["/workspace"]
  grep: {}
  find: {}
  ls: {}
constraints:
  max_turns: 25
  timeout_secs: 300
sandbox:
  cpus: 1
  memory_mib: 1024
  network: true
"#;
        let config: AgentConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.name, "deepseek-coder");
        assert_eq!(config.llm.provider, "deepseek");
        assert_eq!(config.llm.api_key_env.as_deref(), Some("DEEPSEEK_API_KEY"));
        assert!(config.llm.base_url.is_none()); // uses default deepseek endpoint
    }

    // ========================================================================
    // P6: Toolset presets
    // ========================================================================

    #[test]
    fn toolset_coding_enables_all_seven_tools() {
        let yaml = r#"
name: t
description: "t"
llm: { provider: a, model: b, max_tokens: 1 }
system_prompt: "t"
tools:
  toolset: coding
constraints: { max_turns: 1, timeout_secs: 1 }
"#;
        let config: AgentConfig = serde_yaml::from_str(yaml).unwrap();
        let names = config.tools.tool_names();
        assert_eq!(names.len(), 7);
        for tool in &["bash", "read", "write", "edit", "grep", "find", "ls"] {
            assert!(names.contains(&tool.to_string()), "missing tool: {tool}");
        }
    }

    #[test]
    fn toolset_coding_policy_allows_all_binaries() {
        let yaml = r#"
name: t
description: "t"
llm: { provider: a, model: b, max_tokens: 1 }
system_prompt: "t"
tools:
  toolset: coding
constraints: { max_turns: 1, timeout_secs: 1 }
"#;
        let config: AgentConfig = serde_yaml::from_str(yaml).unwrap();
        let policy = config.tools.to_policy();
        assert!(policy.allowed_binaries.contains(&"*".to_string()));
    }

    #[test]
    fn toolset_readonly_has_no_bash_no_write_no_edit() {
        let yaml = r#"
name: t
description: "t"
llm: { provider: a, model: b, max_tokens: 1 }
system_prompt: "t"
tools:
  toolset: readonly
constraints: { max_turns: 1, timeout_secs: 1 }
"#;
        let config: AgentConfig = serde_yaml::from_str(yaml).unwrap();
        let names = config.tools.tool_names();
        assert!(!names.contains(&"bash".to_string()));
        assert!(!names.contains(&"write".to_string()));
        assert!(!names.contains(&"edit".to_string()));
        assert!(names.contains(&"read".to_string()));
        assert!(names.contains(&"grep".to_string()));
        assert!(names.contains(&"find".to_string()));
        assert!(names.contains(&"ls".to_string()));
    }

    #[test]
    fn toolset_minimal_same_as_readonly() {
        let yaml = r#"
name: t
description: "t"
llm: { provider: a, model: b, max_tokens: 1 }
system_prompt: "t"
tools:
  toolset: minimal
constraints: { max_turns: 1, timeout_secs: 1 }
"#;
        let config: AgentConfig = serde_yaml::from_str(yaml).unwrap();
        let names = config.tools.tool_names();
        assert_eq!(names.len(), 4);
        assert!(!names.contains(&"bash".to_string()));
        assert!(names.contains(&"read".to_string()));
    }

    #[test]
    fn toolset_ops_has_bash_read_grep_find_ls_no_write_edit() {
        let yaml = r#"
name: t
description: "t"
llm: { provider: a, model: b, max_tokens: 1 }
system_prompt: "t"
tools:
  toolset: ops
constraints: { max_turns: 1, timeout_secs: 1 }
"#;
        let config: AgentConfig = serde_yaml::from_str(yaml).unwrap();
        let names = config.tools.tool_names();
        assert_eq!(names.len(), 5);
        assert!(names.contains(&"bash".to_string()));
        assert!(names.contains(&"read".to_string()));
        assert!(names.contains(&"grep".to_string()));
        assert!(names.contains(&"find".to_string()));
        assert!(names.contains(&"ls".to_string()));
        assert!(!names.contains(&"write".to_string()));
        assert!(!names.contains(&"edit".to_string()));
    }

    #[test]
    fn toolset_ops_policy_has_restricted_binaries() {
        let yaml = r#"
name: t
description: "t"
llm: { provider: a, model: b, max_tokens: 1 }
system_prompt: "t"
tools:
  toolset: ops
constraints: { max_turns: 1, timeout_secs: 1 }
"#;
        let config: AgentConfig = serde_yaml::from_str(yaml).unwrap();
        let policy = config.tools.to_policy();
        // ops preset should have specific allowed binaries, not wildcard
        assert!(!policy.allowed_binaries.contains(&"*".to_string()));
        // Should include common ops tools
        for bin in &["curl", "docker", "kubectl", "ssh", "systemctl"] {
            assert!(
                policy.allowed_binaries.contains(&bin.to_string()),
                "ops preset missing binary: {bin}"
            );
        }
    }

    #[test]
    fn toolset_web_has_bash_and_read_only() {
        let yaml = r#"
name: t
description: "t"
llm: { provider: a, model: b, max_tokens: 1 }
system_prompt: "t"
tools:
  toolset: web
constraints: { max_turns: 1, timeout_secs: 1 }
"#;
        let config: AgentConfig = serde_yaml::from_str(yaml).unwrap();
        let names = config.tools.tool_names();
        assert!(names.contains(&"bash".to_string()));
        assert!(names.contains(&"read".to_string()));
        assert!(!names.contains(&"write".to_string()));
        assert!(!names.contains(&"edit".to_string()));
    }

    #[test]
    fn toolset_web_policy_has_http_binaries() {
        let yaml = r#"
name: t
description: "t"
llm: { provider: a, model: b, max_tokens: 1 }
system_prompt: "t"
tools:
  toolset: web
constraints: { max_turns: 1, timeout_secs: 1 }
"#;
        let config: AgentConfig = serde_yaml::from_str(yaml).unwrap();
        let policy = config.tools.to_policy();
        assert!(!policy.allowed_binaries.contains(&"*".to_string()));
        for bin in &["curl", "wget", "jq", "python"] {
            assert!(
                policy.allowed_binaries.contains(&bin.to_string()),
                "web preset missing binary: {bin}"
            );
        }
    }

    #[test]
    fn toolset_with_explicit_override_uses_override() {
        // toolset provides defaults, but explicit tool config overrides
        let yaml = r#"
name: t
description: "t"
llm: { provider: a, model: b, max_tokens: 1 }
system_prompt: "t"
tools:
  toolset: coding
  bash:
    allowed_binaries: [python]
constraints: { max_turns: 1, timeout_secs: 1 }
"#;
        let config: AgentConfig = serde_yaml::from_str(yaml).unwrap();
        let policy = config.tools.to_policy();
        // explicit bash config overrides toolset's wildcard
        assert!(policy.allowed_binaries.contains(&"python".to_string()));
        assert!(!policy.allowed_binaries.contains(&"*".to_string()));
    }

    #[test]
    fn toolset_with_partial_override_merges() {
        // toolset provides base tools, explicit config overrides specific ones
        let yaml = r#"
name: t
description: "t"
llm: { provider: a, model: b, max_tokens: 1 }
system_prompt: "t"
tools:
  toolset: coding
  read:
    allowed_paths: ["/restricted"]
constraints: { max_turns: 1, timeout_secs: 1 }
"#;
        let config: AgentConfig = serde_yaml::from_str(yaml).unwrap();
        let names = config.tools.tool_names();
        // All 7 tools still enabled (from toolset)
        assert_eq!(names.len(), 7);
        // But read has restricted paths from explicit config
        let policy = config.tools.to_policy();
        assert_eq!(policy.allowed_read_paths, vec!["/restricted"]);
    }

    #[test]
    fn toolset_unknown_value_returns_error() {
        let yaml = r#"
name: t
description: "t"
llm: { provider: a, model: b, max_tokens: 1 }
system_prompt: "t"
tools:
  toolset: nonexistent
constraints: { max_turns: 1, timeout_secs: 1 }
"#;
        let result = serde_yaml::from_str::<AgentConfig>(yaml);
        assert!(result.is_err());
    }

    #[test]
    fn no_toolset_field_backward_compatible() {
        // Existing configs without toolset field should parse exactly as before
        let yaml = r#"
name: t
description: "t"
llm: { provider: a, model: b, max_tokens: 1 }
system_prompt: "t"
tools:
  bash:
    allowed_binaries: [cargo]
  grep: {}
constraints: { max_turns: 1, timeout_secs: 1 }
"#;
        let config: AgentConfig = serde_yaml::from_str(yaml).unwrap();
        let names = config.tools.tool_names();
        assert_eq!(names.len(), 2);
        assert!(names.contains(&"bash".to_string()));
        assert!(names.contains(&"grep".to_string()));
    }

    #[test]
    fn empty_tools_with_no_toolset_has_zero_tools() {
        let yaml = r#"
name: t
description: "t"
llm: { provider: a, model: b, max_tokens: 1 }
system_prompt: "t"
tools: {}
constraints: { max_turns: 1, timeout_secs: 1 }
"#;
        let config: AgentConfig = serde_yaml::from_str(yaml).unwrap();
        assert!(config.tools.tool_names().is_empty());
    }

    #[test]
    fn toolset_coding_readonly_round_trip_serialize() {
        // Toolset should serialize/deserialize correctly
        let coding = Toolset::Coding;
        let readonly = Toolset::Readonly;
        assert_eq!(serde_yaml::to_string(&coding).unwrap().trim(), "coding");
        assert_eq!(serde_yaml::to_string(&readonly).unwrap().trim(), "readonly");
    }

    #[test]
    fn toolset_expand_coding_all_tools_present() {
        let expanded = Toolset::Coding.expand();
        assert!(expanded.bash.is_some());
        assert!(expanded.read.is_some());
        assert!(expanded.write.is_some());
        assert!(expanded.edit.is_some());
        assert!(expanded.grep.is_some());
        assert!(expanded.find.is_some());
        assert!(expanded.ls.is_some());
        assert_eq!(expanded.bash.unwrap().allowed_binaries, vec!["*"]);
    }

    #[test]
    fn toolset_expand_ops_has_bash_read_grep_find_ls() {
        let expanded = Toolset::Ops.expand();
        assert!(expanded.bash.is_some());
        assert!(expanded.read.is_some());
        assert!(expanded.write.is_none());
        assert!(expanded.edit.is_none());
        assert!(expanded.grep.is_some());
        assert!(expanded.find.is_some());
        assert!(expanded.ls.is_some());
        let binaries = &expanded.bash.unwrap().allowed_binaries;
        assert!(!binaries.contains(&"*".to_string()));
        assert!(binaries.contains(&"curl".to_string()));
        assert!(binaries.contains(&"docker".to_string()));
    }

    #[test]
    fn toolset_expand_web_has_bash_read_only() {
        let expanded = Toolset::Web.expand();
        assert!(expanded.bash.is_some());
        assert!(expanded.read.is_some());
        assert!(expanded.write.is_none());
        assert!(expanded.edit.is_none());
        assert!(expanded.grep.is_none());
        assert!(expanded.find.is_none());
        assert!(expanded.ls.is_none());
        let binaries = &expanded.bash.unwrap().allowed_binaries;
        assert!(binaries.contains(&"curl".to_string()));
        assert!(binaries.contains(&"python".to_string()));
    }

    #[test]
    fn toolset_expand_minimal_read_grep_find_ls_only() {
        let expanded = Toolset::Minimal.expand();
        assert!(expanded.bash.is_none());
        assert!(expanded.read.is_some());
        assert!(expanded.write.is_none());
        assert!(expanded.edit.is_none());
        assert!(expanded.grep.is_some());
        assert!(expanded.find.is_some());
        assert!(expanded.ls.is_some());
    }

    #[test]
    fn toolset_expand_readonly_same_as_minimal() {
        let minimal = Toolset::Minimal.expand();
        let readonly = Toolset::Readonly.expand();
        assert_eq!(minimal.bash.is_some(), readonly.bash.is_some());
        assert_eq!(minimal.read.is_some(), readonly.read.is_some());
        assert_eq!(minimal.write.is_some(), readonly.write.is_some());
        assert_eq!(minimal.edit.is_some(), readonly.edit.is_some());
        assert_eq!(minimal.grep.is_some(), readonly.grep.is_some());
        assert_eq!(minimal.find.is_some(), readonly.find.is_some());
        assert_eq!(minimal.ls.is_some(), readonly.ls.is_some());
    }

    #[test]
    fn toolset_readonly_policy_precise() {
        let yaml = r#"
name: t
description: "t"
llm: { provider: a, model: b, max_tokens: 1 }
system_prompt: "t"
tools:
  toolset: readonly
constraints: { max_turns: 1, timeout_secs: 1 }
"#;
        let config: AgentConfig = serde_yaml::from_str(yaml).unwrap();
        let policy = config.tools.to_policy();
        assert!(policy.allowed_write_paths.is_empty());
        // readonly has no bash, so only grep/find/ls binaries
        assert!(policy.allowed_binaries.contains(&"grep".to_string()));
        assert!(policy.allowed_binaries.contains(&"find".to_string()));
        assert!(policy.allowed_binaries.contains(&"ls".to_string()));
        assert!(!policy.allowed_binaries.contains(&"*".to_string()));
        assert_eq!(policy.allowed_binaries.len(), 3);
    }

    #[test]
    fn toolset_minimal_policy_same_as_readonly() {
        let yaml = r#"
name: t
description: "t"
llm: { provider: a, model: b, max_tokens: 1 }
system_prompt: "t"
tools:
  toolset: minimal
constraints: { max_turns: 1, timeout_secs: 1 }
"#;
        let config: AgentConfig = serde_yaml::from_str(yaml).unwrap();
        let policy = config.tools.to_policy();
        assert!(policy.allowed_write_paths.is_empty());
        assert_eq!(policy.allowed_binaries.len(), 3);
    }

    #[test]
    fn toolset_web_tool_names_precise() {
        let yaml = r#"
name: t
description: "t"
llm: { provider: a, model: b, max_tokens: 1 }
system_prompt: "t"
tools:
  toolset: web
constraints: { max_turns: 1, timeout_secs: 1 }
"#;
        let config: AgentConfig = serde_yaml::from_str(yaml).unwrap();
        let names = config.tools.tool_names();
        assert_eq!(names.len(), 2);
        assert!(names.contains(&"bash".to_string()));
        assert!(names.contains(&"read".to_string()));
        // Explicitly verify these are absent
        assert!(!names.contains(&"write".to_string()));
        assert!(!names.contains(&"edit".to_string()));
        assert!(!names.contains(&"grep".to_string()));
        assert!(!names.contains(&"find".to_string()));
        assert!(!names.contains(&"ls".to_string()));
    }

    // -- Override / merge scenarios --

    #[test]
    fn toolset_readonly_plus_explicit_bash_adds_tool() {
        // "additive" override: readonly base + explicit bash
        let yaml = r#"
name: t
description: "t"
llm: { provider: a, model: b, max_tokens: 1 }
system_prompt: "t"
tools:
  toolset: readonly
  bash:
    allowed_binaries: [python3]
constraints: { max_turns: 1, timeout_secs: 1 }
"#;
        let config: AgentConfig = serde_yaml::from_str(yaml).unwrap();
        let names = config.tools.tool_names();
        // Should have readonly tools + bash
        assert_eq!(names.len(), 5);
        assert!(names.contains(&"bash".to_string()));
        assert!(names.contains(&"read".to_string()));
        let policy = config.tools.to_policy();
        assert!(policy.allowed_binaries.contains(&"python3".to_string()));
    }

    #[test]
    fn toolset_coding_override_write_with_restricted_paths() {
        let yaml = r#"
name: t
description: "t"
llm: { provider: a, model: b, max_tokens: 1 }
system_prompt: "t"
tools:
  toolset: coding
  write:
    allowed_paths: ["/tmp"]
constraints: { max_turns: 1, timeout_secs: 1 }
"#;
        let config: AgentConfig = serde_yaml::from_str(yaml).unwrap();
        let names = config.tools.tool_names();
        assert_eq!(names.len(), 7); // all tools still present
        let policy = config.tools.to_policy();
        assert_eq!(policy.allowed_write_paths, vec!["/tmp"]);
    }

    #[test]
    fn toolset_coding_override_edit_with_restricted_paths() {
        let yaml = r#"
name: t
description: "t"
llm: { provider: a, model: b, max_tokens: 1 }
system_prompt: "t"
tools:
  toolset: coding
  edit:
    allowed_paths: ["/src"]
constraints: { max_turns: 1, timeout_secs: 1 }
"#;
        let config: AgentConfig = serde_yaml::from_str(yaml).unwrap();
        let policy = config.tools.to_policy();
        // edit implies read and write paths
        assert!(policy.allowed_read_paths.contains(&"/src".to_string()));
        assert!(policy.allowed_write_paths.contains(&"/src".to_string()));
    }

    #[test]
    fn toolset_ops_override_bash_narrows_binaries() {
        let yaml = r#"
name: t
description: "t"
llm: { provider: a, model: b, max_tokens: 1 }
system_prompt: "t"
tools:
  toolset: ops
  bash:
    allowed_binaries: [docker, kubectl]
constraints: { max_turns: 1, timeout_secs: 1 }
"#;
        let config: AgentConfig = serde_yaml::from_str(yaml).unwrap();
        let policy = config.tools.to_policy();
        // Only docker + kubectl (from explicit) + grep/find/ls (from preset tools)
        assert!(policy.allowed_binaries.contains(&"docker".to_string()));
        assert!(policy.allowed_binaries.contains(&"kubectl".to_string()));
        assert!(!policy.allowed_binaries.contains(&"curl".to_string()));
    }

    // -- Error / boundary scenarios --

    #[test]
    fn toolset_empty_string_returns_error() {
        let yaml = r#"
name: t
description: "t"
llm: { provider: a, model: b, max_tokens: 1 }
system_prompt: "t"
tools:
  toolset: ""
constraints: { max_turns: 1, timeout_secs: 1 }
"#;
        assert!(serde_yaml::from_str::<AgentConfig>(yaml).is_err());
    }

    #[test]
    fn toolset_uppercase_returns_error() {
        let yaml = r#"
name: t
description: "t"
llm: { provider: a, model: b, max_tokens: 1 }
system_prompt: "t"
tools:
  toolset: CODING
constraints: { max_turns: 1, timeout_secs: 1 }
"#;
        assert!(serde_yaml::from_str::<AgentConfig>(yaml).is_err());
    }

    #[test]
    fn toolset_mixed_case_returns_error() {
        let yaml = r#"
name: t
description: "t"
llm: { provider: a, model: b, max_tokens: 1 }
system_prompt: "t"
tools:
  toolset: Coding
constraints: { max_turns: 1, timeout_secs: 1 }
"#;
        assert!(serde_yaml::from_str::<AgentConfig>(yaml).is_err());
    }

    #[test]
    fn toolset_integer_value_returns_error() {
        let yaml = r#"
name: t
description: "t"
llm: { provider: a, model: b, max_tokens: 1 }
system_prompt: "t"
tools:
  toolset: 42
constraints: { max_turns: 1, timeout_secs: 1 }
"#;
        assert!(serde_yaml::from_str::<AgentConfig>(yaml).is_err());
    }

    #[test]
    fn toolset_list_value_returns_error() {
        let yaml = r#"
name: t
description: "t"
llm: { provider: a, model: b, max_tokens: 1 }
system_prompt: "t"
tools:
  toolset: [coding, ops]
constraints: { max_turns: 1, timeout_secs: 1 }
"#;
        assert!(serde_yaml::from_str::<AgentConfig>(yaml).is_err());
    }

    #[test]
    fn toolset_explicit_null_treated_as_no_toolset() {
        let yaml = r#"
name: t
description: "t"
llm: { provider: a, model: b, max_tokens: 1 }
system_prompt: "t"
tools:
  toolset: ~
  bash:
    allowed_binaries: [cargo]
constraints: { max_turns: 1, timeout_secs: 1 }
"#;
        let config: AgentConfig = serde_yaml::from_str(yaml).unwrap();
        let names = config.tools.tool_names();
        assert_eq!(names.len(), 1);
        assert!(names.contains(&"bash".to_string()));
    }

    #[test]
    fn toolset_serialize_round_trip_all_variants() {
        for (toolset, expected) in [
            (Toolset::Coding, "coding"),
            (Toolset::Ops, "ops"),
            (Toolset::Web, "web"),
            (Toolset::Minimal, "minimal"),
            (Toolset::Readonly, "readonly"),
        ] {
            let serialized = serde_yaml::to_string(&toolset).unwrap();
            assert_eq!(serialized.trim(), expected);
            let deserialized: Toolset = serde_yaml::from_str(expected).unwrap();
            assert_eq!(deserialized, toolset);
        }
    }

    // -- State: "cannot disable preset tool" behavior documentation --

    // ========================================================================
    // P7: Security Hardening — NetworkPolicy
    // ========================================================================

    #[test]
    fn network_policy_bool_false_is_airgapped() {
        let yaml = r#"
name: t
description: "t"
llm: { provider: a, model: b, max_tokens: 1 }
system_prompt: "t"
tools: {}
constraints: { max_turns: 1, timeout_secs: 1 }
sandbox:
  network: false
"#;
        let config: AgentConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.sandbox.unwrap().network, NetworkPolicy::Airgapped);
    }

    #[test]
    fn network_policy_bool_true_is_full() {
        let yaml = r#"
name: t
description: "t"
llm: { provider: a, model: b, max_tokens: 1 }
system_prompt: "t"
tools: {}
constraints: { max_turns: 1, timeout_secs: 1 }
sandbox:
  network: true
"#;
        let config: AgentConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.sandbox.unwrap().network, NetworkPolicy::Full);
    }

    #[test]
    fn network_policy_string_airgapped() {
        let yaml = r#"
name: t
description: "t"
llm: { provider: a, model: b, max_tokens: 1 }
system_prompt: "t"
tools: {}
constraints: { max_turns: 1, timeout_secs: 1 }
sandbox:
  network: airgapped
"#;
        let config: AgentConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.sandbox.unwrap().network, NetworkPolicy::Airgapped);
    }

    #[test]
    fn network_policy_string_whitelist() {
        let yaml = r#"
name: t
description: "t"
llm: { provider: a, model: b, max_tokens: 1 }
system_prompt: "t"
tools: {}
constraints: { max_turns: 1, timeout_secs: 1 }
sandbox:
  network: whitelist
  allowed_hosts:
    - api.openai.com
    - api.anthropic.com
"#;
        let config: AgentConfig = serde_yaml::from_str(yaml).unwrap();
        let sb = config.sandbox.unwrap();
        assert_eq!(sb.network, NetworkPolicy::Whitelist);
        assert_eq!(
            sb.allowed_hosts,
            vec!["api.openai.com", "api.anthropic.com"]
        );
    }

    #[test]
    fn network_policy_string_full() {
        let yaml = r#"
name: t
description: "t"
llm: { provider: a, model: b, max_tokens: 1 }
system_prompt: "t"
tools: {}
constraints: { max_turns: 1, timeout_secs: 1 }
sandbox:
  network: full
"#;
        let config: AgentConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.sandbox.unwrap().network, NetworkPolicy::Full);
    }

    #[test]
    fn network_policy_default_is_airgapped() {
        // When network field is omitted, default should be airgapped
        let yaml = r#"
name: t
description: "t"
llm: { provider: a, model: b, max_tokens: 1 }
system_prompt: "t"
tools: {}
constraints: { max_turns: 1, timeout_secs: 1 }
sandbox:
  cpus: 1
"#;
        let config: AgentConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.sandbox.unwrap().network, NetworkPolicy::Airgapped);
    }

    #[test]
    fn network_policy_invalid_string_error() {
        let yaml = r#"
name: t
description: "t"
llm: { provider: a, model: b, max_tokens: 1 }
system_prompt: "t"
tools: {}
constraints: { max_turns: 1, timeout_secs: 1 }
sandbox:
  network: foobar
"#;
        let result = serde_yaml::from_str::<AgentConfig>(yaml);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("unknown network policy"),
            "error should mention unknown policy, got: {err}"
        );
    }

    #[test]
    fn network_policy_integer_error() {
        let yaml = r#"
name: t
description: "t"
llm: { provider: a, model: b, max_tokens: 1 }
system_prompt: "t"
tools: {}
constraints: { max_turns: 1, timeout_secs: 1 }
sandbox:
  network: 42
"#;
        // Integer is neither bool nor string — should fail
        assert!(serde_yaml::from_str::<AgentConfig>(yaml).is_err());
    }

    #[test]
    fn network_policy_serialize_roundtrip() {
        // Serialize always uses string form, deserialize accepts both
        for (policy, expected_str) in [
            (NetworkPolicy::Airgapped, "airgapped"),
            (NetworkPolicy::Whitelist, "whitelist"),
            (NetworkPolicy::Full, "full"),
        ] {
            let serialized = serde_yaml::to_string(&policy).unwrap();
            assert_eq!(serialized.trim(), expected_str);
            let deserialized: NetworkPolicy = serde_yaml::from_str(&serialized).unwrap();
            assert_eq!(deserialized, policy);
        }
    }

    #[test]
    fn network_policy_default_trait() {
        assert_eq!(NetworkPolicy::default(), NetworkPolicy::Airgapped);
    }

    // ========================================================================
    // P7: Security Hardening — SandboxConfig with allowed_hosts
    // ========================================================================

    #[test]
    fn sandbox_whitelist_with_hosts() {
        let yaml = r#"
name: t
description: "t"
llm: { provider: a, model: b, max_tokens: 1 }
system_prompt: "t"
tools: {}
constraints: { max_turns: 1, timeout_secs: 1 }
sandbox:
  network: whitelist
  allowed_hosts:
    - api.openai.com
    - "*.anthropic.com"
"#;
        let config: AgentConfig = serde_yaml::from_str(yaml).unwrap();
        let sb = config.sandbox.unwrap();
        assert_eq!(sb.network, NetworkPolicy::Whitelist);
        assert_eq!(sb.allowed_hosts.len(), 2);
        assert_eq!(sb.allowed_hosts[0], "api.openai.com");
        assert_eq!(sb.allowed_hosts[1], "*.anthropic.com");
    }

    #[test]
    fn sandbox_airgapped_with_hosts_still_airgapped() {
        // allowed_hosts is accepted but logically ignored for airgapped
        let yaml = r#"
name: t
description: "t"
llm: { provider: a, model: b, max_tokens: 1 }
system_prompt: "t"
tools: {}
constraints: { max_turns: 1, timeout_secs: 1 }
sandbox:
  network: airgapped
  allowed_hosts: [api.example.com]
"#;
        let config: AgentConfig = serde_yaml::from_str(yaml).unwrap();
        let sb = config.sandbox.unwrap();
        assert_eq!(sb.network, NetworkPolicy::Airgapped);
        assert_eq!(sb.allowed_hosts, vec!["api.example.com"]);
    }

    #[test]
    fn sandbox_full_with_hosts_accepted() {
        // allowed_hosts is accepted but logically ignored for full access
        let yaml = r#"
name: t
description: "t"
llm: { provider: a, model: b, max_tokens: 1 }
system_prompt: "t"
tools: {}
constraints: { max_turns: 1, timeout_secs: 1 }
sandbox:
  network: full
  allowed_hosts: [api.example.com]
"#;
        let config: AgentConfig = serde_yaml::from_str(yaml).unwrap();
        let sb = config.sandbox.unwrap();
        assert_eq!(sb.network, NetworkPolicy::Full);
        // Hosts stored but ignored at enforcement level
        assert_eq!(sb.allowed_hosts.len(), 1);
    }

    #[test]
    fn sandbox_default_empty_hosts() {
        let yaml = r#"
name: t
description: "t"
llm: { provider: a, model: b, max_tokens: 1 }
system_prompt: "t"
tools: {}
constraints: { max_turns: 1, timeout_secs: 1 }
sandbox: {}
"#;
        let config: AgentConfig = serde_yaml::from_str(yaml).unwrap();
        assert!(config.sandbox.unwrap().allowed_hosts.is_empty());
    }

    // ========================================================================
    // P7: Security Hardening — SecurityConfig
    // ========================================================================

    #[test]
    fn security_config_all_defaults() {
        let cfg = SecurityConfig::default();
        assert!(cfg.seccomp);
        assert!(cfg.landlock);
        assert_eq!(cfg.max_file_size_mb, 100);
        assert_eq!(cfg.max_open_files, 256);
        assert_eq!(cfg.tmpfs_size_mb, 512);
    }

    #[test]
    fn security_config_explicit_all_fields() {
        let yaml = r#"
name: t
description: "t"
llm: { provider: a, model: b, max_tokens: 1 }
system_prompt: "t"
tools: {}
constraints: { max_turns: 1, timeout_secs: 1 }
sandbox:
  security:
    seccomp: false
    landlock: false
    max_file_size_mb: 50
    max_open_files: 128
    tmpfs_size_mb: 256
"#;
        let config: AgentConfig = serde_yaml::from_str(yaml).unwrap();
        let sec = &config.sandbox.unwrap().security;
        assert!(!sec.seccomp);
        assert!(!sec.landlock);
        assert_eq!(sec.max_file_size_mb, 50);
        assert_eq!(sec.max_open_files, 128);
        assert_eq!(sec.tmpfs_size_mb, 256);
    }

    #[test]
    fn security_config_partial_override() {
        // Only override seccomp, rest should keep defaults
        let yaml = r#"
name: t
description: "t"
llm: { provider: a, model: b, max_tokens: 1 }
system_prompt: "t"
tools: {}
constraints: { max_turns: 1, timeout_secs: 1 }
sandbox:
  security:
    seccomp: false
"#;
        let config: AgentConfig = serde_yaml::from_str(yaml).unwrap();
        let sec = &config.sandbox.unwrap().security;
        assert!(!sec.seccomp);
        assert!(sec.landlock); // default
        assert_eq!(sec.max_file_size_mb, 100); // default
        assert_eq!(sec.max_open_files, 256); // default
        assert_eq!(sec.tmpfs_size_mb, 512); // default
    }

    #[test]
    fn security_config_landlock_disabled() {
        let yaml = r#"
name: t
description: "t"
llm: { provider: a, model: b, max_tokens: 1 }
system_prompt: "t"
tools: {}
constraints: { max_turns: 1, timeout_secs: 1 }
sandbox:
  security:
    landlock: false
"#;
        let config: AgentConfig = serde_yaml::from_str(yaml).unwrap();
        let sec = &config.sandbox.unwrap().security;
        assert!(sec.seccomp); // default: enabled
        assert!(!sec.landlock);
    }

    #[test]
    fn security_config_zero_max_file_size() {
        let yaml = r#"
name: t
description: "t"
llm: { provider: a, model: b, max_tokens: 1 }
system_prompt: "t"
tools: {}
constraints: { max_turns: 1, timeout_secs: 1 }
sandbox:
  security:
    max_file_size_mb: 0
"#;
        let config: AgentConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.sandbox.unwrap().security.max_file_size_mb, 0);
    }

    #[test]
    fn security_config_large_values() {
        let yaml = r#"
name: t
description: "t"
llm: { provider: a, model: b, max_tokens: 1 }
system_prompt: "t"
tools: {}
constraints: { max_turns: 1, timeout_secs: 1 }
sandbox:
  security:
    max_file_size_mb: 10240
    max_open_files: 65536
    tmpfs_size_mb: 8192
"#;
        let config: AgentConfig = serde_yaml::from_str(yaml).unwrap();
        let sec = &config.sandbox.unwrap().security;
        assert_eq!(sec.max_file_size_mb, 10240);
        assert_eq!(sec.max_open_files, 65536);
        assert_eq!(sec.tmpfs_size_mb, 8192);
    }

    #[test]
    fn security_config_omitted_uses_defaults() {
        // No security section → SecurityConfig::default()
        let yaml = r#"
name: t
description: "t"
llm: { provider: a, model: b, max_tokens: 1 }
system_prompt: "t"
tools: {}
constraints: { max_turns: 1, timeout_secs: 1 }
sandbox:
  cpus: 2
"#;
        let config: AgentConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.sandbox.unwrap().security, SecurityConfig::default());
    }

    #[test]
    fn security_config_empty_section_uses_defaults() {
        // Explicit empty security section → all defaults
        let yaml = r#"
name: t
description: "t"
llm: { provider: a, model: b, max_tokens: 1 }
system_prompt: "t"
tools: {}
constraints: { max_turns: 1, timeout_secs: 1 }
sandbox:
  security: {}
"#;
        let config: AgentConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.sandbox.unwrap().security, SecurityConfig::default());
    }

    #[test]
    fn security_config_serialize_roundtrip() {
        let cfg = SecurityConfig {
            seccomp: false,
            landlock: true,
            max_file_size_mb: 200,
            max_open_files: 512,
            tmpfs_size_mb: 1024,
            max_processes: 128,
        };
        let yaml = serde_yaml::to_string(&cfg).unwrap();
        let parsed: SecurityConfig = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(parsed, cfg);
    }

    // ========================================================================
    // P7: Security Hardening — Extended SandboxConfig
    // ========================================================================

    #[test]
    fn sandbox_exec_timeout_default() {
        let yaml = r#"
name: t
description: "t"
llm: { provider: a, model: b, max_tokens: 1 }
system_prompt: "t"
tools: {}
constraints: { max_turns: 1, timeout_secs: 1 }
sandbox: {}
"#;
        let config: AgentConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.sandbox.unwrap().exec_timeout_secs, 30);
    }

    #[test]
    fn sandbox_exec_timeout_custom() {
        let yaml = r#"
name: t
description: "t"
llm: { provider: a, model: b, max_tokens: 1 }
system_prompt: "t"
tools: {}
constraints: { max_turns: 1, timeout_secs: 1 }
sandbox:
  exec_timeout_secs: 120
"#;
        let config: AgentConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.sandbox.unwrap().exec_timeout_secs, 120);
    }

    #[test]
    fn sandbox_backward_compat_bool_network_true() {
        // Old-style YAML with network: true should still parse correctly
        let yaml = r#"
name: t
description: "t"
llm: { provider: a, model: b, max_tokens: 1 }
system_prompt: "t"
tools: {}
constraints: { max_turns: 1, timeout_secs: 1 }
sandbox:
  cpus: 2
  memory_mib: 1024
  network: true
"#;
        let config: AgentConfig = serde_yaml::from_str(yaml).unwrap();
        let sb = config.sandbox.unwrap();
        assert_eq!(sb.cpus, 2);
        assert_eq!(sb.memory_mib, 1024);
        assert_eq!(sb.network, NetworkPolicy::Full);
        // New fields should have defaults
        assert!(sb.allowed_hosts.is_empty());
        assert_eq!(sb.security, SecurityConfig::default());
        assert_eq!(sb.exec_timeout_secs, 30);
    }

    #[test]
    fn sandbox_backward_compat_bool_network_false() {
        let yaml = r#"
name: t
description: "t"
llm: { provider: a, model: b, max_tokens: 1 }
system_prompt: "t"
tools: {}
constraints: { max_turns: 1, timeout_secs: 1 }
sandbox:
  cpus: 1
  memory_mib: 512
  network: false
"#;
        let config: AgentConfig = serde_yaml::from_str(yaml).unwrap();
        let sb = config.sandbox.unwrap();
        assert_eq!(sb.network, NetworkPolicy::Airgapped);
    }

    #[test]
    fn sandbox_backward_compat_empty() {
        // sandbox: {} should work with all defaults
        let yaml = r#"
name: t
description: "t"
llm: { provider: a, model: b, max_tokens: 1 }
system_prompt: "t"
tools: {}
constraints: { max_turns: 1, timeout_secs: 1 }
sandbox: {}
"#;
        let config: AgentConfig = serde_yaml::from_str(yaml).unwrap();
        let sb = config.sandbox.unwrap();
        assert_eq!(sb.cpus, 1);
        assert_eq!(sb.memory_mib, 512);
        assert_eq!(sb.network, NetworkPolicy::Airgapped);
        assert!(sb.allowed_hosts.is_empty());
        assert_eq!(sb.security, SecurityConfig::default());
        assert_eq!(sb.exec_timeout_secs, 30);
    }

    #[test]
    fn sandbox_full_yaml_every_field() {
        let yaml = r#"
name: secure-agent
description: "fully secured agent"
llm: { provider: anthropic, model: claude-sonnet-4-20250514, max_tokens: 4096 }
system_prompt: "secure test"
tools:
  toolset: coding
constraints: { max_turns: 30, timeout_secs: 600 }
sandbox:
  cpus: 4
  memory_mib: 4096
  network: whitelist
  allowed_hosts:
    - api.anthropic.com
    - api.openai.com
    - "*.githubusercontent.com"
  security:
    seccomp: true
    landlock: true
    max_file_size_mb: 200
    max_open_files: 512
    tmpfs_size_mb: 1024
  exec_timeout_secs: 60
"#;
        let config: AgentConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.name, "secure-agent");
        let sb = config.sandbox.unwrap();
        assert_eq!(sb.cpus, 4);
        assert_eq!(sb.memory_mib, 4096);
        assert_eq!(sb.network, NetworkPolicy::Whitelist);
        assert_eq!(sb.allowed_hosts.len(), 3);
        assert_eq!(sb.allowed_hosts[0], "api.anthropic.com");
        assert_eq!(sb.allowed_hosts[2], "*.githubusercontent.com");
        assert!(sb.security.seccomp);
        assert!(sb.security.landlock);
        assert_eq!(sb.security.max_file_size_mb, 200);
        assert_eq!(sb.security.max_open_files, 512);
        assert_eq!(sb.security.tmpfs_size_mb, 1024);
        assert_eq!(sb.exec_timeout_secs, 60);
    }

    #[test]
    fn agent_config_existing_yaml_still_parses_with_security_fields() {
        // The original FEISHU_YAML has no sandbox/security — must still parse
        let config: AgentConfig = serde_yaml::from_str(FEISHU_YAML).unwrap();
        assert!(config.sandbox.is_none());
    }

    #[test]
    fn network_policy_whitelist_empty_hosts_accepted() {
        // Whitelist with no hosts is valid (but useless — no traffic allowed)
        let yaml = r#"
name: t
description: "t"
llm: { provider: a, model: b, max_tokens: 1 }
system_prompt: "t"
tools: {}
constraints: { max_turns: 1, timeout_secs: 1 }
sandbox:
  network: whitelist
"#;
        let config: AgentConfig = serde_yaml::from_str(yaml).unwrap();
        let sb = config.sandbox.unwrap();
        assert_eq!(sb.network, NetworkPolicy::Whitelist);
        assert!(sb.allowed_hosts.is_empty());
    }

    #[test]
    fn sandbox_exec_timeout_zero_allowed() {
        // Zero timeout = no per-command timeout enforcement
        let yaml = r#"
name: t
description: "t"
llm: { provider: a, model: b, max_tokens: 1 }
system_prompt: "t"
tools: {}
constraints: { max_turns: 1, timeout_secs: 1 }
sandbox:
  exec_timeout_secs: 0
"#;
        let config: AgentConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.sandbox.unwrap().exec_timeout_secs, 0);
    }

    // ========================================================================
    // P7: Security Hardening — Boundary / Error path supplements
    // ========================================================================

    #[test]
    fn network_policy_case_sensitive_rejects_capitalized() {
        for value in &[
            "Full",
            "FULL",
            "Airgapped",
            "AIRGAPPED",
            "Whitelist",
            "WHITELIST",
        ] {
            let yaml = format!(
                r#"
name: t
description: "t"
llm: {{ provider: a, model: b, max_tokens: 1 }}
system_prompt: "t"
tools: {{}}
constraints: {{ max_turns: 1, timeout_secs: 1 }}
sandbox:
  network: {value}
"#
            );
            let result = serde_yaml::from_str::<AgentConfig>(&yaml);
            assert!(
                result.is_err(),
                "network: {value} should be rejected (case sensitive)"
            );
        }
    }

    #[test]
    fn network_policy_empty_string_error() {
        let yaml = r#"
name: t
description: "t"
llm: { provider: a, model: b, max_tokens: 1 }
system_prompt: "t"
tools: {}
constraints: { max_turns: 1, timeout_secs: 1 }
sandbox:
  network: ""
"#;
        let result = serde_yaml::from_str::<AgentConfig>(yaml);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("unknown network policy"),
            "empty string should produce 'unknown network policy' error, got: {err}"
        );
    }

    #[test]
    fn network_policy_yaml_null_is_error() {
        // network: ~ (YAML null) is NOT the same as omitting the field.
        // serde(default) only activates when the key is absent, not when
        // the value is null. With our custom Deserialize impl, null doesn't
        // match bool or string, so it correctly errors.
        let yaml = r#"
name: t
description: "t"
llm: { provider: a, model: b, max_tokens: 1 }
system_prompt: "t"
tools: {}
constraints: { max_turns: 1, timeout_secs: 1 }
sandbox:
  network: ~
"#;
        assert!(serde_yaml::from_str::<AgentConfig>(yaml).is_err());
    }

    #[test]
    fn security_config_string_where_u32_expected_error() {
        let yaml = r#"
name: t
description: "t"
llm: { provider: a, model: b, max_tokens: 1 }
system_prompt: "t"
tools: {}
constraints: { max_turns: 1, timeout_secs: 1 }
sandbox:
  security:
    max_file_size_mb: "abc"
"#;
        assert!(serde_yaml::from_str::<AgentConfig>(yaml).is_err());
    }

    #[test]
    fn security_config_negative_value_error() {
        let yaml = r#"
name: t
description: "t"
llm: { provider: a, model: b, max_tokens: 1 }
system_prompt: "t"
tools: {}
constraints: { max_turns: 1, timeout_secs: 1 }
sandbox:
  security:
    max_file_size_mb: -1
"#;
        assert!(serde_yaml::from_str::<AgentConfig>(yaml).is_err());
    }

    #[test]
    fn security_config_u32_overflow_error() {
        let yaml = r#"
name: t
description: "t"
llm: { provider: a, model: b, max_tokens: 1 }
system_prompt: "t"
tools: {}
constraints: { max_turns: 1, timeout_secs: 1 }
sandbox:
  security:
    max_open_files: 4294967296
"#;
        assert!(serde_yaml::from_str::<AgentConfig>(yaml).is_err());
    }

    #[test]
    fn security_config_int_where_bool_expected_error() {
        let yaml = r#"
name: t
description: "t"
llm: { provider: a, model: b, max_tokens: 1 }
system_prompt: "t"
tools: {}
constraints: { max_turns: 1, timeout_secs: 1 }
sandbox:
  security:
    seccomp: 42
"#;
        // seccomp expects bool, not integer
        assert!(serde_yaml::from_str::<AgentConfig>(yaml).is_err());
    }

    #[test]
    fn sandbox_allowed_hosts_special_characters_accepted() {
        // Serde layer accepts any string; validation is at enforcement level
        let yaml = r#"
name: t
description: "t"
llm: { provider: a, model: b, max_tokens: 1 }
system_prompt: "t"
tools: {}
constraints: { max_turns: 1, timeout_secs: 1 }
sandbox:
  network: whitelist
  allowed_hosts:
    - ""
    - "host with spaces"
    - "../../etc/passwd"
    - "api.例え.jp"
"#;
        let config: AgentConfig = serde_yaml::from_str(yaml).unwrap();
        let sb = config.sandbox.unwrap();
        assert_eq!(sb.allowed_hosts.len(), 4);
        assert_eq!(sb.allowed_hosts[0], "");
        assert_eq!(sb.allowed_hosts[3], "api.例え.jp");
    }

    #[test]
    fn sandbox_allowed_hosts_integer_coerced_to_string() {
        // serde_yaml coerces YAML scalars (including integers) to String
        // in a Vec<String>. This documents that behavior — host validation
        // is at enforcement level, not serde level.
        let yaml = r#"
name: t
description: "t"
llm: { provider: a, model: b, max_tokens: 1 }
system_prompt: "t"
tools: {}
constraints: { max_turns: 1, timeout_secs: 1 }
sandbox:
  network: whitelist
  allowed_hosts:
    - 42
"#;
        let config: AgentConfig = serde_yaml::from_str(yaml).unwrap();
        let sb = config.sandbox.unwrap();
        assert_eq!(sb.allowed_hosts, vec!["42"]);
    }

    #[test]
    fn sandbox_config_full_serialize_roundtrip() {
        // Construct a SandboxConfig with non-default values in every field,
        // serialize to YAML, deserialize back, and verify all fields match.
        let original = SandboxConfig {
            mode: SandboxMode::Microvm,
            rootfs: RootfsTier::Minimal,
            cpus: 4,
            memory_mib: 2048,
            network: NetworkPolicy::Whitelist,
            allowed_hosts: vec!["api.example.com".into(), "*.internal.net".into()],
            security: SecurityConfig {
                seccomp: false,
                landlock: true,
                max_file_size_mb: 200,
                max_open_files: 512,
                tmpfs_size_mb: 1024,
                max_processes: 128,
            },
            exec_timeout_secs: 60,
            workspace_host: None,
        };
        let yaml = serde_yaml::to_string(&original).unwrap();
        let parsed: SandboxConfig = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(parsed.cpus, 4);
        assert_eq!(parsed.memory_mib, 2048);
        assert_eq!(parsed.network, NetworkPolicy::Whitelist);
        assert_eq!(
            parsed.allowed_hosts,
            vec!["api.example.com", "*.internal.net"]
        );
        assert_eq!(parsed.security.seccomp, false);
        assert_eq!(parsed.security.landlock, true);
        assert_eq!(parsed.security.max_file_size_mb, 200);
        assert_eq!(parsed.security.max_open_files, 512);
        assert_eq!(parsed.security.tmpfs_size_mb, 1024);
        assert_eq!(parsed.security.max_processes, 128);
        assert_eq!(parsed.exec_timeout_secs, 60);
    }

    #[test]
    fn sandbox_exec_timeout_u32_max() {
        let yaml = r#"
name: t
description: "t"
llm: { provider: a, model: b, max_tokens: 1 }
system_prompt: "t"
tools: {}
constraints: { max_turns: 1, timeout_secs: 1 }
sandbox:
  exec_timeout_secs: 4294967295
"#;
        let config: AgentConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.sandbox.unwrap().exec_timeout_secs, u32::MAX);
    }

    #[test]
    fn security_config_max_file_size_u32_max() {
        let yaml = r#"
name: t
description: "t"
llm: { provider: a, model: b, max_tokens: 1 }
system_prompt: "t"
tools: {}
constraints: { max_turns: 1, timeout_secs: 1 }
sandbox:
  security:
    max_file_size_mb: 4294967295
"#;
        let config: AgentConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.sandbox.unwrap().security.max_file_size_mb, u32::MAX);
    }

    // -- State: "cannot disable preset tool" behavior documentation --

    #[test]
    fn toolset_preset_tool_cannot_be_disabled_via_yaml_null() {
        // This documents the design: when a toolset is set, you cannot
        // disable a tool that the preset enables by setting it to null/~.
        // serde_yaml deserializes `write: ~` as None, and `None.or(preset)`
        // falls back to the preset. This is intentional — use a more
        // restrictive toolset instead.
        let yaml = r#"
name: t
description: "t"
llm: { provider: a, model: b, max_tokens: 1 }
system_prompt: "t"
tools:
  toolset: coding
  write: ~
constraints: { max_turns: 1, timeout_secs: 1 }
"#;
        let config: AgentConfig = serde_yaml::from_str(yaml).unwrap();
        let names = config.tools.tool_names();
        // write is still present because toolset preset fills it in
        assert!(names.contains(&"write".to_string()));
        assert_eq!(names.len(), 7);
    }

    // ========================================================================
    // Sprint 2 — M2: workspace_host in SandboxConfig
    // ========================================================================

    const MINIMAL_YAML: &str = r#"
name: t
description: "t"
llm: { provider: a, model: b, max_tokens: 1 }
system_prompt: "t"
tools: {}
constraints: { max_turns: 1, timeout_secs: 1 }
"#;

    #[test]
    fn sandbox_config_workspace_host_absent_by_default() {
        let yaml = format!("{}\nsandbox:\n  cpus: 1\n  memory_mib: 512\n", MINIMAL_YAML);
        let config: AgentConfig = serde_yaml::from_str(&yaml).unwrap();
        let sb = config.sandbox.unwrap();
        assert!(
            sb.workspace_host.is_none(),
            "workspace_host should be None when not set"
        );
    }

    #[test]
    fn sandbox_config_workspace_host_parses_absolute_path() {
        let yaml = format!(
            "{}\nsandbox:\n  workspace_host: /var/lib/sage/agents/feishu\n",
            MINIMAL_YAML
        );
        let config: AgentConfig = serde_yaml::from_str(&yaml).unwrap();
        let sb = config.sandbox.unwrap();
        let path = sb.workspace_host.expect("workspace_host should be Some");
        assert_eq!(path.to_string_lossy(), "/var/lib/sage/agents/feishu");
    }

    #[test]
    fn sandbox_config_workspace_host_tilde_is_expanded() {
        let yaml = format!(
            "{}\nsandbox:\n  workspace_host: ~/workspace/feishu\n",
            MINIMAL_YAML
        );
        let config: AgentConfig = serde_yaml::from_str(&yaml).unwrap();
        let sb = config.sandbox.unwrap();
        let path = sb.workspace_host.expect("workspace_host should be Some");
        let s = path.to_string_lossy();
        assert!(!s.starts_with('~'), "tilde must be expanded, got: {s}");
        assert!(
            s.ends_with("workspace/feishu"),
            "path should end with workspace/feishu, got: {s}"
        );
    }

    #[test]
    fn sandbox_config_workspace_host_dotted_path_expanded() {
        // ~/.sage/agents/myagent — dotfile dir under home
        let yaml = format!(
            "{}\nsandbox:\n  workspace_host: ~/.sage/agents/myagent\n",
            MINIMAL_YAML
        );
        let config: AgentConfig = serde_yaml::from_str(&yaml).unwrap();
        let sb = config.sandbox.unwrap();
        let path = sb.workspace_host.unwrap();
        let s = path.to_string_lossy();
        assert!(!s.starts_with('~'), "tilde must be expanded, got: {s}");
        assert!(s.ends_with(".sage/agents/myagent"), "got: {s}");
    }

    #[test]
    fn existing_sandbox_config_without_workspace_host_still_parses() {
        // Backward compat: no workspace_host field → defaults to None
        let yaml = r#"
name: coding-assistant
description: "t"
llm: { provider: a, model: b, max_tokens: 1 }
system_prompt: "t"
tools: {}
constraints: { max_turns: 1, timeout_secs: 1 }
sandbox:
  cpus: 2
  memory_mib: 2048
"#;
        let config: AgentConfig = serde_yaml::from_str(yaml).unwrap();
        let sb = config.sandbox.unwrap();
        assert!(
            sb.workspace_host.is_none(),
            "workspace_host defaults to None"
        );
    }

    #[test]
    fn sandbox_config_workspace_host_no_sandbox_section_still_parses() {
        // Config with no sandbox section at all should parse fine
        let config: AgentConfig = serde_yaml::from_str(FEISHU_YAML).unwrap();
        assert!(config.sandbox.is_none());
    }

    // ========================================================================
    // Sprint 2 — M3: MemoryConfig + MemoryInjectMode + SessionType
    // ========================================================================

    #[test]
    fn agent_config_without_memory_field_defaults_to_none() {
        let config: AgentConfig = serde_yaml::from_str(FEISHU_YAML).unwrap();
        assert!(
            config.memory.is_none(),
            "memory should be None when not specified"
        );
    }

    #[test]
    fn agent_config_minimal_without_memory_parses() {
        let config: AgentConfig = serde_yaml::from_str(MINIMAL_YAML).unwrap();
        assert!(config.memory.is_none());
    }

    #[test]
    fn memory_inject_mode_prepend_system_parses() {
        let yaml = format!("{}\nmemory:\n  inject_as: prepend_system\n", MINIMAL_YAML);
        let config: AgentConfig = serde_yaml::from_str(&yaml).unwrap();
        let mem = config.memory.expect("memory should be Some");
        assert!(
            matches!(mem.inject_as, MemoryInjectMode::PrependSystem),
            "expected PrependSystem"
        );
    }

    #[test]
    fn memory_inject_mode_initial_message_parses() {
        let yaml = format!("{}\nmemory:\n  inject_as: initial_message\n", MINIMAL_YAML);
        let config: AgentConfig = serde_yaml::from_str(&yaml).unwrap();
        let mem = config.memory.unwrap();
        assert!(matches!(mem.inject_as, MemoryInjectMode::InitialMessage));
    }

    #[test]
    fn memory_config_default_auto_load_contains_agent_md() {
        let yaml = format!("{}\nmemory:\n  inject_as: prepend_system\n", MINIMAL_YAML);
        let config: AgentConfig = serde_yaml::from_str(&yaml).unwrap();
        let mem = config.memory.unwrap();
        assert!(
            mem.auto_load.contains(&"AGENT.md".to_string()),
            "default auto_load must include AGENT.md, got: {:?}",
            mem.auto_load
        );
    }

    #[test]
    fn memory_config_default_auto_load_contains_memory_md() {
        let yaml = format!("{}\nmemory:\n  inject_as: prepend_system\n", MINIMAL_YAML);
        let config: AgentConfig = serde_yaml::from_str(&yaml).unwrap();
        let mem = config.memory.unwrap();
        assert!(
            mem.auto_load.contains(&"memory/MEMORY.md".to_string()),
            "default auto_load must include memory/MEMORY.md, got: {:?}",
            mem.auto_load
        );
    }

    #[test]
    fn memory_config_custom_auto_load_overrides_default() {
        let yaml = format!(
            "{}\nmemory:\n  auto_load: [\"CUSTOM.md\", \"notes/NOTES.md\"]\n  inject_as: prepend_system\n",
            MINIMAL_YAML
        );
        let config: AgentConfig = serde_yaml::from_str(&yaml).unwrap();
        let mem = config.memory.unwrap();
        assert_eq!(mem.auto_load.len(), 2, "custom list should have 2 entries");
        assert!(mem.auto_load.contains(&"CUSTOM.md".to_string()));
        assert!(mem.auto_load.contains(&"notes/NOTES.md".to_string()));
        // Default entries must NOT be present when explicitly overridden
        assert!(
            !mem.auto_load.contains(&"AGENT.md".to_string()),
            "AGENT.md should not appear when auto_load is explicitly set"
        );
    }

    #[test]
    fn session_type_user_driven_parses() {
        let yaml = format!(
            "{}\nmemory:\n  inject_as: prepend_system\n  session_type: user_driven\n",
            MINIMAL_YAML
        );
        let config: AgentConfig = serde_yaml::from_str(&yaml).unwrap();
        let st = config
            .memory
            .unwrap()
            .session_type
            .expect("session_type should be Some");
        assert!(matches!(st, SessionType::UserDriven));
    }

    #[test]
    fn session_type_wiki_maintenance_parses() {
        let yaml = format!(
            "{}\nmemory:\n  inject_as: prepend_system\n  session_type: wiki_maintenance\n",
            MINIMAL_YAML
        );
        let config: AgentConfig = serde_yaml::from_str(&yaml).unwrap();
        assert!(matches!(
            config.memory.unwrap().session_type.unwrap(),
            SessionType::WikiMaintenance
        ));
    }

    #[test]
    fn session_type_skill_evaluation_parses() {
        let yaml = format!(
            "{}\nmemory:\n  inject_as: prepend_system\n  session_type: skill_evaluation\n",
            MINIMAL_YAML
        );
        let config: AgentConfig = serde_yaml::from_str(&yaml).unwrap();
        assert!(matches!(
            config.memory.unwrap().session_type.unwrap(),
            SessionType::SkillEvaluation
        ));
    }

    #[test]
    fn session_type_harness_run_parses() {
        let yaml = format!(
            "{}\nmemory:\n  inject_as: prepend_system\n  session_type: harness_run\n",
            MINIMAL_YAML
        );
        let config: AgentConfig = serde_yaml::from_str(&yaml).unwrap();
        assert!(matches!(
            config.memory.unwrap().session_type.unwrap(),
            SessionType::HarnessRun
        ));
    }

    #[test]
    fn session_type_absent_defaults_to_none() {
        let yaml = format!("{}\nmemory:\n  inject_as: prepend_system\n", MINIMAL_YAML);
        let config: AgentConfig = serde_yaml::from_str(&yaml).unwrap();
        assert!(config.memory.unwrap().session_type.is_none());
    }

    // ========================================================================
    // Sprint 7 — WikiConfig (wiki self-maintenance)
    // ========================================================================

    #[test]
    fn wiki_config_default_values() {
        let w = WikiConfig::default();
        assert_eq!(w.trigger_sessions, 3);
        assert_eq!(w.cooldown_secs, 1800);
        assert!(!w.enabled, "enabled must default to false (opt-in)");
    }

    #[test]
    fn wiki_config_yaml_empty_map_applies_all_defaults() {
        // `wiki: {}` — every field takes its default.
        let yaml = format!("{}\nwiki: {{}}\n", MINIMAL_YAML);
        let config: AgentConfig = serde_yaml::from_str(&yaml).unwrap();
        let w = config.wiki.expect("wiki should be Some for `wiki: {}`");
        assert_eq!(w.trigger_sessions, 3);
        assert_eq!(w.cooldown_secs, 1800);
        assert!(!w.enabled);
    }

    #[test]
    fn wiki_config_yaml_full_fields_parse() {
        let yaml = format!(
            "{}\nwiki:\n  trigger_sessions: 5\n  cooldown_secs: 600\n  enabled: true\n",
            MINIMAL_YAML
        );
        let config: AgentConfig = serde_yaml::from_str(&yaml).unwrap();
        let w = config.wiki.unwrap();
        assert_eq!(w.trigger_sessions, 5);
        assert_eq!(w.cooldown_secs, 600);
        assert!(w.enabled);
    }

    #[test]
    fn wiki_config_absent_in_agent_config_is_none() {
        // When the `wiki:` key is entirely missing, the field must be `None`.
        let config: AgentConfig = serde_yaml::from_str(MINIMAL_YAML).unwrap();
        assert!(config.wiki.is_none(), "wiki should be None when absent");
    }

    #[test]
    fn wiki_config_yaml_roundtrip() {
        let cfg = WikiConfig {
            trigger_sessions: 7,
            cooldown_secs: 120,
            enabled: true,
        };
        let yaml = serde_yaml::to_string(&cfg).unwrap();
        let back: WikiConfig = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(back.trigger_sessions, 7);
        assert_eq!(back.cooldown_secs, 120);
        assert!(back.enabled);
    }

    #[test]
    fn wiki_config_partial_fields_fill_defaults() {
        // Only `enabled` is specified — the other two fall back to defaults.
        let yaml = format!("{}\nwiki:\n  enabled: true\n", MINIMAL_YAML);
        let config: AgentConfig = serde_yaml::from_str(&yaml).unwrap();
        let w = config.wiki.unwrap();
        assert!(w.enabled);
        assert_eq!(w.trigger_sessions, 3);
        assert_eq!(w.cooldown_secs, 1800);
    }

    // ── M2: error paths ──────────────────────────────────────────────────────

    #[test]
    fn sandbox_config_workspace_host_integer_coerced_to_string() {
        // serde_yaml 0.9 coerces YAML scalars (integers, booleans) to String when the
        // target type is String. workspace_host: 123 therefore parses as Some("123").
        // This documents the actual behaviour — callers should validate the resulting
        // path before use.
        let yaml = format!("{}\nsandbox:\n  workspace_host: 123\n", MINIMAL_YAML);
        let config: AgentConfig = serde_yaml::from_str(&yaml).unwrap();
        let sb = config.sandbox.unwrap();
        let path = sb
            .workspace_host
            .expect("integer is coerced to string Some(\"123\")");
        assert_eq!(path.to_string_lossy(), "123");
    }

    #[test]
    fn sandbox_config_workspace_host_list_type_returns_error() {
        let yaml = format!(
            "{}\nsandbox:\n  workspace_host:\n  - /path/a\n  - /path/b\n",
            MINIMAL_YAML
        );
        let result = serde_yaml::from_str::<AgentConfig>(&yaml);
        assert!(
            result.is_err(),
            "list-type workspace_host must return a parse error"
        );
    }

    // ── M2: boundary ─────────────────────────────────────────────────────────

    #[test]
    fn sandbox_config_workspace_host_empty_string_parses_to_empty_path() {
        // Empty string is accepted by the parser; validation is the caller's job
        let yaml = format!("{}\nsandbox:\n  workspace_host: \"\"\n", MINIMAL_YAML);
        let config: AgentConfig = serde_yaml::from_str(&yaml).unwrap();
        let sb = config.sandbox.unwrap();
        let path = sb
            .workspace_host
            .expect("empty string should parse as Some");
        assert_eq!(path.to_string_lossy(), "");
    }

    #[test]
    fn sandbox_config_workspace_host_relative_path_accepted_unchanged() {
        // Relative path (no tilde, no leading /) is passed through as-is
        let yaml = format!(
            "{}\nsandbox:\n  workspace_host: relative/path/here\n",
            MINIMAL_YAML
        );
        let config: AgentConfig = serde_yaml::from_str(&yaml).unwrap();
        let sb = config.sandbox.unwrap();
        let path = sb.workspace_host.expect("relative path should parse");
        assert_eq!(path.to_string_lossy(), "relative/path/here");
    }

    #[test]
    fn sandbox_config_workspace_host_path_with_spaces() {
        let yaml = format!(
            "{}\nsandbox:\n  workspace_host: \"/path with spaces/agent\"\n",
            MINIMAL_YAML
        );
        let config: AgentConfig = serde_yaml::from_str(&yaml).unwrap();
        let sb = config.sandbox.unwrap();
        let path = sb.workspace_host.unwrap();
        assert_eq!(path.to_string_lossy(), "/path with spaces/agent");
    }

    #[test]
    fn expand_tilde_no_tilde_prefix_unchanged() {
        // Strings without ~ prefix are never modified
        assert_eq!(expand_tilde("/absolute/path"), "/absolute/path");
        assert_eq!(expand_tilde("relative/path"), "relative/path");
        assert_eq!(expand_tilde(""), "");
    }

    #[test]
    fn expand_tilde_does_not_expand_internal_tilde() {
        // Only a leading ~ is expanded — ~ elsewhere in the path is literal
        let result = expand_tilde("/some/~/path");
        assert_eq!(result, "/some/~/path");
    }

    // ── M3: error paths ──────────────────────────────────────────────────────

    #[test]
    fn memory_inject_mode_invalid_value_returns_error() {
        let yaml = format!("{}\nmemory:\n  inject_as: bad_mode\n", MINIMAL_YAML);
        let result = serde_yaml::from_str::<AgentConfig>(&yaml);
        assert!(
            result.is_err(),
            "invalid inject_as value must return a YAML parse error"
        );
    }

    #[test]
    fn memory_inject_mode_missing_returns_error() {
        // inject_as has no default — omitting it when memory section exists is an error
        let yaml = format!("{}\nmemory:\n  auto_load: [\"AGENT.md\"]\n", MINIMAL_YAML);
        let result = serde_yaml::from_str::<AgentConfig>(&yaml);
        assert!(
            result.is_err(),
            "missing inject_as field must return a parse error"
        );
    }

    #[test]
    fn session_type_invalid_value_returns_error() {
        let yaml = format!(
            "{}\nmemory:\n  inject_as: prepend_system\n  session_type: bad_type\n",
            MINIMAL_YAML
        );
        let result = serde_yaml::from_str::<AgentConfig>(&yaml);
        assert!(
            result.is_err(),
            "invalid session_type must return a YAML parse error"
        );
    }

    #[test]
    fn memory_inject_mode_integer_type_returns_error() {
        // inject_as must be a string — integer value should fail
        let yaml = format!("{}\nmemory:\n  inject_as: 1\n", MINIMAL_YAML);
        let result = serde_yaml::from_str::<AgentConfig>(&yaml);
        assert!(
            result.is_err(),
            "integer inject_as must return a parse error"
        );
    }

    // ── M3: boundary ─────────────────────────────────────────────────────────

    #[test]
    fn memory_auto_load_empty_list_is_accepted() {
        // Explicit empty list overrides the default — results in empty Vec
        let yaml = format!(
            "{}\nmemory:\n  auto_load: []\n  inject_as: prepend_system\n",
            MINIMAL_YAML
        );
        let config: AgentConfig = serde_yaml::from_str(&yaml).unwrap();
        let mem = config.memory.unwrap();
        assert!(
            mem.auto_load.is_empty(),
            "explicit empty auto_load should result in empty vec, got: {:?}",
            mem.auto_load
        );
    }

    #[test]
    fn memory_auto_load_single_entry() {
        let yaml = format!(
            "{}\nmemory:\n  auto_load: [\"AGENT.md\"]\n  inject_as: prepend_system\n",
            MINIMAL_YAML
        );
        let config: AgentConfig = serde_yaml::from_str(&yaml).unwrap();
        let mem = config.memory.unwrap();
        assert_eq!(mem.auto_load.len(), 1);
        assert_eq!(mem.auto_load[0], "AGENT.md");
    }

    #[test]
    fn full_config_with_all_sprint2_fields_parses() {
        // Integration: workspace_host + memory config together
        let yaml = r#"
name: knowledge-agent
description: "知识管理专员"
llm:
  provider: anthropic
  model: claude-haiku-4-5-20251001
  max_tokens: 4096
system_prompt: "你是知识管理专员。"
tools:
  toolset: coding
constraints: { max_turns: 20, timeout_secs: 300 }
sandbox:
  cpus: 2
  memory_mib: 1024
  workspace_host: ~/.sage/agents/knowledge
memory:
  auto_load: ["AGENT.md", "memory/MEMORY.md"]
  inject_as: prepend_system
  session_type: user_driven
"#;
        let config: AgentConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.name, "knowledge-agent");
        let sb = config.sandbox.unwrap();
        let ws = sb.workspace_host.unwrap();
        assert!(
            !ws.to_string_lossy().starts_with('~'),
            "workspace_host tilde must be expanded"
        );
        assert!(ws.to_string_lossy().ends_with(".sage/agents/knowledge"));
        let mem = config.memory.unwrap();
        assert_eq!(mem.auto_load.len(), 2);
        assert!(matches!(mem.inject_as, MemoryInjectMode::PrependSystem));
        assert!(matches!(mem.session_type.unwrap(), SessionType::UserDriven));
    }

    // ── Sprint 3 — v0.8: SandboxMode ─────────────────────────────────────────

    const S3_BASE_YAML: &str = "name: t\ndescription: \"\"\nllm: { provider: test, model: m, max_tokens: 256 }\nsystem_prompt: test\ntools: {}\nconstraints: { max_turns: 5, timeout_secs: 60 }";

    fn minimal_yaml_with_sandbox(sandbox_extra: &str) -> String {
        format!("{S3_BASE_YAML}\nsandbox:\n  cpus: 1\n  memory_mib: 512\n{sandbox_extra}")
    }

    #[test]
    fn sandbox_mode_defaults_to_microvm() {
        let yaml = minimal_yaml_with_sandbox("");
        let config: AgentConfig = serde_yaml::from_str(&yaml).unwrap();
        let sb = config.sandbox.unwrap();
        assert_eq!(
            sb.mode,
            SandboxMode::Microvm,
            "default mode should be Microvm"
        );
    }

    #[test]
    fn sandbox_mode_host_parses() {
        let yaml = minimal_yaml_with_sandbox("  mode: host\n");
        let config: AgentConfig = serde_yaml::from_str(&yaml).unwrap();
        let sb = config.sandbox.unwrap();
        assert_eq!(sb.mode, SandboxMode::Host);
    }

    #[test]
    fn sandbox_mode_microvm_explicit_parses() {
        let yaml = minimal_yaml_with_sandbox("  mode: microvm\n");
        let config: AgentConfig = serde_yaml::from_str(&yaml).unwrap();
        let sb = config.sandbox.unwrap();
        assert_eq!(sb.mode, SandboxMode::Microvm);
    }

    #[test]
    fn sandbox_mode_invalid_string_returns_error() {
        let yaml = minimal_yaml_with_sandbox("  mode: docker\n");
        let result = serde_yaml::from_str::<AgentConfig>(&yaml);
        assert!(result.is_err(), "unknown sandbox mode must fail to parse");
    }

    #[test]
    fn sandbox_mode_host_preserves_other_fields() {
        let yaml = minimal_yaml_with_sandbox("  mode: host\n  workspace_host: /tmp/ws\n");
        let config: AgentConfig = serde_yaml::from_str(&yaml).unwrap();
        let sb = config.sandbox.unwrap();
        assert_eq!(sb.mode, SandboxMode::Host);
        assert!(
            sb.workspace_host.is_some(),
            "other fields must be preserved alongside mode: host"
        );
    }

    #[test]
    fn sandbox_mode_roundtrips_via_serde_yaml() {
        // Verify SandboxMode::Host serialises back to "host"
        let mode = SandboxMode::Host;
        let s = serde_yaml::to_string(&mode).unwrap();
        assert!(
            s.trim() == "host",
            "Host must serialise to 'host', got: {s:?}"
        );

        let mode = SandboxMode::Microvm;
        let s = serde_yaml::to_string(&mode).unwrap();
        assert!(
            s.trim() == "microvm",
            "Microvm must serialise to 'microvm', got: {s:?}"
        );
    }

    // ── Sprint 3 — v0.8: RootfsTier ──────────────────────────────────────────

    #[test]
    fn rootfs_tier_defaults_to_minimal() {
        let yaml = minimal_yaml_with_sandbox("");
        let config: AgentConfig = serde_yaml::from_str(&yaml).unwrap();
        let sb = config.sandbox.unwrap();
        assert_eq!(
            sb.rootfs,
            RootfsTier::Minimal,
            "default rootfs should be Minimal"
        );
    }

    #[test]
    fn rootfs_tier_standard_parses() {
        let yaml = minimal_yaml_with_sandbox("  rootfs: standard\n");
        let config: AgentConfig = serde_yaml::from_str(&yaml).unwrap();
        let sb = config.sandbox.unwrap();
        assert_eq!(sb.rootfs, RootfsTier::Standard);
    }

    #[test]
    fn rootfs_tier_minimal_explicit_parses() {
        let yaml = minimal_yaml_with_sandbox("  rootfs: minimal\n");
        let config: AgentConfig = serde_yaml::from_str(&yaml).unwrap();
        let sb = config.sandbox.unwrap();
        assert_eq!(sb.rootfs, RootfsTier::Minimal);
    }

    #[test]
    fn rootfs_tier_invalid_string_returns_error() {
        let yaml = minimal_yaml_with_sandbox("  rootfs: ubuntu\n");
        let result = serde_yaml::from_str::<AgentConfig>(&yaml);
        assert!(result.is_err(), "unknown rootfs tier must fail to parse");
    }

    #[test]
    fn rootfs_tier_roundtrips_via_serde_yaml() {
        let tier = RootfsTier::Standard;
        let s = serde_yaml::to_string(&tier).unwrap();
        assert!(
            s.trim() == "standard",
            "Standard must serialise to 'standard', got: {s:?}"
        );

        let tier = RootfsTier::Minimal;
        let s = serde_yaml::to_string(&tier).unwrap();
        assert!(
            s.trim() == "minimal",
            "Minimal must serialise to 'minimal', got: {s:?}"
        );
    }

    #[test]
    fn sandbox_mode_and_rootfs_parse_together() {
        let yaml = minimal_yaml_with_sandbox("  mode: host\n  rootfs: standard\n");
        let config: AgentConfig = serde_yaml::from_str(&yaml).unwrap();
        let sb = config.sandbox.unwrap();
        assert_eq!(sb.mode, SandboxMode::Host);
        assert_eq!(sb.rootfs, RootfsTier::Standard);
    }

    #[test]
    fn existing_sandbox_config_without_mode_or_rootfs_still_parses() {
        // Backward compat: configs written before Sprint 3 must still parse cleanly.
        let yaml = minimal_yaml_with_sandbox("  network: airgapped\n  exec_timeout_secs: 30\n");
        let config: AgentConfig = serde_yaml::from_str(&yaml).unwrap();
        let sb = config.sandbox.unwrap();
        assert_eq!(
            sb.mode,
            SandboxMode::Microvm,
            "missing mode defaults to Microvm"
        );
        assert_eq!(
            sb.rootfs,
            RootfsTier::Minimal,
            "missing rootfs defaults to Minimal"
        );
    }

    #[test]
    fn sandbox_mode_microvm_with_rootfs_standard() {
        // Both non-default values in opposite corners of the matrix
        let yaml = minimal_yaml_with_sandbox("  mode: microvm\n  rootfs: standard\n");
        let config: AgentConfig = serde_yaml::from_str(&yaml).unwrap();
        let sb = config.sandbox.unwrap();
        assert_eq!(sb.mode, SandboxMode::Microvm);
        assert_eq!(sb.rootfs, RootfsTier::Standard);
    }

    #[test]
    fn sandbox_mode_integer_type_returns_error() {
        // YAML integer where a string enum is expected must fail cleanly
        let yaml = minimal_yaml_with_sandbox("  mode: 1\n");
        let result = serde_yaml::from_str::<AgentConfig>(&yaml);
        assert!(
            result.is_err(),
            "integer-typed mode must fail: expected string enum 'microvm'/'host'"
        );
    }

    #[test]
    fn rootfs_tier_integer_type_returns_error() {
        let yaml = minimal_yaml_with_sandbox("  rootfs: 2\n");
        let result = serde_yaml::from_str::<AgentConfig>(&yaml);
        assert!(
            result.is_err(),
            "integer-typed rootfs must fail: expected string enum"
        );
    }

    // ── Sprint 3 — v0.8: SandboxMode::with_dev_override ─────────────────────

    #[test]
    fn dev_override_true_forces_host_mode() {
        // --dev=true must translate to SandboxMode::Host regardless of config value
        let result = SandboxMode::Microvm.with_dev_override(true);
        assert_eq!(
            result,
            SandboxMode::Host,
            "--dev=true must override to Host"
        );
    }

    #[test]
    fn dev_override_false_preserves_microvm() {
        let result = SandboxMode::Microvm.with_dev_override(false);
        assert_eq!(
            result,
            SandboxMode::Microvm,
            "--dev=false must leave Microvm unchanged"
        );
    }

    #[test]
    fn dev_override_false_preserves_host() {
        let result = SandboxMode::Host.with_dev_override(false);
        assert_eq!(
            result,
            SandboxMode::Host,
            "--dev=false must leave Host unchanged"
        );
    }

    // ========================================================================
    // Sprint 4 — H1 HookConfig + H2 HarnessConfig YAML parsing
    // ========================================================================

    // ── HookConfig — basic parsing ────────────────────────────────────────────

    #[test]
    fn parse_hooks_pre_tool_use() {
        let yaml = r#"
name: t
description: "t"
llm: { provider: a, model: b, max_tokens: 1 }
system_prompt: "t"
tools: {}
constraints: { max_turns: 1, timeout_secs: 1 }
hooks:
  pre_tool_use:
    - command: "/scripts/validate-tool.sh"
"#;
        let config: AgentConfig = serde_yaml::from_str(yaml).unwrap();
        let hooks = config.hooks.expect("hooks should be Some");
        assert_eq!(hooks.pre_tool_use.len(), 1);
        assert_eq!(hooks.pre_tool_use[0].command, "/scripts/validate-tool.sh");
    }

    #[test]
    fn parse_hooks_post_tool_use() {
        let yaml = r#"
name: t
description: "t"
llm: { provider: a, model: b, max_tokens: 1 }
system_prompt: "t"
tools: {}
constraints: { max_turns: 1, timeout_secs: 1 }
hooks:
  post_tool_use:
    - command: "/scripts/log-tool.sh"
"#;
        let config: AgentConfig = serde_yaml::from_str(yaml).unwrap();
        let hooks = config.hooks.expect("hooks should be Some");
        assert_eq!(hooks.post_tool_use.len(), 1);
        assert_eq!(hooks.post_tool_use[0].command, "/scripts/log-tool.sh");
    }

    #[test]
    fn parse_hooks_stop_event() {
        let yaml = r#"
name: t
description: "t"
llm: { provider: a, model: b, max_tokens: 1 }
system_prompt: "t"
tools: {}
constraints: { max_turns: 1, timeout_secs: 1 }
hooks:
  stop:
    - command: "/scripts/eval.sh"
      timeout_secs: 30
"#;
        let config: AgentConfig = serde_yaml::from_str(yaml).unwrap();
        let hooks = config.hooks.expect("hooks should be Some");
        assert_eq!(hooks.stop.len(), 1);
        assert_eq!(hooks.stop[0].command, "/scripts/eval.sh");
        assert_eq!(hooks.stop[0].timeout_secs, Some(30));
    }

    #[test]
    fn hook_config_timeout_secs_defaults_to_none_when_omitted() {
        let yaml = r#"
name: t
description: "t"
llm: { provider: a, model: b, max_tokens: 1 }
system_prompt: "t"
tools: {}
constraints: { max_turns: 1, timeout_secs: 1 }
hooks:
  pre_tool_use:
    - command: "/scripts/check.sh"
"#;
        let config: AgentConfig = serde_yaml::from_str(yaml).unwrap();
        let hooks = config.hooks.unwrap();
        assert_eq!(
            hooks.pre_tool_use[0].timeout_secs, None,
            "timeout_secs must be None when omitted"
        );
    }

    #[test]
    fn parse_hooks_multiple_per_event() {
        // Multiple hooks per event type execute in order.
        let yaml = r#"
name: t
description: "t"
llm: { provider: a, model: b, max_tokens: 1 }
system_prompt: "t"
tools: {}
constraints: { max_turns: 1, timeout_secs: 1 }
hooks:
  pre_tool_use:
    - command: "/scripts/check1.sh"
    - command: "/scripts/check2.sh"
      timeout_secs: 10
"#;
        let config: AgentConfig = serde_yaml::from_str(yaml).unwrap();
        let hooks = config.hooks.unwrap();
        assert_eq!(hooks.pre_tool_use.len(), 2);
        assert_eq!(hooks.pre_tool_use[0].command, "/scripts/check1.sh");
        assert_eq!(hooks.pre_tool_use[0].timeout_secs, None);
        assert_eq!(hooks.pre_tool_use[1].command, "/scripts/check2.sh");
        assert_eq!(hooks.pre_tool_use[1].timeout_secs, Some(10));
    }

    #[test]
    fn parse_hooks_all_three_event_types_together() {
        let yaml = r#"
name: t
description: "t"
llm: { provider: a, model: b, max_tokens: 1 }
system_prompt: "t"
tools: {}
constraints: { max_turns: 1, timeout_secs: 1 }
hooks:
  pre_tool_use:
    - command: "/hooks/pre.sh"
  post_tool_use:
    - command: "/hooks/post.sh"
  stop:
    - command: "/hooks/stop.sh"
"#;
        let config: AgentConfig = serde_yaml::from_str(yaml).unwrap();
        let hooks = config.hooks.unwrap();
        assert_eq!(hooks.pre_tool_use[0].command, "/hooks/pre.sh");
        assert_eq!(hooks.post_tool_use[0].command, "/hooks/post.sh");
        assert_eq!(hooks.stop[0].command, "/hooks/stop.sh");
    }

    // ── HooksConfig defaults ──────────────────────────────────────────────────

    #[test]
    fn empty_hooks_section_gives_empty_vecs() {
        // hooks: {} with no event keys → all three lists are empty
        let yaml = r#"
name: t
description: "t"
llm: { provider: a, model: b, max_tokens: 1 }
system_prompt: "t"
tools: {}
constraints: { max_turns: 1, timeout_secs: 1 }
hooks: {}
"#;
        let config: AgentConfig = serde_yaml::from_str(yaml).unwrap();
        let hooks = config
            .hooks
            .expect("hooks: {} must deserialize as Some(HooksConfig)");
        assert!(
            hooks.pre_tool_use.is_empty(),
            "pre_tool_use should be empty when omitted"
        );
        assert!(
            hooks.post_tool_use.is_empty(),
            "post_tool_use should be empty when omitted"
        );
        assert!(hooks.stop.is_empty(), "stop should be empty when omitted");
    }

    #[test]
    fn agent_without_hooks_field_is_none() {
        // FEISHU_YAML has no hooks field — backward compat must give None
        let config: AgentConfig = serde_yaml::from_str(FEISHU_YAML).unwrap();
        assert!(
            config.hooks.is_none(),
            "omitted hooks field must deserialize as None"
        );
    }

    // ── HarnessConfig parsing ─────────────────────────────────────────────────

    #[test]
    fn parse_harness_minimal() {
        let yaml = r#"
name: t
description: "t"
llm: { provider: a, model: b, max_tokens: 1 }
system_prompt: "t"
tools: {}
constraints: { max_turns: 1, timeout_secs: 1 }
harness:
  evaluator: "/scripts/eval.sh"
"#;
        let config: AgentConfig = serde_yaml::from_str(yaml).unwrap();
        let harness = config.harness.expect("harness should be Some");
        assert_eq!(harness.evaluator, "/scripts/eval.sh");
    }

    #[test]
    fn parse_harness_with_explicit_timeout() {
        let yaml = r#"
name: t
description: "t"
llm: { provider: a, model: b, max_tokens: 1 }
system_prompt: "t"
tools: {}
constraints: { max_turns: 1, timeout_secs: 1 }
harness:
  evaluator: "/scripts/eval.sh"
  timeout_secs: 60
"#;
        let config: AgentConfig = serde_yaml::from_str(yaml).unwrap();
        let harness = config.harness.unwrap();
        assert_eq!(harness.evaluator, "/scripts/eval.sh");
        assert_eq!(harness.timeout_secs, Some(60));
    }

    #[test]
    fn harness_timeout_defaults_to_none_when_omitted() {
        let yaml = r#"
name: t
description: "t"
llm: { provider: a, model: b, max_tokens: 1 }
system_prompt: "t"
tools: {}
constraints: { max_turns: 1, timeout_secs: 1 }
harness:
  evaluator: "/scripts/eval.sh"
"#;
        let config: AgentConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(
            config.harness.unwrap().timeout_secs,
            None,
            "harness timeout_secs must be None when omitted"
        );
    }

    #[test]
    fn agent_without_harness_field_is_none() {
        let config: AgentConfig = serde_yaml::from_str(FEISHU_YAML).unwrap();
        assert!(
            config.harness.is_none(),
            "omitted harness field must be None"
        );
    }

    // ── Backward compatibility ────────────────────────────────────────────────

    #[test]
    fn existing_config_still_parses_with_hooks_and_harness_as_none() {
        // Any config without hooks/harness must still parse cleanly
        let config: AgentConfig = serde_yaml::from_str(FEISHU_YAML).unwrap();
        assert_eq!(config.name, "feishu-assistant");
        assert!(config.hooks.is_none());
        assert!(config.harness.is_none());
    }

    // ── Integration — full Sprint 4 config ───────────────────────────────────

    // ── HookConfig edge cases ─────────────────────────────────────────────────

    #[test]
    fn hook_timeout_secs_zero_is_valid() {
        // timeout_secs = 0 is a valid u32 — parser must accept it
        let yaml = r#"
name: t
description: "t"
llm: { provider: a, model: b, max_tokens: 1 }
system_prompt: "t"
tools: {}
constraints: { max_turns: 1, timeout_secs: 1 }
hooks:
  stop:
    - command: "/scripts/eval.sh"
      timeout_secs: 0
"#;
        let config: AgentConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(
            config.hooks.unwrap().stop[0].timeout_secs,
            Some(0),
            "timeout_secs: 0 must parse as Some(0)"
        );
    }

    #[test]
    fn hooks_empty_pre_tool_use_list_is_empty_vec() {
        // hooks: pre_tool_use: [] should parse as Some(HooksConfig) with empty vec,
        // not as None or parse error.
        let yaml = r#"
name: t
description: "t"
llm: { provider: a, model: b, max_tokens: 1 }
system_prompt: "t"
tools: {}
constraints: { max_turns: 1, timeout_secs: 1 }
hooks:
  pre_tool_use: []
"#;
        let config: AgentConfig = serde_yaml::from_str(yaml).unwrap();
        let hooks = config.hooks.expect("hooks should be Some");
        assert!(
            hooks.pre_tool_use.is_empty(),
            "explicit empty list must parse as empty Vec, not None"
        );
    }

    #[test]
    fn hook_command_empty_string_parses_successfully() {
        // An empty command string is structurally valid YAML; parse succeeds.
        // Execution-layer validation is the executor's responsibility, not the parser's.
        let yaml = r#"
name: t
description: "t"
llm: { provider: a, model: b, max_tokens: 1 }
system_prompt: "t"
tools: {}
constraints: { max_turns: 1, timeout_secs: 1 }
hooks:
  pre_tool_use:
    - command: ""
"#;
        let config: AgentConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(
            config.hooks.unwrap().pre_tool_use[0].command,
            "",
            "empty command string must parse as empty String"
        );
    }

    // ── HookConfig error paths ────────────────────────────────────────────────

    #[test]
    fn hooks_field_as_scalar_is_parse_error() {
        // hooks: "invalid" (scalar, not mapping) must fail deserialization
        let yaml = r#"
name: t
description: "t"
llm: { provider: a, model: b, max_tokens: 1 }
system_prompt: "t"
tools: {}
constraints: { max_turns: 1, timeout_secs: 1 }
hooks: "invalid string value"
"#;
        assert!(
            serde_yaml::from_str::<AgentConfig>(yaml).is_err(),
            "hooks: scalar must be a parse error"
        );
    }

    #[test]
    fn hook_timeout_secs_negative_is_parse_error() {
        // timeout_secs is u32 — negative values must fail deserialization
        let yaml = r#"
name: t
description: "t"
llm: { provider: a, model: b, max_tokens: 1 }
system_prompt: "t"
tools: {}
constraints: { max_turns: 1, timeout_secs: 1 }
hooks:
  stop:
    - command: "/scripts/eval.sh"
      timeout_secs: -1
"#;
        assert!(
            serde_yaml::from_str::<AgentConfig>(yaml).is_err(),
            "negative timeout_secs must be a parse error (u32 cannot be negative)"
        );
    }

    #[test]
    fn hook_timeout_secs_string_is_parse_error() {
        // timeout_secs must be numeric, not a string
        let yaml = r#"
name: t
description: "t"
llm: { provider: a, model: b, max_tokens: 1 }
system_prompt: "t"
tools: {}
constraints: { max_turns: 1, timeout_secs: 1 }
hooks:
  pre_tool_use:
    - command: "/scripts/check.sh"
      timeout_secs: "thirty"
"#;
        assert!(
            serde_yaml::from_str::<AgentConfig>(yaml).is_err(),
            "string timeout_secs must be a parse error"
        );
    }

    #[test]
    fn harness_missing_evaluator_is_parse_error() {
        // `evaluator` is a required field in HarnessConfig — `harness: {}` must fail
        let yaml = r#"
name: t
description: "t"
llm: { provider: a, model: b, max_tokens: 1 }
system_prompt: "t"
tools: {}
constraints: { max_turns: 1, timeout_secs: 1 }
harness: {}
"#;
        assert!(
            serde_yaml::from_str::<AgentConfig>(yaml).is_err(),
            "harness: {{}} (missing evaluator) must be a parse error"
        );
    }

    #[test]
    fn harness_timeout_secs_negative_is_parse_error() {
        let yaml = r#"
name: t
description: "t"
llm: { provider: a, model: b, max_tokens: 1 }
system_prompt: "t"
tools: {}
constraints: { max_turns: 1, timeout_secs: 1 }
harness:
  evaluator: "/scripts/eval.sh"
  timeout_secs: -5
"#;
        assert!(
            serde_yaml::from_str::<AgentConfig>(yaml).is_err(),
            "negative harness timeout_secs must be a parse error (u32)"
        );
    }

    // ── Integration — full Sprint 4 config ───────────────────────────────────

    #[test]
    fn parse_full_sprint4_config_hooks_and_harness() {
        let yaml = r#"
name: eval-agent
description: "Agent with sprint4 hooks + harness"
llm: { provider: anthropic, model: claude-haiku-4-5-20251001, max_tokens: 4096 }
system_prompt: "evaluator agent"
tools:
  bash: { allowed_binaries: [python3] }
  read: { allowed_paths: ["/workspace"] }
constraints: { max_turns: 10, timeout_secs: 120 }
hooks:
  pre_tool_use:
    - command: "/hooks/pre-tool.sh"
      timeout_secs: 5
  post_tool_use:
    - command: "/hooks/post-tool.sh"
  stop:
    - command: "/hooks/stop.sh"
      timeout_secs: 30
harness:
  evaluator: "/scripts/eval.sh"
  timeout_secs: 60
"#;
        let config: AgentConfig = serde_yaml::from_str(yaml).unwrap();
        let hooks = config.hooks.unwrap();
        let harness = config.harness.unwrap();

        assert_eq!(hooks.pre_tool_use.len(), 1);
        assert_eq!(hooks.pre_tool_use[0].command, "/hooks/pre-tool.sh");
        assert_eq!(hooks.pre_tool_use[0].timeout_secs, Some(5));

        assert_eq!(hooks.post_tool_use.len(), 1);
        assert_eq!(hooks.post_tool_use[0].command, "/hooks/post-tool.sh");
        assert_eq!(hooks.post_tool_use[0].timeout_secs, None);

        assert_eq!(hooks.stop.len(), 1);
        assert_eq!(hooks.stop[0].command, "/hooks/stop.sh");
        assert_eq!(hooks.stop[0].timeout_secs, Some(30));

        assert_eq!(harness.evaluator, "/scripts/eval.sh");
        assert_eq!(harness.timeout_secs, Some(60));
    }

    // ========================================================================
    // Sprint 6 S6.5: HooksConfig extended fields (8 lifecycle events)
    //
    // YAML keys are snake_case (session_start / pre_compact / …). The runtime
    // HookEvent enum yields PascalCase names ("SessionStart", "PreCompact")
    // via HookEvent::name() — that asymmetry is intentional and only affects
    // the hook-script payload, not config parsing tested here.
    // ========================================================================

    // ── Struct-level basics ────────────────────────────────────────────────────

    #[test]
    fn hooks_config_default_all_fields_empty() {
        // Default must produce empty Vecs for every one of the 8 event fields,
        // including the 5 added in Sprint 6.
        let hc = HooksConfig::default();
        assert!(
            hc.pre_tool_use.is_empty(),
            "pre_tool_use must default to empty"
        );
        assert!(
            hc.post_tool_use.is_empty(),
            "post_tool_use must default to empty"
        );
        assert!(hc.stop.is_empty(), "stop must default to empty");
        assert!(
            hc.session_start.is_empty(),
            "session_start must default to empty"
        );
        assert!(
            hc.session_end.is_empty(),
            "session_end must default to empty"
        );
        assert!(
            hc.user_prompt_submit.is_empty(),
            "user_prompt_submit must default to empty"
        );
        assert!(
            hc.pre_compact.is_empty(),
            "pre_compact must default to empty"
        );
        assert!(
            hc.post_compact.is_empty(),
            "post_compact must default to empty"
        );
    }

    #[test]
    fn hooks_config_serde_roundtrip_preserves_new_fields() {
        // Round-trip through serde_yaml to confirm all 5 new fields are
        // serialized with stable snake_case keys and deserialized back.
        let hc = HooksConfig {
            pre_tool_use: vec![HookConfig {
                command: "pre-tool.sh".to_string(),
                timeout_secs: None,
            }],
            post_tool_use: vec![HookConfig {
                command: "post-tool.sh".to_string(),
                timeout_secs: None,
            }],
            stop: vec![HookConfig {
                command: "stop.sh".to_string(),
                timeout_secs: None,
            }],
            session_start: vec![HookConfig {
                command: "ss.sh".to_string(),
                timeout_secs: Some(10),
            }],
            session_end: vec![HookConfig {
                command: "se.sh".to_string(),
                timeout_secs: Some(20),
            }],
            user_prompt_submit: vec![HookConfig {
                command: "ups.sh".to_string(),
                timeout_secs: None,
            }],
            pre_compact: vec![HookConfig {
                command: "pre-compact.sh".to_string(),
                timeout_secs: Some(30),
            }],
            post_compact: vec![HookConfig {
                command: "post-compact.sh".to_string(),
                timeout_secs: None,
            }],
        };

        let yaml = serde_yaml::to_string(&hc).expect("serialize HooksConfig");
        let decoded: HooksConfig = serde_yaml::from_str(&yaml).expect("deserialize HooksConfig");

        assert_eq!(decoded.session_start.len(), 1);
        assert_eq!(decoded.session_start[0].command, "ss.sh");
        assert_eq!(decoded.session_start[0].timeout_secs, Some(10));

        assert_eq!(decoded.session_end.len(), 1);
        assert_eq!(decoded.session_end[0].command, "se.sh");
        assert_eq!(decoded.session_end[0].timeout_secs, Some(20));

        assert_eq!(decoded.user_prompt_submit.len(), 1);
        assert_eq!(decoded.user_prompt_submit[0].command, "ups.sh");

        assert_eq!(decoded.pre_compact.len(), 1);
        assert_eq!(decoded.pre_compact[0].command, "pre-compact.sh");
        assert_eq!(decoded.pre_compact[0].timeout_secs, Some(30));

        assert_eq!(decoded.post_compact.len(), 1);
        assert_eq!(decoded.post_compact[0].command, "post-compact.sh");
    }

    #[test]
    fn hooks_config_new_fields_are_public() {
        // Compile-time check: all 5 new fields are publicly accessible by
        // their snake_case identifiers. If a field is renamed or made private,
        // this test fails to compile.
        let hc = HooksConfig::default();
        let _ = &hc.session_start;
        let _ = &hc.session_end;
        let _ = &hc.user_prompt_submit;
        let _ = &hc.pre_compact;
        let _ = &hc.post_compact;
    }

    // ── YAML parsing — backward compatibility ─────────────────────────────────

    #[test]
    fn hooks_config_yaml_only_legacy_fields_parses_with_empty_new_fields() {
        // Legacy config writing only the Sprint-4 fields must still parse, with
        // all 5 Sprint-6 fields defaulted to empty Vec.
        let yaml = r#"
name: t
description: "t"
llm: { provider: a, model: b, max_tokens: 1 }
system_prompt: "t"
tools: {}
constraints: { max_turns: 1, timeout_secs: 1 }
hooks:
  pre_tool_use:
    - command: "/scripts/pre.sh"
  post_tool_use:
    - command: "/scripts/post.sh"
  stop:
    - command: "/scripts/stop.sh"
"#;
        let config: AgentConfig = serde_yaml::from_str(yaml).unwrap();
        let hooks = config.hooks.expect("hooks block must deserialize");
        assert_eq!(hooks.pre_tool_use.len(), 1);
        assert_eq!(hooks.post_tool_use.len(), 1);
        assert_eq!(hooks.stop.len(), 1);
        assert!(
            hooks.session_start.is_empty(),
            "session_start defaults to empty"
        );
        assert!(
            hooks.session_end.is_empty(),
            "session_end defaults to empty"
        );
        assert!(
            hooks.user_prompt_submit.is_empty(),
            "user_prompt_submit defaults to empty"
        );
        assert!(
            hooks.pre_compact.is_empty(),
            "pre_compact defaults to empty"
        );
        assert!(
            hooks.post_compact.is_empty(),
            "post_compact defaults to empty"
        );
    }

    #[test]
    fn hooks_config_yaml_empty_block_parses_with_all_empty() {
        // `hooks: {}` with no child keys must yield Some(HooksConfig) where all
        // 8 event lists are empty.
        let yaml = r#"
name: t
description: "t"
llm: { provider: a, model: b, max_tokens: 1 }
system_prompt: "t"
tools: {}
constraints: { max_turns: 1, timeout_secs: 1 }
hooks: {}
"#;
        let config: AgentConfig = serde_yaml::from_str(yaml).unwrap();
        let hooks = config.hooks.expect("hooks: {} must deserialize as Some");
        assert!(hooks.pre_tool_use.is_empty());
        assert!(hooks.post_tool_use.is_empty());
        assert!(hooks.stop.is_empty());
        assert!(hooks.session_start.is_empty());
        assert!(hooks.session_end.is_empty());
        assert!(hooks.user_prompt_submit.is_empty());
        assert!(hooks.pre_compact.is_empty());
        assert!(hooks.post_compact.is_empty());
    }

    #[test]
    fn hooks_config_yaml_missing_hooks_section_is_none() {
        // AgentConfig without a hooks field must deserialize with hooks = None
        // (Sprint 6 additions do not change this existing contract).
        let config: AgentConfig = serde_yaml::from_str(FEISHU_YAML).unwrap();
        assert!(
            config.hooks.is_none(),
            "omitted hooks field must remain None after Sprint 6 additions"
        );
    }

    // ── YAML parsing — new fields (one test per field) ────────────────────────

    #[test]
    fn hooks_config_yaml_session_start_parses_command_list() {
        let yaml = r#"
name: t
description: "t"
llm: { provider: a, model: b, max_tokens: 1 }
system_prompt: "t"
tools: {}
constraints: { max_turns: 1, timeout_secs: 1 }
hooks:
  session_start:
    - command: "python init.py"
"#;
        let config: AgentConfig = serde_yaml::from_str(yaml).unwrap();
        let hooks = config.hooks.unwrap();
        assert_eq!(hooks.session_start.len(), 1);
        assert_eq!(hooks.session_start[0].command, "python init.py");
        assert_eq!(hooks.session_start[0].timeout_secs, None);
    }

    #[test]
    fn hooks_config_yaml_session_end_parses_command_list() {
        let yaml = r#"
name: t
description: "t"
llm: { provider: a, model: b, max_tokens: 1 }
system_prompt: "t"
tools: {}
constraints: { max_turns: 1, timeout_secs: 1 }
hooks:
  session_end:
    - command: "python cleanup.py"
      timeout_secs: 15
"#;
        let config: AgentConfig = serde_yaml::from_str(yaml).unwrap();
        let hooks = config.hooks.unwrap();
        assert_eq!(hooks.session_end.len(), 1);
        assert_eq!(hooks.session_end[0].command, "python cleanup.py");
        assert_eq!(hooks.session_end[0].timeout_secs, Some(15));
    }

    #[test]
    fn hooks_config_yaml_user_prompt_submit_parses_command_list() {
        let yaml = r#"
name: t
description: "t"
llm: { provider: a, model: b, max_tokens: 1 }
system_prompt: "t"
tools: {}
constraints: { max_turns: 1, timeout_secs: 1 }
hooks:
  user_prompt_submit:
    - command: "/scripts/log-prompt.sh"
"#;
        let config: AgentConfig = serde_yaml::from_str(yaml).unwrap();
        let hooks = config.hooks.unwrap();
        assert_eq!(hooks.user_prompt_submit.len(), 1);
        assert_eq!(
            hooks.user_prompt_submit[0].command,
            "/scripts/log-prompt.sh"
        );
        assert_eq!(hooks.user_prompt_submit[0].timeout_secs, None);
    }

    #[test]
    fn hooks_config_yaml_pre_compact_parses_command_list() {
        let yaml = r#"
name: t
description: "t"
llm: { provider: a, model: b, max_tokens: 1 }
system_prompt: "t"
tools: {}
constraints: { max_turns: 1, timeout_secs: 1 }
hooks:
  pre_compact:
    - command: "/scripts/snapshot.sh"
      timeout_secs: 45
"#;
        let config: AgentConfig = serde_yaml::from_str(yaml).unwrap();
        let hooks = config.hooks.unwrap();
        assert_eq!(hooks.pre_compact.len(), 1);
        assert_eq!(hooks.pre_compact[0].command, "/scripts/snapshot.sh");
        assert_eq!(hooks.pre_compact[0].timeout_secs, Some(45));
    }

    #[test]
    fn hooks_config_yaml_post_compact_parses_command_list() {
        let yaml = r#"
name: t
description: "t"
llm: { provider: a, model: b, max_tokens: 1 }
system_prompt: "t"
tools: {}
constraints: { max_turns: 1, timeout_secs: 1 }
hooks:
  post_compact:
    - command: "/scripts/log-compaction.sh"
"#;
        let config: AgentConfig = serde_yaml::from_str(yaml).unwrap();
        let hooks = config.hooks.unwrap();
        assert_eq!(hooks.post_compact.len(), 1);
        assert_eq!(hooks.post_compact[0].command, "/scripts/log-compaction.sh");
        assert_eq!(hooks.post_compact[0].timeout_secs, None);
    }

    // ── Composition & boundary cases ──────────────────────────────────────────

    #[test]
    fn hooks_config_yaml_all_eight_events_coexist() {
        // Specifying one hook per event across all 8 events must produce 8
        // single-element Vecs.
        let yaml = r#"
name: t
description: "t"
llm: { provider: a, model: b, max_tokens: 1 }
system_prompt: "t"
tools: {}
constraints: { max_turns: 1, timeout_secs: 1 }
hooks:
  pre_tool_use:
    - command: "/hooks/pre-tool.sh"
  post_tool_use:
    - command: "/hooks/post-tool.sh"
  stop:
    - command: "/hooks/stop.sh"
  session_start:
    - command: "/hooks/session-start.sh"
  session_end:
    - command: "/hooks/session-end.sh"
  user_prompt_submit:
    - command: "/hooks/user-prompt.sh"
  pre_compact:
    - command: "/hooks/pre-compact.sh"
  post_compact:
    - command: "/hooks/post-compact.sh"
"#;
        let config: AgentConfig = serde_yaml::from_str(yaml).unwrap();
        let hooks = config.hooks.unwrap();
        assert_eq!(hooks.pre_tool_use.len(), 1);
        assert_eq!(hooks.post_tool_use.len(), 1);
        assert_eq!(hooks.stop.len(), 1);
        assert_eq!(hooks.session_start.len(), 1);
        assert_eq!(hooks.session_end.len(), 1);
        assert_eq!(hooks.user_prompt_submit.len(), 1);
        assert_eq!(hooks.pre_compact.len(), 1);
        assert_eq!(hooks.post_compact.len(), 1);

        assert_eq!(hooks.session_start[0].command, "/hooks/session-start.sh");
        assert_eq!(hooks.session_end[0].command, "/hooks/session-end.sh");
        assert_eq!(hooks.user_prompt_submit[0].command, "/hooks/user-prompt.sh");
        assert_eq!(hooks.pre_compact[0].command, "/hooks/pre-compact.sh");
        assert_eq!(hooks.post_compact[0].command, "/hooks/post-compact.sh");
    }

    #[test]
    fn hooks_config_yaml_multi_commands_per_event_preserves_order() {
        // Multiple hooks for a single new-event field must preserve YAML order.
        let yaml = r#"
name: t
description: "t"
llm: { provider: a, model: b, max_tokens: 1 }
system_prompt: "t"
tools: {}
constraints: { max_turns: 1, timeout_secs: 1 }
hooks:
  pre_compact:
    - command: "/hooks/first.sh"
    - command: "/hooks/second.sh"
      timeout_secs: 20
    - command: "/hooks/third.sh"
"#;
        let config: AgentConfig = serde_yaml::from_str(yaml).unwrap();
        let hooks = config.hooks.unwrap();
        assert_eq!(hooks.pre_compact.len(), 3);
        assert_eq!(hooks.pre_compact[0].command, "/hooks/first.sh");
        assert_eq!(hooks.pre_compact[0].timeout_secs, None);
        assert_eq!(hooks.pre_compact[1].command, "/hooks/second.sh");
        assert_eq!(hooks.pre_compact[1].timeout_secs, Some(20));
        assert_eq!(hooks.pre_compact[2].command, "/hooks/third.sh");
        assert_eq!(hooks.pre_compact[2].timeout_secs, None);
    }

    #[test]
    fn hooks_config_yaml_unknown_event_key_is_ignored() {
        // HooksConfig does not use #[serde(deny_unknown_fields)], so unknown
        // event keys in YAML must be silently ignored — the 8 known fields
        // parse normally.
        let yaml = r#"
name: t
description: "t"
llm: { provider: a, model: b, max_tokens: 1 }
system_prompt: "t"
tools: {}
constraints: { max_turns: 1, timeout_secs: 1 }
hooks:
  random_event:
    - command: "foo"
  session_start:
    - command: "/hooks/ss.sh"
"#;
        let config: AgentConfig = serde_yaml::from_str(yaml)
            .expect("unknown hook event key must not cause a parse error");
        let hooks = config.hooks.unwrap();
        assert_eq!(hooks.session_start.len(), 1);
        assert_eq!(hooks.session_start[0].command, "/hooks/ss.sh");
    }

    // ── AgentConfig integration ───────────────────────────────────────────────

    #[test]
    fn agent_config_with_hooks_all_events_parses() {
        // Full AgentConfig with hooks for every one of the 8 events parses and
        // retains all payloads.
        let yaml = r#"
name: full-agent
description: "Agent with all 8 lifecycle hook events"
llm: { provider: anthropic, model: claude-haiku-4-5-20251001, max_tokens: 4096 }
system_prompt: "full agent"
tools:
  bash: { allowed_binaries: [python3] }
constraints: { max_turns: 10, timeout_secs: 120 }
hooks:
  pre_tool_use:
    - command: "/hooks/pre-tool.sh"
  post_tool_use:
    - command: "/hooks/post-tool.sh"
  stop:
    - command: "/hooks/stop.sh"
  session_start:
    - command: "/hooks/ss.sh"
  session_end:
    - command: "/hooks/se.sh"
  user_prompt_submit:
    - command: "/hooks/ups.sh"
  pre_compact:
    - command: "/hooks/pre-compact.sh"
  post_compact:
    - command: "/hooks/post-compact.sh"
"#;
        let config: AgentConfig = serde_yaml::from_str(yaml).unwrap();
        let hooks = config.hooks.expect("hooks block must be Some");
        // Sprint 4 fields intact.
        assert_eq!(hooks.pre_tool_use[0].command, "/hooks/pre-tool.sh");
        assert_eq!(hooks.post_tool_use[0].command, "/hooks/post-tool.sh");
        assert_eq!(hooks.stop[0].command, "/hooks/stop.sh");
        // Sprint 6 fields present.
        assert_eq!(hooks.session_start[0].command, "/hooks/ss.sh");
        assert_eq!(hooks.session_end[0].command, "/hooks/se.sh");
        assert_eq!(hooks.user_prompt_submit[0].command, "/hooks/ups.sh");
        assert_eq!(hooks.pre_compact[0].command, "/hooks/pre-compact.sh");
        assert_eq!(hooks.post_compact[0].command, "/hooks/post-compact.sh");
    }

    #[test]
    fn agent_config_hooks_optional_defaults_to_none() {
        // Reaffirm after Sprint 6 field additions: an AgentConfig YAML without
        // a top-level `hooks:` key still produces config.hooks == None.
        let yaml = r#"
name: no-hooks-agent
description: "Agent without any hooks section"
llm: { provider: a, model: b, max_tokens: 1 }
system_prompt: "t"
tools: {}
constraints: { max_turns: 1, timeout_secs: 1 }
"#;
        let config: AgentConfig = serde_yaml::from_str(yaml).unwrap();
        assert!(
            config.hooks.is_none(),
            "missing hooks section must deserialize to None"
        );
    }

    // ========================================================================
    // Sprint 8: ChannelConfig (Feishu)
    // ========================================================================

    const CHANNEL_BASE_YAML: &str = r#"
name: t
description: "t"
llm: { provider: a, model: b, max_tokens: 1 }
system_prompt: "t"
tools: {}
constraints: { max_turns: 1, timeout_secs: 1 }
"#;

    #[test]
    fn channel_config_default_webhook_port_is_3400() {
        // default_webhook_port() is exercised indirectly via serde default.
        let yaml = format!(
            r#"{CHANNEL_BASE_YAML}
channel:
  type: feishu
  app_id: cli_abc
  app_secret: secret123
"#
        );
        let config: AgentConfig = serde_yaml::from_str(&yaml).unwrap();
        match config.channel.expect("channel must be Some") {
            ChannelConfig::Feishu { webhook_port, .. } => {
                assert_eq!(webhook_port, 3400);
            }
        }
    }

    #[test]
    fn channel_config_parse_feishu_full_fields() {
        let yaml = format!(
            r#"{CHANNEL_BASE_YAML}
channel:
  type: feishu
  app_id: cli_abc
  app_secret: "secret-xyz"
  verification_token: "vtok-1"
  webhook_port: 4500
"#
        );
        let config: AgentConfig = serde_yaml::from_str(&yaml).unwrap();
        let ch = config.channel.expect("channel must be Some");
        match ch {
            ChannelConfig::Feishu {
                app_id,
                app_secret,
                verification_token,
                webhook_port,
            } => {
                assert_eq!(app_id, "cli_abc");
                assert_eq!(app_secret, "secret-xyz");
                assert_eq!(verification_token.as_deref(), Some("vtok-1"));
                assert_eq!(webhook_port, 4500);
            }
        }
    }

    #[test]
    fn channel_config_verification_token_optional() {
        let yaml = format!(
            r#"{CHANNEL_BASE_YAML}
channel:
  type: feishu
  app_id: cli_abc
  app_secret: "secret-xyz"
  webhook_port: 3400
"#
        );
        let config: AgentConfig = serde_yaml::from_str(&yaml).unwrap();
        match config.channel.unwrap() {
            ChannelConfig::Feishu {
                verification_token, ..
            } => {
                assert!(verification_token.is_none());
            }
        }
    }

    #[test]
    fn channel_config_webhook_port_defaults_to_3400_when_omitted() {
        let yaml = format!(
            r#"{CHANNEL_BASE_YAML}
channel:
  type: feishu
  app_id: cli_abc
  app_secret: s
  verification_token: v
"#
        );
        let config: AgentConfig = serde_yaml::from_str(&yaml).unwrap();
        match config.channel.unwrap() {
            ChannelConfig::Feishu { webhook_port, .. } => assert_eq!(webhook_port, 3400),
        }
    }

    #[test]
    fn channel_config_serde_roundtrip_preserves_fields() {
        let original = ChannelConfig::Feishu {
            app_id: "cli_round".into(),
            app_secret: "rts".into(),
            verification_token: Some("vt".into()),
            webhook_port: 8080,
        };
        let yaml = serde_yaml::to_string(&original).unwrap();
        let decoded: ChannelConfig = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(decoded, original);
    }

    #[test]
    fn agent_config_without_channel_section_defaults_to_none() {
        let config: AgentConfig = serde_yaml::from_str(CHANNEL_BASE_YAML).unwrap();
        assert!(config.channel.is_none());
    }

    #[test]
    fn channel_config_unknown_type_fails_to_parse() {
        let yaml = format!(
            r#"{CHANNEL_BASE_YAML}
channel:
  type: mirc
  app_id: x
  app_secret: y
"#
        );
        let result: Result<AgentConfig, _> = serde_yaml::from_str(&yaml);
        assert!(
            result.is_err(),
            "unknown channel type must fail to deserialize"
        );
    }

    // ========================================================================
    // M1: LlmConfig field changes — max_tokens Optional + context_window
    // ========================================================================

    const M1_BASE_YAML: &str = r#"
name: m1-test
description: "M1 test agent"
system_prompt: "test"
tools: {}
constraints: { max_turns: 1, timeout_secs: 30 }
"#;

    #[test]
    fn llm_config_max_tokens_optional_defaults_to_none() {
        // max_tokens field omitted entirely — should parse and give Option::None
        let yaml = format!(
            r#"{}
llm:
  provider: kimi
  model: kimi-k2.5
"#,
            M1_BASE_YAML
        );
        let config: AgentConfig = serde_yaml::from_str(&yaml).unwrap();
        assert!(
            config.llm.max_tokens.is_none(),
            "max_tokens omitted should be None, got: {:?}",
            config.llm.max_tokens
        );
    }

    #[test]
    fn llm_config_max_tokens_read_when_present() {
        let yaml = format!(
            r#"{}
llm:
  provider: kimi
  model: kimi-k2.5
  max_tokens: 8192
"#,
            M1_BASE_YAML
        );
        let config: AgentConfig = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(
            config.llm.max_tokens,
            Some(8192),
            "max_tokens: 8192 should parse to Some(8192)"
        );
    }

    #[test]
    fn llm_config_context_window_optional_defaults_to_none() {
        // context_window omitted — should parse and give None
        let yaml = format!(
            r#"{}
llm:
  provider: kimi
  model: kimi-k2.5
"#,
            M1_BASE_YAML
        );
        let config: AgentConfig = serde_yaml::from_str(&yaml).unwrap();
        assert!(
            config.llm.context_window.is_none(),
            "context_window omitted should be None, got: {:?}",
            config.llm.context_window
        );
    }

    #[test]
    fn llm_config_context_window_read_when_present() {
        let yaml = format!(
            r#"{}
llm:
  provider: kimi
  model: kimi-k2.5
  context_window: 262144
"#,
            M1_BASE_YAML
        );
        let config: AgentConfig = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(
            config.llm.context_window,
            Some(262144),
            "context_window: 262144 should parse to Some(262144)"
        );
    }

    #[test]
    fn llm_config_arbitrary_model_string_accepted_at_serde_layer() {
        // Serde layer does NOT validate model string — weak binding
        let yaml = format!(
            r#"{}
llm:
  provider: kimi
  model: "literally-anything-xyz-123"
"#,
            M1_BASE_YAML
        );
        let result: Result<AgentConfig, _> = serde_yaml::from_str(&yaml);
        assert!(
            result.is_ok(),
            "arbitrary model string must parse at serde layer (weak binding)"
        );
        let config = result.unwrap();
        assert_eq!(config.llm.model, "literally-anything-xyz-123");
    }

    #[test]
    fn llm_config_empty_model_still_parses_at_serde_layer() {
        // Empty model string — serde must not reject it (post-validation handles this)
        let yaml = format!(
            r#"{}
llm:
  provider: kimi
  model: ""
"#,
            M1_BASE_YAML
        );
        let result: Result<AgentConfig, _> = serde_yaml::from_str(&yaml);
        assert!(
            result.is_ok(),
            "empty model string must parse at serde layer; post-validation rejects it later"
        );
    }

    #[test]
    fn llm_config_unknown_provider_parses_at_serde_layer() {
        // Serde layer does NOT validate provider — only validate_provider() does
        let yaml = format!(
            r#"{}
llm:
  provider: "not-a-real-provider-xyz"
  model: some-model
"#,
            M1_BASE_YAML
        );
        let result: Result<AgentConfig, _> = serde_yaml::from_str(&yaml);
        assert!(
            result.is_ok(),
            "unknown provider must parse at serde layer (validation is post-parse)"
        );
    }

    #[test]
    fn load_config_rejects_unknown_provider_with_helpful_error() {
        // validate_provider("not-a-real-provider-xyz") must return Err with hint
        let result = validate_provider("not-a-real-provider-xyz");
        assert!(
            result.is_err(),
            "validate_provider should reject unknown provider"
        );
        let msg = result.unwrap_err();
        // Error message must hint at valid providers
        assert!(
            msg.contains("valid providers") || msg.contains("provider"),
            "error message should mention valid providers, got: {msg}"
        );
        // Should mention the bad provider id
        assert!(
            msg.contains("not-a-real-provider-xyz"),
            "error message should echo back the invalid provider id, got: {msg}"
        );
    }

    #[test]
    fn load_config_rejects_empty_provider() {
        let result = validate_provider("");
        assert!(
            result.is_err(),
            "validate_provider should reject empty string provider"
        );
    }

    #[test]
    fn validate_provider_accepts_every_known_provider_from_spec_table() {
        // Data-driven: new providers in list_providers() auto-covered.
        // Hardcoding 17 ids would drift when Sprint 12 M1 v2 added the 18th
        // (openrouter) and the test would silently skip it.
        for spec in sage_runtime::llm::provider_specs::list_providers() {
            let result = validate_provider(spec.id);
            assert!(
                result.is_ok(),
                "validate_provider(\"{}\") should succeed but got: {:?}",
                spec.id,
                result
            );
        }
    }
}
