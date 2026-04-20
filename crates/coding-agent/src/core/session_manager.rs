/// Session manager — append-only conversation trees stored as JSONL files.
///
/// Mirrors pi-mono packages/coding-agent/src/core/session-manager.ts
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use ulid::Ulid;

pub const CURRENT_SESSION_VERSION: u32 = 3;

// ============================================================================
// Types
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionHeader {
    #[serde(rename = "type")]
    pub entry_type: String, // always "session"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<u32>,
    pub id: String,
    pub timestamp: String,
    pub cwd: String,
    #[serde(rename = "parentSession", skip_serializing_if = "Option::is_none")]
    pub parent_session: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewSessionOptions {
    pub id: Option<String>,
    pub parent_session: Option<String>,
}

// ---- Entry base fields (embedded via serde flatten) ----

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionMessageEntry {
    #[serde(rename = "type")]
    pub entry_type: String,
    pub id: String,
    #[serde(rename = "parentId")]
    pub parent_id: Option<String>,
    pub timestamp: String,
    pub message: Value, // AgentMessage as JSON (role-tagged union)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThinkingLevelChangeEntry {
    #[serde(rename = "type")]
    pub entry_type: String,
    pub id: String,
    #[serde(rename = "parentId")]
    pub parent_id: Option<String>,
    pub timestamp: String,
    #[serde(rename = "thinkingLevel")]
    pub thinking_level: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelChangeEntry {
    #[serde(rename = "type")]
    pub entry_type: String,
    pub id: String,
    #[serde(rename = "parentId")]
    pub parent_id: Option<String>,
    pub timestamp: String,
    pub provider: String,
    #[serde(rename = "modelId")]
    pub model_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompactionEntry {
    #[serde(rename = "type")]
    pub entry_type: String,
    pub id: String,
    #[serde(rename = "parentId")]
    pub parent_id: Option<String>,
    pub timestamp: String,
    pub summary: String,
    #[serde(rename = "firstKeptEntryId")]
    pub first_kept_entry_id: String,
    #[serde(rename = "tokensBefore")]
    pub tokens_before: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<Value>,
    #[serde(rename = "fromHook", skip_serializing_if = "Option::is_none")]
    pub from_hook: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BranchSummaryEntry {
    #[serde(rename = "type")]
    pub entry_type: String,
    pub id: String,
    #[serde(rename = "parentId")]
    pub parent_id: Option<String>,
    pub timestamp: String,
    #[serde(rename = "fromId")]
    pub from_id: String,
    pub summary: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<Value>,
    #[serde(rename = "fromHook", skip_serializing_if = "Option::is_none")]
    pub from_hook: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomEntry {
    #[serde(rename = "type")]
    pub entry_type: String,
    #[serde(rename = "customType")]
    pub custom_type: String,
    pub id: String,
    #[serde(rename = "parentId")]
    pub parent_id: Option<String>,
    pub timestamp: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LabelEntry {
    #[serde(rename = "type")]
    pub entry_type: String,
    pub id: String,
    #[serde(rename = "parentId")]
    pub parent_id: Option<String>,
    pub timestamp: String,
    #[serde(rename = "targetId")]
    pub target_id: String,
    pub label: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionInfoEntry {
    #[serde(rename = "type")]
    pub entry_type: String,
    pub id: String,
    #[serde(rename = "parentId")]
    pub parent_id: Option<String>,
    pub timestamp: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomMessageEntry {
    #[serde(rename = "type")]
    pub entry_type: String,
    #[serde(rename = "customType")]
    pub custom_type: String,
    pub id: String,
    #[serde(rename = "parentId")]
    pub parent_id: Option<String>,
    pub timestamp: String,
    pub content: Value, // string or array of TextContent/ImageContent
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<Value>,
    pub display: bool,
}

/// Unified session entry enum — carries all variants.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SessionEntry {
    Message(SessionMessageEntry),
    ThinkingLevelChange(ThinkingLevelChangeEntry),
    ModelChange(ModelChangeEntry),
    Compaction(CompactionEntry),
    BranchSummary(BranchSummaryEntry),
    Custom(CustomEntry),
    CustomMessage(CustomMessageEntry),
    Label(LabelEntry),
    SessionInfo(SessionInfoEntry),
}

impl SessionEntry {
    pub fn id(&self) -> &str {
        match self {
            SessionEntry::Message(e) => &e.id,
            SessionEntry::ThinkingLevelChange(e) => &e.id,
            SessionEntry::ModelChange(e) => &e.id,
            SessionEntry::Compaction(e) => &e.id,
            SessionEntry::BranchSummary(e) => &e.id,
            SessionEntry::Custom(e) => &e.id,
            SessionEntry::CustomMessage(e) => &e.id,
            SessionEntry::Label(e) => &e.id,
            SessionEntry::SessionInfo(e) => &e.id,
        }
    }

    pub fn parent_id(&self) -> Option<&str> {
        match self {
            SessionEntry::Message(e) => e.parent_id.as_deref(),
            SessionEntry::ThinkingLevelChange(e) => e.parent_id.as_deref(),
            SessionEntry::ModelChange(e) => e.parent_id.as_deref(),
            SessionEntry::Compaction(e) => e.parent_id.as_deref(),
            SessionEntry::BranchSummary(e) => e.parent_id.as_deref(),
            SessionEntry::Custom(e) => e.parent_id.as_deref(),
            SessionEntry::CustomMessage(e) => e.parent_id.as_deref(),
            SessionEntry::Label(e) => e.parent_id.as_deref(),
            SessionEntry::SessionInfo(e) => e.parent_id.as_deref(),
        }
    }

    pub fn timestamp(&self) -> &str {
        match self {
            SessionEntry::Message(e) => &e.timestamp,
            SessionEntry::ThinkingLevelChange(e) => &e.timestamp,
            SessionEntry::ModelChange(e) => &e.timestamp,
            SessionEntry::Compaction(e) => &e.timestamp,
            SessionEntry::BranchSummary(e) => &e.timestamp,
            SessionEntry::Custom(e) => &e.timestamp,
            SessionEntry::CustomMessage(e) => &e.timestamp,
            SessionEntry::Label(e) => &e.timestamp,
            SessionEntry::SessionInfo(e) => &e.timestamp,
        }
    }

    /// Returns true if this is a `message` entry.
    pub fn is_message(&self) -> bool {
        matches!(self, SessionEntry::Message(_))
    }

    /// Returns the `role` field from a message entry's JSON payload, or None.
    pub fn message_role(&self) -> Option<&str> {
        if let SessionEntry::Message(e) = self {
            e.message["role"].as_str()
        } else {
            None
        }
    }

    /// Returns the `stopReason` field from a message entry's JSON payload, or None.
    pub fn message_stop_reason(&self) -> Option<&str> {
        if let SessionEntry::Message(e) = self {
            e.message["stopReason"].as_str()
        } else {
            None
        }
    }

    /// Returns true if the message content contains any text blocks.
    pub fn message_has_text_content(&self) -> bool {
        if let SessionEntry::Message(e) = self {
            let content = &e.message["content"];
            if let Some(text) = content.as_str() {
                return !text.is_empty();
            }
            if let Some(arr) = content.as_array() {
                return arr.iter().any(|block| {
                    block["type"].as_str() == Some("text")
                        && !block["text"].as_str().unwrap_or("").is_empty()
                });
            }
        }
        false
    }

    /// Returns text content from a message entry (for search), or None.
    pub fn message_text_content(&self) -> Option<String> {
        if let SessionEntry::Message(e) = self {
            let content = &e.message["content"];
            if let Some(text) = content.as_str() {
                if !text.is_empty() {
                    return Some(text.to_string());
                }
            }
            if let Some(arr) = content.as_array() {
                let parts: Vec<&str> = arr
                    .iter()
                    .filter_map(|block| {
                        if block["type"].as_str() == Some("text") {
                            block["text"].as_str()
                        } else {
                            None
                        }
                    })
                    .collect();
                if !parts.is_empty() {
                    return Some(parts.join(" "));
                }
            }
        }
        None
    }

    /// Returns true if this is a "settings-like" entry (not a message).
    pub fn is_settings_entry(&self) -> bool {
        matches!(
            self,
            SessionEntry::Label(_)
                | SessionEntry::Custom(_)
                | SessionEntry::ModelChange(_)
                | SessionEntry::ThinkingLevelChange(_)
                | SessionEntry::SessionInfo(_)
        )
    }
}

/// Raw file entry — header or entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum FileEntry {
    Session(SessionHeader),
    #[serde(other)]
    Unknown,
}

/// Tree node for get_tree() — defensive copy of session structure.
#[derive(Debug, Clone)]
pub struct SessionTreeNode {
    pub entry: SessionEntry,
    pub children: Vec<SessionTreeNode>,
    pub label: Option<String>,
}

/// Resolved session context for LLM consumption.
#[derive(Debug, Clone)]
pub struct SessionContext {
    /// Messages in chronological order (includes compaction summary when present).
    pub messages: Vec<Value>,
    pub thinking_level: String,
    pub model: Option<ModelInfo>,
}

#[derive(Debug, Clone)]
pub struct ModelInfo {
    pub provider: String,
    pub model_id: String,
}

/// Metadata about a session file.
#[derive(Debug, Clone)]
pub struct SessionInfo {
    pub path: PathBuf,
    pub id: String,
    pub cwd: String,
    pub name: Option<String>,
    pub parent_session_path: Option<String>,
    pub created: chrono::DateTime<Utc>,
    pub modified: chrono::DateTime<Utc>,
    pub message_count: usize,
    pub first_message: String,
    pub all_messages_text: String,
}

// ============================================================================
// Helper functions
// ============================================================================

fn now_iso() -> String {
    Utc::now().to_rfc3339()
}

fn generate_id(by_id: &HashMap<String, SessionEntry>) -> String {
    for _ in 0..100 {
        let id = Ulid::new().to_string().to_lowercase()[..8].to_string();
        if !by_id.contains_key(&id) {
            return id;
        }
    }
    Ulid::new().to_string().to_lowercase()
}

// ============================================================================
// Migration
// ============================================================================

/// Migrate v1 → v2: add id/parentId tree structure.
fn migrate_v1_to_v2(entries: &mut Vec<Value>) {
    let mut ids: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut prev_id: Option<String> = None;

    // First pass: assign IDs
    for entry in entries.iter_mut() {
        let entry_type = entry
            .get("type")
            .and_then(|t| t.as_str())
            .unwrap_or("")
            .to_string();

        if entry_type == "session" {
            if let Some(obj) = entry.as_object_mut() {
                obj.insert("version".to_string(), Value::Number(2.into()));
            }
            continue;
        }

        // Generate unique ID
        let id = loop {
            let candidate = Ulid::new().to_string().to_lowercase()[..8].to_string().to_string();
            if !ids.contains(&candidate) {
                break candidate;
            }
        };
        ids.insert(id.clone());

        if let Some(obj) = entry.as_object_mut() {
            obj.insert("id".to_string(), Value::String(id.clone()));
            obj.insert(
                "parentId".to_string(),
                prev_id
                    .as_ref()
                    .map(|p| Value::String(p.clone()))
                    .unwrap_or(Value::Null),
            );
        }
        prev_id = Some(id);
    }
}

/// Migrate v2 → v3: rename hookMessage role to custom.
fn migrate_v2_to_v3(entries: &mut Vec<Value>) {
    for entry in entries.iter_mut() {
        let entry_type = entry
            .get("type")
            .and_then(|t| t.as_str())
            .unwrap_or("")
            .to_string();

        if entry_type == "session" {
            if let Some(obj) = entry.as_object_mut() {
                obj.insert("version".to_string(), Value::Number(3.into()));
            }
            continue;
        }

        if entry_type == "message" {
            if let Some(role) = entry
                .get("message")
                .and_then(|m| m.get("role"))
                .and_then(|r| r.as_str())
            {
                if role == "hookMessage" {
                    if let Some(msg) = entry
                        .get_mut("message")
                        .and_then(|m| m.as_object_mut())
                    {
                        msg.insert("role".to_string(), Value::String("custom".to_string()));
                    }
                }
            }
        }
    }
}

fn migrate_to_current_version(entries: &mut Vec<Value>) -> bool {
    let version = entries
        .iter()
        .find(|e| e.get("type").and_then(|t| t.as_str()) == Some("session"))
        .and_then(|h| h.get("version"))
        .and_then(|v| v.as_u64())
        .unwrap_or(1) as u32;

    if version >= CURRENT_SESSION_VERSION {
        return false;
    }

    if version < 2 {
        migrate_v1_to_v2(entries);
    }
    if version < 3 {
        migrate_v2_to_v3(entries);
    }

    true
}

// ============================================================================
// Parse / Load helpers (exported for tests)
// ============================================================================

/// Parse JSONL content into raw JSON Values.
pub fn parse_session_entries_raw(content: &str) -> Vec<Value> {
    let mut entries = Vec::new();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Ok(v) = serde_json::from_str::<Value>(line) {
            entries.push(v);
        }
    }
    entries
}

/// Parse raw values into typed SessionEntry list (skips header and unknown types).
pub fn values_to_session_entries(values: &[Value]) -> Vec<SessionEntry> {
    let mut entries = Vec::new();
    for v in values {
        let t = v.get("type").and_then(|t| t.as_str()).unwrap_or("");
        if t == "session" {
            continue;
        }
        // Attempt to deserialize as SessionEntry using the flattened JSON.
        // We re-serialize each value and deserialize as untagged.
        if let Ok(entry) = parse_single_entry(v) {
            entries.push(entry);
        }
    }
    entries
}

fn parse_single_entry(v: &Value) -> Result<SessionEntry, serde_json::Error> {
    let t = v.get("type").and_then(|t| t.as_str()).unwrap_or("");
    match t {
        "message" => Ok(SessionEntry::Message(serde_json::from_value(v.clone())?)),
        "thinking_level_change" => Ok(SessionEntry::ThinkingLevelChange(
            serde_json::from_value(v.clone())?,
        )),
        "model_change" => Ok(SessionEntry::ModelChange(serde_json::from_value(
            v.clone(),
        )?)),
        "compaction" => Ok(SessionEntry::Compaction(serde_json::from_value(
            v.clone(),
        )?)),
        "branch_summary" => Ok(SessionEntry::BranchSummary(serde_json::from_value(
            v.clone(),
        )?)),
        "custom" => Ok(SessionEntry::Custom(serde_json::from_value(v.clone())?)),
        "custom_message" => Ok(SessionEntry::CustomMessage(serde_json::from_value(
            v.clone(),
        )?)),
        "label" => Ok(SessionEntry::Label(serde_json::from_value(v.clone())?)),
        "session_info" => Ok(SessionEntry::SessionInfo(serde_json::from_value(
            v.clone(),
        )?)),
        _ => Err(serde_json::from_str::<SessionEntry>("null").unwrap_err()),
    }
}

/// Load entries from a JSONL session file (sync).
pub fn load_entries_from_file(file_path: &Path) -> Vec<Value> {
    if !file_path.exists() {
        return Vec::new();
    }

    let content = match std::fs::read_to_string(file_path) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };

    let mut entries = parse_session_entries_raw(&content);

    if entries.is_empty() {
        return entries;
    }

    // Validate session header
    let first_type = entries[0]
        .get("type")
        .and_then(|t| t.as_str())
        .unwrap_or("");
    let has_id = entries[0].get("id").and_then(|i| i.as_str()).is_some();

    if first_type != "session" || !has_id {
        return Vec::new();
    }

    entries
}

fn is_valid_session_file(file_path: &Path) -> bool {
    use std::io::Read;
    let mut file = match std::fs::File::open(file_path) {
        Ok(f) => f,
        Err(_) => return false,
    };
    let mut buf = [0u8; 512];
    let n = match file.read(&mut buf) {
        Ok(n) => n,
        Err(_) => return false,
    };
    let first_line = String::from_utf8_lossy(&buf[..n]);
    let first_line = first_line.lines().next().unwrap_or("");
    if first_line.is_empty() {
        return false;
    }
    if let Ok(v) = serde_json::from_str::<Value>(first_line) {
        v.get("type").and_then(|t| t.as_str()) == Some("session")
            && v.get("id").and_then(|i| i.as_str()).is_some()
    } else {
        false
    }
}

/// Find the most recently modified valid session file in a directory.
pub fn find_most_recent_session(session_dir: &Path) -> Option<PathBuf> {
    let read_dir = std::fs::read_dir(session_dir).ok()?;
    let mut files: Vec<(PathBuf, std::time::SystemTime)> = read_dir
        .filter_map(|entry| entry.ok())
        .filter(|entry| {
            entry
                .file_name()
                .to_str()
                .map(|n| n.ends_with(".jsonl"))
                .unwrap_or(false)
        })
        .map(|entry| entry.path())
        .filter(|p| is_valid_session_file(p))
        .filter_map(|p| {
            let mtime = std::fs::metadata(&p).ok()?.modified().ok()?;
            Some((p, mtime))
        })
        .collect();

    files.sort_by(|a, b| b.1.cmp(&a.1));
    files.into_iter().next().map(|(p, _)| p)
}

// ============================================================================
// buildSessionContext (free function, exported for tests)
// ============================================================================

/// Build session context from entries, optionally targeting a specific leaf.
/// Returns messages in LLM order, plus resolved thinking_level and model.
pub fn build_session_context(
    entries: &[SessionEntry],
    leaf_id: Option<Option<&str>>,
    by_id_opt: Option<&HashMap<String, SessionEntry>>,
) -> SessionContext {
    let default_map;
    let by_id = if let Some(m) = by_id_opt {
        m
    } else {
        let mut map = HashMap::new();
        for e in entries {
            map.insert(e.id().to_string(), e.clone());
        }
        default_map = map;
        &default_map
    };

    // Determine leaf
    let leaf = match leaf_id {
        Some(None) => {
            // Explicitly null — return empty
            return SessionContext {
                messages: Vec::new(),
                thinking_level: "off".to_string(),
                model: None,
            };
        }
        Some(Some(id)) => {
            if let Some(e) = by_id.get(id) {
                Some(e)
            } else {
                // id not found — fall back to last entry
                entries.last()
            }
        }
        None => entries.last(),
    };

    let leaf = match leaf {
        Some(e) => e,
        None => {
            return SessionContext {
                messages: Vec::new(),
                thinking_level: "off".to_string(),
                model: None,
            }
        }
    };

    // Walk from leaf to root
    let mut path: Vec<&SessionEntry> = Vec::new();
    let mut current_id: Option<String> = Some(leaf.id().to_string());
    while let Some(ref cid) = current_id.clone() {
        if let Some(entry) = by_id.get(cid) {
            path.push(entry);
            current_id = entry.parent_id().map(|s| s.to_string());
        } else {
            break;
        }
    }
    path.reverse();

    // Extract settings and find last compaction
    let mut thinking_level = "off".to_string();
    let mut model: Option<ModelInfo> = None;
    let mut compaction_idx: Option<usize> = None;

    for (i, entry) in path.iter().enumerate() {
        match entry {
            SessionEntry::ThinkingLevelChange(e) => {
                thinking_level = e.thinking_level.clone();
            }
            SessionEntry::ModelChange(e) => {
                model = Some(ModelInfo {
                    provider: e.provider.clone(),
                    model_id: e.model_id.clone(),
                });
            }
            SessionEntry::Message(e) => {
                let role = e.message.get("role").and_then(|r| r.as_str()).unwrap_or("");
                if role == "assistant" {
                    let provider = e
                        .message
                        .get("provider")
                        .and_then(|p| p.as_str())
                        .unwrap_or("")
                        .to_string();
                    let model_id = e
                        .message
                        .get("model")
                        .and_then(|m| m.as_str())
                        .unwrap_or("")
                        .to_string();
                    model = Some(ModelInfo { provider, model_id });
                }
            }
            SessionEntry::Compaction(_) => {
                compaction_idx = Some(i);
            }
            _ => {}
        }
    }

    // Build message list
    let append_message = |entry: &SessionEntry| -> Option<Value> {
        match entry {
            SessionEntry::Message(e) => Some(e.message.clone()),
            SessionEntry::CustomMessage(e) => {
                // Synthesize a user-role custom message
                Some(serde_json::json!({
                    "role": "custom",
                    "customType": e.custom_type,
                    "content": e.content,
                    "display": e.display,
                    "timestamp": e.timestamp
                }))
            }
            SessionEntry::BranchSummary(e) if !e.summary.is_empty() => {
                Some(serde_json::json!({
                    "role": "branchSummary",
                    "summary": e.summary,
                    "fromId": e.from_id,
                    "timestamp": e.timestamp
                }))
            }
            _ => None,
        }
    };

    let mut messages: Vec<Value> = Vec::new();

    if let Some(comp_idx) = compaction_idx {
        let compaction = match &path[comp_idx] {
            SessionEntry::Compaction(e) => e,
            _ => unreachable!(),
        };

        // Emit summary first
        messages.push(serde_json::json!({
            "role": "compactionSummary",
            "summary": compaction.summary,
            "tokensBefore": compaction.tokens_before,
            "timestamp": compaction.timestamp
        }));

        // Emit kept messages (before compaction, starting from firstKeptEntryId)
        let first_kept_id = &compaction.first_kept_entry_id;
        let mut found_first_kept = false;
        for i in 0..comp_idx {
            let entry = path[i];
            if entry.id() == first_kept_id {
                found_first_kept = true;
            }
            if found_first_kept {
                if let Some(msg) = append_message(entry) {
                    messages.push(msg);
                }
            }
        }

        // Emit messages after compaction
        for i in (comp_idx + 1)..path.len() {
            if let Some(msg) = append_message(path[i]) {
                messages.push(msg);
            }
        }
    } else {
        for entry in &path {
            if let Some(msg) = append_message(entry) {
                messages.push(msg);
            }
        }
    }

    SessionContext {
        messages,
        thinking_level,
        model,
    }
}

pub fn get_latest_compaction_entry(entries: &[SessionEntry]) -> Option<&CompactionEntry> {
    for entry in entries.iter().rev() {
        if let SessionEntry::Compaction(e) = entry {
            return Some(e);
        }
    }
    None
}

// ============================================================================
// SessionManager
// ============================================================================

/// Append-only conversation tree backed by a JSONL file.
pub struct SessionManager {
    session_id: String,
    session_file: Option<PathBuf>,
    session_dir: PathBuf,
    cwd: String,
    persist: bool,
    flushed: bool,
    file_values: Vec<Value>, // raw JSON values including header
    by_id: HashMap<String, SessionEntry>,
    labels_by_id: HashMap<String, String>,
    leaf_id: Option<String>,
}

impl SessionManager {
    fn new(
        cwd: String,
        session_dir: PathBuf,
        session_file: Option<PathBuf>,
        persist: bool,
    ) -> Self {
        let mut sm = Self {
            session_id: String::new(),
            session_file: None,
            session_dir: session_dir.clone(),
            cwd,
            persist,
            flushed: false,
            file_values: Vec::new(),
            by_id: HashMap::new(),
            labels_by_id: HashMap::new(),
            leaf_id: None,
        };

        if persist && !session_dir.as_os_str().is_empty() && !session_dir.exists() {
            std::fs::create_dir_all(&session_dir).ok();
        }

        if let Some(path) = session_file {
            sm.set_session_file(&path);
        } else {
            sm.new_session(None);
        }

        sm
    }

    /// Switch to a different session file.
    pub fn set_session_file(&mut self, session_file: &Path) {
        let resolved = session_file
            .canonicalize()
            .unwrap_or_else(|_| session_file.to_path_buf());
        self.session_file = Some(resolved.clone());

        if resolved.exists() {
            let mut values = load_entries_from_file(&resolved);

            if values.is_empty() {
                let explicit = resolved.clone();
                self.new_session(None);
                self.session_file = Some(explicit.clone());
                self.rewrite_file();
                self.flushed = true;
                return;
            }

            let header_id = values
                .iter()
                .find(|e| e.get("type").and_then(|t| t.as_str()) == Some("session"))
                .and_then(|h| h.get("id").and_then(|i| i.as_str()))
                .map(|s| s.to_string())
                .unwrap_or_else(|| Ulid::new().to_string().to_lowercase());

            self.session_id = header_id;

            if migrate_to_current_version(&mut values) {
                self.file_values = values;
                self.rewrite_file();
            } else {
                self.file_values = values;
            }

            self.build_index();
            self.flushed = true;
        } else {
            let explicit = resolved.clone();
            self.new_session(None);
            self.session_file = Some(explicit);
        }
    }

    /// Start a new (empty) session. Returns the session file path if persisting.
    pub fn new_session(&mut self, options: Option<NewSessionOptions>) -> Option<PathBuf> {
        let id = options
            .as_ref()
            .and_then(|o| o.id.clone())
            .unwrap_or_else(|| Ulid::new().to_string().to_lowercase());

        let parent_session = options.and_then(|o| o.parent_session);

        self.session_id = id.clone();
        let timestamp = now_iso();

        let mut header = serde_json::json!({
            "type": "session",
            "version": CURRENT_SESSION_VERSION,
            "id": id,
            "timestamp": timestamp,
            "cwd": self.cwd,
        });

        if let Some(ref ps) = parent_session {
            header
                .as_object_mut()
                .unwrap()
                .insert("parentSession".to_string(), Value::String(ps.clone()));
        }

        self.file_values = vec![header];
        self.by_id.clear();
        self.labels_by_id.clear();
        self.leaf_id = None;
        self.flushed = false;

        if self.persist {
            let file_timestamp = timestamp.replace([':', '.'], "-");
            let file_name = format!("{}_{}.jsonl", file_timestamp, self.session_id);
            self.session_file = Some(self.session_dir.join(file_name));
        }

        self.session_file.clone()
    }

    fn build_index(&mut self) {
        self.by_id.clear();
        self.labels_by_id.clear();
        self.leaf_id = None;

        let entries = values_to_session_entries(&self.file_values);
        for entry in entries {
            let id = entry.id().to_string();
            self.leaf_id = Some(id.clone());

            if let SessionEntry::Label(ref l) = entry {
                if let Some(ref label) = l.label {
                    if !label.is_empty() {
                        self.labels_by_id
                            .insert(l.target_id.clone(), label.clone());
                    } else {
                        self.labels_by_id.remove(&l.target_id);
                    }
                } else {
                    self.labels_by_id.remove(&l.target_id);
                }
            }

            self.by_id.insert(id, entry);
        }
    }

    fn rewrite_file(&self) {
        if !self.persist {
            return;
        }
        if let Some(ref path) = self.session_file {
            let content: String = self
                .file_values
                .iter()
                .map(|v| serde_json::to_string(v).unwrap_or_default())
                .collect::<Vec<_>>()
                .join("\n")
                + "\n";
            std::fs::write(path, &content).ok();
        }
    }

    fn persist_entry(&mut self, entry_value: &Value) {
        if !self.persist {
            return;
        }
        let path = match self.session_file.clone() {
            Some(p) => p,
            None => return,
        };

        let has_assistant = self.file_values.iter().any(|v| {
            v.get("type").and_then(|t| t.as_str()) == Some("message")
                && v.get("message")
                    .and_then(|m| m.get("role"))
                    .and_then(|r| r.as_str())
                    == Some("assistant")
        });

        if !has_assistant {
            self.flushed = false;
            return;
        }

        if !self.flushed {
            // Write all accumulated entries
            let mut content = String::new();
            for v in &self.file_values {
                content.push_str(&serde_json::to_string(v).unwrap_or_default());
                content.push('\n');
            }
            std::fs::write(&path, &content).ok();
            self.flushed = true;
        } else {
            use std::io::Write;
            let mut file = std::fs::OpenOptions::new()
                .append(true)
                .create(true)
                .open(&path)
                .unwrap();
            let line = serde_json::to_string(entry_value).unwrap_or_default();
            writeln!(file, "{}", line).ok();
        }
    }

    fn append_entry(&mut self, entry: SessionEntry, raw_value: Value) {
        let id = entry.id().to_string();
        self.file_values.push(raw_value.clone());
        self.by_id.insert(id.clone(), entry);
        self.leaf_id = Some(id);
        self.persist_entry(&raw_value);
    }

    // =========================================================================
    // Append methods
    // =========================================================================

    /// Append a message as child of current leaf. Returns entry id.
    pub fn append_message(&mut self, message: Value) -> String {
        let id = generate_id(&self.by_id);
        let value = serde_json::json!({
            "type": "message",
            "id": id,
            "parentId": self.leaf_id,
            "timestamp": now_iso(),
            "message": message
        });
        let entry = SessionEntry::Message(serde_json::from_value(value.clone()).unwrap());
        self.append_entry(entry, value);
        id
    }

    /// Append a thinking level change. Returns entry id.
    pub fn append_thinking_level_change(&mut self, thinking_level: &str) -> String {
        let id = generate_id(&self.by_id);
        let value = serde_json::json!({
            "type": "thinking_level_change",
            "id": id,
            "parentId": self.leaf_id,
            "timestamp": now_iso(),
            "thinkingLevel": thinking_level
        });
        let entry =
            SessionEntry::ThinkingLevelChange(serde_json::from_value(value.clone()).unwrap());
        self.append_entry(entry, value);
        id
    }

    /// Append a model change. Returns entry id.
    pub fn append_model_change(&mut self, provider: &str, model_id: &str) -> String {
        let id = generate_id(&self.by_id);
        let value = serde_json::json!({
            "type": "model_change",
            "id": id,
            "parentId": self.leaf_id,
            "timestamp": now_iso(),
            "provider": provider,
            "modelId": model_id
        });
        let entry = SessionEntry::ModelChange(serde_json::from_value(value.clone()).unwrap());
        self.append_entry(entry, value);
        id
    }

    /// Append a compaction entry. Returns entry id.
    pub fn append_compaction(
        &mut self,
        summary: &str,
        first_kept_entry_id: &str,
        tokens_before: u64,
        details: Option<Value>,
        from_hook: Option<bool>,
    ) -> String {
        let id = generate_id(&self.by_id);
        let mut value = serde_json::json!({
            "type": "compaction",
            "id": id,
            "parentId": self.leaf_id,
            "timestamp": now_iso(),
            "summary": summary,
            "firstKeptEntryId": first_kept_entry_id,
            "tokensBefore": tokens_before
        });

        if let Some(d) = details {
            value.as_object_mut().unwrap().insert("details".to_string(), d);
        }
        if let Some(fh) = from_hook {
            value
                .as_object_mut()
                .unwrap()
                .insert("fromHook".to_string(), Value::Bool(fh));
        }

        let entry = SessionEntry::Compaction(serde_json::from_value(value.clone()).unwrap());
        self.append_entry(entry, value);
        id
    }

    /// Append a custom entry. Returns entry id.
    pub fn append_custom_entry(&mut self, custom_type: &str, data: Option<Value>) -> String {
        let id = generate_id(&self.by_id);
        let mut value = serde_json::json!({
            "type": "custom",
            "customType": custom_type,
            "id": id,
            "parentId": self.leaf_id,
            "timestamp": now_iso()
        });
        if let Some(d) = data {
            value.as_object_mut().unwrap().insert("data".to_string(), d);
        }
        let entry = SessionEntry::Custom(serde_json::from_value(value.clone()).unwrap());
        self.append_entry(entry, value);
        id
    }

    /// Append a session info entry. Returns entry id.
    pub fn append_session_info(&mut self, name: &str) -> String {
        let id = generate_id(&self.by_id);
        let value = serde_json::json!({
            "type": "session_info",
            "id": id,
            "parentId": self.leaf_id,
            "timestamp": now_iso(),
            "name": name.trim()
        });
        let entry = SessionEntry::SessionInfo(serde_json::from_value(value.clone()).unwrap());
        self.append_entry(entry, value);
        id
    }

    /// Append a custom message entry. Returns entry id.
    pub fn append_custom_message_entry(
        &mut self,
        custom_type: &str,
        content: Value,
        display: bool,
        details: Option<Value>,
    ) -> String {
        let id = generate_id(&self.by_id);
        let mut value = serde_json::json!({
            "type": "custom_message",
            "customType": custom_type,
            "content": content,
            "display": display,
            "id": id,
            "parentId": self.leaf_id,
            "timestamp": now_iso()
        });
        if let Some(d) = details {
            value
                .as_object_mut()
                .unwrap()
                .insert("details".to_string(), d);
        }
        let entry = SessionEntry::CustomMessage(serde_json::from_value(value.clone()).unwrap());
        self.append_entry(entry, value);
        id
    }

    // =========================================================================
    // Accessors
    // =========================================================================

    pub fn is_persisted(&self) -> bool {
        self.persist
    }

    pub fn get_cwd(&self) -> &str {
        &self.cwd
    }

    pub fn get_session_dir(&self) -> &Path {
        &self.session_dir
    }

    pub fn get_session_id(&self) -> &str {
        &self.session_id
    }

    pub fn get_session_file(&self) -> Option<&Path> {
        self.session_file.as_deref()
    }

    pub fn get_leaf_id(&self) -> Option<&str> {
        self.leaf_id.as_deref()
    }

    pub fn get_leaf_entry(&self) -> Option<&SessionEntry> {
        self.leaf_id.as_ref().and_then(|id| self.by_id.get(id))
    }

    pub fn get_entry(&self, id: &str) -> Option<&SessionEntry> {
        self.by_id.get(id)
    }

    pub fn get_children(&self, parent_id: &str) -> Vec<&SessionEntry> {
        self.by_id
            .values()
            .filter(|e| e.parent_id() == Some(parent_id))
            .collect()
    }

    pub fn get_label(&self, id: &str) -> Option<&str> {
        self.labels_by_id.get(id).map(|s| s.as_str())
    }

    /// Append a label change. Returns entry id.
    pub fn append_label_change(&mut self, target_id: &str, label: Option<&str>) -> anyhow::Result<String> {
        if !self.by_id.contains_key(target_id) {
            anyhow::bail!("Entry {} not found", target_id);
        }
        let id = generate_id(&self.by_id);
        let value = serde_json::json!({
            "type": "label",
            "id": id,
            "parentId": self.leaf_id,
            "timestamp": now_iso(),
            "targetId": target_id,
            "label": label
        });
        let entry = SessionEntry::Label(serde_json::from_value(value.clone()).unwrap());

        if let Some(l) = label.filter(|l| !l.is_empty()) {
            self.labels_by_id
                .insert(target_id.to_string(), l.to_string());
        } else {
            self.labels_by_id.remove(target_id);
        }

        self.append_entry(entry, value);
        Ok(id)
    }

    /// Get the session name from the latest session_info entry.
    pub fn get_session_name(&self) -> Option<String> {
        let entries = self.get_entries();
        for entry in entries.iter().rev() {
            if let SessionEntry::SessionInfo(e) = entry {
                return e.name.as_ref().and_then(|n| {
                    let trimmed = n.trim().to_string();
                    if trimmed.is_empty() {
                        None
                    } else {
                        Some(trimmed)
                    }
                });
            }
        }
        None
    }

    // =========================================================================
    // Tree Traversal
    // =========================================================================

    /// Walk from an entry to root, returning entries in path order (root-first).
    pub fn get_branch(&self, from_id: Option<&str>) -> Vec<&SessionEntry> {
        let start_id = from_id.or(self.leaf_id.as_deref());
        let mut path: Vec<&SessionEntry> = Vec::new();
        let mut current_id: Option<String> = start_id.map(|s| s.to_string());

        while let Some(ref cid) = current_id.clone() {
            if let Some(entry) = self.by_id.get(cid) {
                path.push(entry);
                current_id = entry.parent_id().map(|s| s.to_string());
            } else {
                break;
            }
        }
        path.reverse();
        path
    }

    /// Build session context (what the LLM sees).
    pub fn build_session_context(&self) -> SessionContext {
        let entries = self.get_entries();
        let entries_vec: Vec<SessionEntry> = entries.into_iter().cloned().collect();
        build_session_context(
            &entries_vec,
            self.leaf_id.as_deref().map(Some),
            Some(&self.by_id),
        )
    }

    /// Get the session header.
    pub fn get_header(&self) -> Option<Value> {
        self.file_values
            .iter()
            .find(|v| v.get("type").and_then(|t| t.as_str()) == Some("session"))
            .cloned()
    }

    /// Get all session entries (excludes header). Returns references.
    pub fn get_entries(&self) -> Vec<&SessionEntry> {
        self.by_id.values().collect()
    }

    /// Get ordered entries (insertion order via file_values).
    pub fn get_entries_ordered(&self) -> Vec<SessionEntry> {
        values_to_session_entries(&self.file_values)
    }

    /// Get session as tree structure.
    pub fn get_tree(&self) -> Vec<SessionTreeNode> {
        let entries = self.get_entries_ordered();
        let mut node_map: HashMap<String, SessionTreeNode> = HashMap::new();
        let mut roots: Vec<String> = Vec::new();
        let mut child_map: HashMap<String, Vec<String>> = HashMap::new();

        // Create nodes
        for entry in &entries {
            let id = entry.id().to_string();
            let label = self.labels_by_id.get(&id).cloned();
            node_map.insert(
                id.clone(),
                SessionTreeNode {
                    entry: entry.clone(),
                    children: Vec::new(),
                    label,
                },
            );
        }

        // Build parent→children map
        for entry in &entries {
            let id = entry.id().to_string();
            match entry.parent_id() {
                None => {
                    roots.push(id);
                }
                Some(pid) if pid == entry.id() => {
                    roots.push(id);
                }
                Some(pid) => {
                    if node_map.contains_key(pid) {
                        child_map.entry(pid.to_string()).or_default().push(id);
                    } else {
                        // Orphan
                        roots.push(id);
                    }
                }
            }
        }

        // Sort children by timestamp
        fn build_node(
            id: &str,
            node_map: &mut HashMap<String, SessionTreeNode>,
            child_map: &HashMap<String, Vec<String>>,
        ) -> SessionTreeNode {
            let mut node = node_map.remove(id).unwrap();
            if let Some(children_ids) = child_map.get(id) {
                let mut children: Vec<SessionTreeNode> = children_ids
                    .iter()
                    .map(|cid| build_node(cid, node_map, child_map))
                    .collect();
                children.sort_by(|a, b| a.entry.timestamp().cmp(b.entry.timestamp()));
                node.children = children;
            }
            node
        }

        let mut result: Vec<SessionTreeNode> = roots
            .iter()
            .filter_map(|id| {
                if node_map.contains_key(id) {
                    Some(build_node(id, &mut node_map, &child_map))
                } else {
                    None
                }
            })
            .collect();

        // Sort roots by timestamp too
        result.sort_by(|a, b| a.entry.timestamp().cmp(b.entry.timestamp()));
        result
    }

    // =========================================================================
    // Branching
    // =========================================================================

    /// Move leaf pointer to the specified entry.
    pub fn branch(&mut self, branch_from_id: &str) -> anyhow::Result<()> {
        if !self.by_id.contains_key(branch_from_id) {
            anyhow::bail!("Entry {} not found", branch_from_id);
        }
        self.leaf_id = Some(branch_from_id.to_string());
        Ok(())
    }

    /// Reset leaf pointer to None (before any entries).
    pub fn reset_leaf(&mut self) {
        self.leaf_id = None;
    }

    /// Branch with a summary of the abandoned path.
    pub fn branch_with_summary(
        &mut self,
        branch_from_id: Option<&str>,
        summary: &str,
        details: Option<Value>,
        from_hook: Option<bool>,
    ) -> anyhow::Result<String> {
        if let Some(id) = branch_from_id {
            if !self.by_id.contains_key(id) {
                anyhow::bail!("Entry {} not found", id);
            }
        }
        self.leaf_id = branch_from_id.map(|s| s.to_string());

        let id = generate_id(&self.by_id);
        let mut value = serde_json::json!({
            "type": "branch_summary",
            "id": id,
            "parentId": self.leaf_id,
            "timestamp": now_iso(),
            "fromId": branch_from_id.unwrap_or("root"),
            "summary": summary
        });

        if let Some(d) = details {
            value.as_object_mut().unwrap().insert("details".to_string(), d);
        }
        if let Some(fh) = from_hook {
            value
                .as_object_mut()
                .unwrap()
                .insert("fromHook".to_string(), Value::Bool(fh));
        }

        let entry = SessionEntry::BranchSummary(serde_json::from_value(value.clone()).unwrap());
        self.append_entry(entry, value);
        Ok(id)
    }

    /// Create a new session containing only the path to a specific leaf.
    pub fn create_branched_session(&mut self, leaf_id: &str) -> anyhow::Result<Option<PathBuf>> {
        let path_entries: Vec<SessionEntry> = self
            .get_branch(Some(leaf_id))
            .into_iter()
            .cloned()
            .collect();

        if path_entries.is_empty() {
            anyhow::bail!("Entry {} not found", leaf_id);
        }

        let path_without_labels: Vec<SessionEntry> = path_entries
            .into_iter()
            .filter(|e| !matches!(e, SessionEntry::Label(_)))
            .collect();

        let new_session_id = Ulid::new().to_string().to_lowercase();
        let timestamp = now_iso();
        let file_timestamp = timestamp.replace([':', '.'], "-");
        let new_session_file = self
            .session_dir
            .join(format!("{}_{}.jsonl", file_timestamp, new_session_id));

        let parent_session = if self.persist {
            self.session_file
                .as_ref()
                .map(|p| p.to_string_lossy().to_string())
        } else {
            None
        };

        let mut new_header = serde_json::json!({
            "type": "session",
            "version": CURRENT_SESSION_VERSION,
            "id": new_session_id,
            "timestamp": timestamp,
            "cwd": self.cwd
        });

        if let Some(ref ps) = parent_session {
            new_header
                .as_object_mut()
                .unwrap()
                .insert("parentSession".to_string(), Value::String(ps.clone()));
        }

        // Collect labels for entries in the path
        let path_entry_ids: std::collections::HashSet<String> =
            path_without_labels.iter().map(|e| e.id().to_string()).collect();

        let labels_to_write: Vec<(String, String)> = self
            .labels_by_id
            .iter()
            .filter(|(tid, _)| path_entry_ids.contains(*tid))
            .map(|(tid, label)| (tid.clone(), label.clone()))
            .collect();

        let mut path_values: Vec<Value> = std::iter::once(new_header)
            .chain(
                path_without_labels
                    .iter()
                    .filter_map(|e| serde_json::to_value(e).ok()),
            )
            .collect();

        // Build label entries
        let mut all_entry_ids = path_entry_ids.clone();
        let last_entry_id = path_without_labels.last().map(|e| e.id().to_string());
        let mut parent_id = last_entry_id;

        let mut label_entries: Vec<Value> = Vec::new();
        for (target_id, label) in &labels_to_write {
            // Generate a unique id not in the set
            let label_id = loop {
                let candidate = Ulid::new().to_string().to_lowercase()[..8].to_string().to_string();
                if !all_entry_ids.contains(&candidate) {
                    break candidate;
                }
            };
            all_entry_ids.insert(label_id.clone());

            let label_value = serde_json::json!({
                "type": "label",
                "id": label_id,
                "parentId": parent_id,
                "timestamp": now_iso(),
                "targetId": target_id,
                "label": label
            });
            parent_id = Some(label_id);
            label_entries.push(label_value);
        }

        path_values.extend(label_entries.iter().cloned());

        if self.persist {
            self.file_values = path_values;
            self.session_id = new_session_id;
            self.session_file = Some(new_session_file.clone());
            self.build_index();

            let has_assistant = self.file_values.iter().any(|v| {
                v.get("type").and_then(|t| t.as_str()) == Some("message")
                    && v.get("message")
                        .and_then(|m| m.get("role"))
                        .and_then(|r| r.as_str())
                        == Some("assistant")
            });

            if has_assistant {
                self.rewrite_file();
                self.flushed = true;
            } else {
                self.flushed = false;
            }

            Ok(Some(new_session_file))
        } else {
            // In-memory mode
            self.file_values = path_values;
            self.session_id = new_session_id;
            self.build_index();
            Ok(None)
        }
    }

    // =========================================================================
    // Static constructors
    // =========================================================================

    /// Create a new session in the given directory.
    pub fn create(cwd: &str, session_dir: Option<&Path>) -> Self {
        let dir = session_dir.map(|p| p.to_path_buf()).unwrap_or_else(|| {
            get_default_session_dir(cwd, None)
        });
        Self::new(cwd.to_string(), dir, None, true)
    }

    /// Open a specific session file.
    pub fn open(path: &Path, session_dir: Option<&Path>) -> Self {
        let entries = load_entries_from_file(path);
        let cwd = entries
            .iter()
            .find(|e| e.get("type").and_then(|t| t.as_str()) == Some("session"))
            .and_then(|h| h.get("cwd").and_then(|c| c.as_str()))
            .unwrap_or(".")
            .to_string();

        let dir = session_dir
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| path.parent().unwrap_or(Path::new(".")).to_path_buf());

        Self::new(cwd, dir, Some(path.to_path_buf()), true)
    }

    /// Continue the most recent session, or create a new one.
    pub fn continue_recent(cwd: &str, session_dir: Option<&Path>) -> Self {
        let dir = session_dir.map(|p| p.to_path_buf()).unwrap_or_else(|| {
            get_default_session_dir(cwd, None)
        });
        if let Some(recent) = find_most_recent_session(&dir) {
            Self::new(cwd.to_string(), dir, Some(recent), true)
        } else {
            Self::new(cwd.to_string(), dir, None, true)
        }
    }

    /// Create an in-memory session (no file persistence).
    pub fn in_memory(cwd: Option<&str>) -> Self {
        let cwd = cwd.unwrap_or(".").to_string();
        Self::new(cwd, PathBuf::new(), None, false)
    }

    /// Fork a session from a source file into a new target cwd.
    pub fn fork_from(
        source_path: &Path,
        target_cwd: &str,
        session_dir: Option<&Path>,
    ) -> anyhow::Result<Self> {
        let source_entries = load_entries_from_file(source_path);
        if source_entries.is_empty() {
            anyhow::bail!(
                "Cannot fork: source session file is empty or invalid: {}",
                source_path.display()
            );
        }

        let has_header = source_entries
            .iter()
            .any(|e| e.get("type").and_then(|t| t.as_str()) == Some("session"));
        if !has_header {
            anyhow::bail!(
                "Cannot fork: source session has no header: {}",
                source_path.display()
            );
        }

        let dir = session_dir.map(|p| p.to_path_buf()).unwrap_or_else(|| {
            get_default_session_dir(target_cwd, None)
        });
        std::fs::create_dir_all(&dir).ok();

        let new_session_id = Ulid::new().to_string().to_lowercase();
        let timestamp = now_iso();
        let file_timestamp = timestamp.replace([':', '.'], "-");
        let new_session_file = dir.join(format!("{}_{}.jsonl", file_timestamp, new_session_id));

        let new_header = serde_json::json!({
            "type": "session",
            "version": CURRENT_SESSION_VERSION,
            "id": new_session_id,
            "timestamp": timestamp,
            "cwd": target_cwd,
            "parentSession": source_path.to_string_lossy()
        });

        use std::io::Write;
        let mut file = std::fs::File::create(&new_session_file)?;
        writeln!(file, "{}", serde_json::to_string(&new_header)?)?;
        for entry in &source_entries {
            if entry.get("type").and_then(|t| t.as_str()) != Some("session") {
                writeln!(file, "{}", serde_json::to_string(entry)?)?;
            }
        }

        Ok(Self::new(
            target_cwd.to_string(),
            dir,
            Some(new_session_file),
            true,
        ))
    }

    /// List sessions in a directory.
    pub async fn list(cwd: &str, session_dir: Option<&Path>) -> Vec<SessionInfo> {
        let dir = session_dir.map(|p| p.to_path_buf()).unwrap_or_else(|| {
            get_default_session_dir(cwd, None)
        });
        let mut sessions = list_sessions_from_dir(&dir).await;
        sessions.sort_by(|a, b| b.modified.cmp(&a.modified));
        sessions
    }
}

// ============================================================================
// Default session directory
// ============================================================================

pub fn get_default_session_dir(cwd: &str, agent_dir: Option<&Path>) -> PathBuf {
    let home = std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."));
    let default_agent = home.join(".pi").join("agent");
    let agent_dir = agent_dir.unwrap_or(&default_agent);

    let safe_path = format!(
        "--{}--",
        cwd.trim_start_matches(['/', '\\'])
            .replace(['/', '\\', ':'], "-")
    );
    let session_dir = agent_dir.join("sessions").join(safe_path);
    std::fs::create_dir_all(&session_dir).ok();
    session_dir
}

// ============================================================================
// Session listing helpers
// ============================================================================

async fn build_session_info(file_path: &Path) -> Option<SessionInfo> {
    let content = tokio::fs::read_to_string(file_path).await.ok()?;
    let mut values: Vec<Value> = Vec::new();

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Ok(v) = serde_json::from_str::<Value>(line) {
            values.push(v);
        }
    }

    if values.is_empty() {
        return None;
    }

    let header = values
        .iter()
        .find(|v| v.get("type").and_then(|t| t.as_str()) == Some("session"))?;

    let stats = tokio::fs::metadata(file_path).await.ok()?;
    let mtime: chrono::DateTime<Utc> = stats
        .modified()
        .ok()
        .map(|t| chrono::DateTime::<Utc>::from(t))
        .unwrap_or_else(Utc::now);

    let mut message_count = 0usize;
    let mut first_message = String::new();
    let mut all_messages: Vec<String> = Vec::new();
    let mut name: Option<String> = None;

    for v in &values {
        let t = v.get("type").and_then(|t| t.as_str()).unwrap_or("");
        if t == "session_info" {
            name = v
                .get("name")
                .and_then(|n| n.as_str())
                .map(|n| n.trim().to_string())
                .filter(|n| !n.is_empty());
        }
        if t != "message" {
            continue;
        }
        message_count += 1;

        let msg = match v.get("message") {
            Some(m) => m,
            None => continue,
        };
        let role = msg.get("role").and_then(|r| r.as_str()).unwrap_or("");
        if role != "user" && role != "assistant" {
            continue;
        }

        let text = match msg.get("content") {
            Some(Value::String(s)) => s.clone(),
            Some(Value::Array(arr)) => arr
                .iter()
                .filter_map(|c| {
                    if c.get("type")?.as_str()? == "text" {
                        c.get("text")?.as_str().map(|s| s.to_string())
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>()
                .join(" "),
            _ => continue,
        };

        if !text.is_empty() {
            all_messages.push(text.clone());
            if first_message.is_empty() && role == "user" {
                first_message = text;
            }
        }
    }

    let cwd = header
        .get("cwd")
        .and_then(|c| c.as_str())
        .unwrap_or("")
        .to_string();
    let parent_session_path = header
        .get("parentSession")
        .and_then(|p| p.as_str())
        .map(|s| s.to_string());
    let id = header.get("id").and_then(|i| i.as_str())?.to_string();

    let created = header
        .get("timestamp")
        .and_then(|t| t.as_str())
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|dt| dt.with_timezone(&Utc))
        .unwrap_or_else(Utc::now);

    Some(SessionInfo {
        path: file_path.to_path_buf(),
        id,
        cwd,
        name,
        parent_session_path,
        created,
        modified: mtime,
        message_count,
        first_message: if first_message.is_empty() {
            "(no messages)".to_string()
        } else {
            first_message
        },
        all_messages_text: all_messages.join(" "),
    })
}

async fn list_sessions_from_dir(dir: &Path) -> Vec<SessionInfo> {
    if !dir.exists() {
        return Vec::new();
    }

    let mut read_dir = match tokio::fs::read_dir(dir).await {
        Ok(rd) => rd,
        Err(_) => return Vec::new(),
    };

    let mut files: Vec<PathBuf> = Vec::new();
    while let Ok(Some(entry)) = read_dir.next_entry().await {
        if entry
            .file_name()
            .to_str()
            .map(|n| n.ends_with(".jsonl"))
            .unwrap_or(false)
        {
            files.push(entry.path());
        }
    }

    let mut sessions: Vec<SessionInfo> = Vec::new();
    for file in files {
        if let Some(info) = build_session_info(&file).await {
            sessions.push(info);
        }
    }
    sessions
}

// ============================================================================
// Exported helpers for tests / compaction
// ============================================================================

/// Parse session entries from JSONL content, apply migrations.
pub fn parse_session_entries(content: &str) -> Vec<Value> {
    let mut entries = parse_session_entries_raw(content);
    migrate_to_current_version(&mut entries);
    entries
}

/// Apply migrations to a set of entries in-place.
pub fn migrate_session_entries(entries: &mut Vec<Value>) {
    migrate_to_current_version(entries);
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;

    fn write_session_file(dir: &Path, name: &str, content: &str) -> PathBuf {
        let path = dir.join(name);
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(content.as_bytes()).unwrap();
        path
    }

    fn valid_header() -> &'static str {
        r#"{"type":"session","id":"abc","timestamp":"2025-01-01T00:00:00Z","cwd":"/tmp"}"#
    }

    fn user_msg_line(id: &str, parent_id: Option<&str>) -> String {
        let parent = parent_id
            .map(|p| format!("\"{}\"", p))
            .unwrap_or("null".to_string());
        format!(
            r#"{{"type":"message","id":"{}","parentId":{},"timestamp":"2025-01-01T00:00:01Z","message":{{"role":"user","content":"hi","timestamp":1}}}}"#,
            id, parent
        )
    }

    // --- load_entries_from_file tests ---

    #[test]
    fn test_load_entries_nonexistent() {
        let dir = TempDir::new().unwrap();
        let entries = load_entries_from_file(&dir.path().join("nonexistent.jsonl"));
        assert!(entries.is_empty());
    }

    #[test]
    fn test_load_entries_empty_file() {
        let dir = TempDir::new().unwrap();
        let path = write_session_file(dir.path(), "empty.jsonl", "");
        assert!(load_entries_from_file(&path).is_empty());
    }

    #[test]
    fn test_load_entries_no_valid_header() {
        let dir = TempDir::new().unwrap();
        let path = write_session_file(
            dir.path(),
            "no-header.jsonl",
            r#"{"type":"message","id":"1"}"#,
        );
        assert!(load_entries_from_file(&path).is_empty());
    }

    #[test]
    fn test_load_entries_malformed_json() {
        let dir = TempDir::new().unwrap();
        let path = write_session_file(dir.path(), "malformed.jsonl", "not json\n");
        assert!(load_entries_from_file(&path).is_empty());
    }

    #[test]
    fn test_load_entries_valid_file() {
        let dir = TempDir::new().unwrap();
        let content = format!("{}\n{}\n", valid_header(), user_msg_line("1", None));
        let path = write_session_file(dir.path(), "valid.jsonl", &content);
        let entries = load_entries_from_file(&path);
        assert_eq!(entries.len(), 2);
        assert_eq!(
            entries[0].get("type").and_then(|t| t.as_str()),
            Some("session")
        );
        assert_eq!(
            entries[1].get("type").and_then(|t| t.as_str()),
            Some("message")
        );
    }

    #[test]
    fn test_load_entries_skips_malformed_lines() {
        let dir = TempDir::new().unwrap();
        let content = format!(
            "{}\nnot valid json\n{}\n",
            valid_header(),
            user_msg_line("1", None)
        );
        let path = write_session_file(dir.path(), "mixed.jsonl", &content);
        let entries = load_entries_from_file(&path);
        assert_eq!(entries.len(), 2); // header + 1 valid message
    }

    // --- find_most_recent_session tests ---

    #[test]
    fn test_find_most_recent_empty_dir() {
        let dir = TempDir::new().unwrap();
        assert!(find_most_recent_session(dir.path()).is_none());
    }

    #[test]
    fn test_find_most_recent_nonexistent_dir() {
        let path = PathBuf::from("/nonexistent-dir-xyz");
        assert!(find_most_recent_session(&path).is_none());
    }

    #[test]
    fn test_find_most_recent_ignores_non_jsonl() {
        let dir = TempDir::new().unwrap();
        write_session_file(dir.path(), "file.txt", "hello");
        write_session_file(dir.path(), "file.json", "{}");
        assert!(find_most_recent_session(dir.path()).is_none());
    }

    #[test]
    fn test_find_most_recent_ignores_invalid_session_header() {
        let dir = TempDir::new().unwrap();
        write_session_file(dir.path(), "invalid.jsonl", r#"{"type":"message"}"#);
        assert!(find_most_recent_session(dir.path()).is_none());
    }

    #[test]
    fn test_find_most_recent_returns_single_valid_file() {
        let dir = TempDir::new().unwrap();
        let path = write_session_file(
            dir.path(),
            "session.jsonl",
            &format!("{}\n", valid_header()),
        );
        assert_eq!(find_most_recent_session(dir.path()), Some(path));
    }

    // --- SessionManager tests ---

    #[test]
    fn test_in_memory_session_basic() {
        let mut sm = SessionManager::in_memory(None);
        assert!(sm.get_leaf_id().is_none());

        let id1 = sm.append_message(serde_json::json!({
            "role": "user",
            "content": "hello",
            "timestamp": 1
        }));
        assert_eq!(sm.get_leaf_id(), Some(id1.as_str()));

        let entries = sm.get_entries();
        assert_eq!(entries.len(), 1);
    }

    #[test]
    fn test_append_creates_parent_chain() {
        let mut sm = SessionManager::in_memory(None);

        let id1 = sm.append_message(serde_json::json!({"role":"user","content":"1","timestamp":1}));
        let id2 = sm.append_message(serde_json::json!({"role":"assistant","content":[{"type":"text","text":"2"}],"timestamp":1}));
        let id3 = sm.append_message(serde_json::json!({"role":"user","content":"3","timestamp":1}));

        let entries = sm.get_entries_ordered();
        assert_eq!(entries.len(), 3);

        // Check parent chain
        assert_eq!(entries[0].id(), id1);
        assert_eq!(entries[0].parent_id(), None);
        assert_eq!(entries[1].id(), id2);
        assert_eq!(entries[1].parent_id(), Some(id1.as_str()));
        assert_eq!(entries[2].id(), id3);
        assert_eq!(entries[2].parent_id(), Some(id2.as_str()));
    }

    #[test]
    fn test_branch_moves_leaf() {
        let mut sm = SessionManager::in_memory(None);

        let id1 = sm.append_message(serde_json::json!({"role":"user","content":"1","timestamp":1}));
        let _id2 = sm.append_message(serde_json::json!({"role":"assistant","content":[{"type":"text","text":"2"}],"timestamp":1}));
        let id3 = sm.append_message(serde_json::json!({"role":"user","content":"3","timestamp":1}));

        assert_eq!(sm.get_leaf_id(), Some(id3.as_str()));

        sm.branch(&id1).unwrap();
        assert_eq!(sm.get_leaf_id(), Some(id1.as_str()));
    }

    #[test]
    fn test_branch_nonexistent_throws() {
        let mut sm = SessionManager::in_memory(None);
        sm.append_message(serde_json::json!({"role":"user","content":"hello","timestamp":1}));
        assert!(sm.branch("nonexistent").is_err());
    }

    #[test]
    fn test_get_branch_returns_path() {
        let mut sm = SessionManager::in_memory(None);

        let id1 = sm.append_message(serde_json::json!({"role":"user","content":"1","timestamp":1}));
        let id2 = sm.append_message(serde_json::json!({"role":"assistant","content":[{"type":"text","text":"2"}],"timestamp":1}));
        let _id3 = sm.append_message(serde_json::json!({"role":"user","content":"3","timestamp":1}));

        let path = sm.get_branch(Some(&id2));
        assert_eq!(path.len(), 2);
        assert_eq!(path[0].id(), id1);
        assert_eq!(path[1].id(), id2);
    }

    #[test]
    fn test_branch_with_summary() {
        let mut sm = SessionManager::in_memory(None);

        let id1 = sm.append_message(serde_json::json!({"role":"user","content":"1","timestamp":1}));
        let _id2 = sm.append_message(serde_json::json!({"role":"assistant","content":[{"type":"text","text":"2"}],"timestamp":1}));
        let _id3 = sm.append_message(serde_json::json!({"role":"user","content":"3","timestamp":1}));

        let summary_id = sm.branch_with_summary(Some(&id1), "Summary of abandoned work", None, None).unwrap();
        assert_eq!(sm.get_leaf_id(), Some(summary_id.as_str()));

        let entries = sm.get_entries_ordered();
        let summary_entry = entries.iter().find(|e| matches!(e, SessionEntry::BranchSummary(_)));
        assert!(summary_entry.is_some());
        assert_eq!(summary_entry.unwrap().parent_id(), Some(id1.as_str()));
    }

    #[test]
    fn test_set_session_file_empty_file_recovery() {
        let dir = TempDir::new().unwrap();
        let empty_file = dir.path().join("empty.jsonl");
        std::fs::write(&empty_file, "").unwrap();

        let sm = SessionManager::open(&empty_file, Some(dir.path()));
        assert!(!sm.get_session_id().is_empty());
        assert!(sm.get_header().is_some());

        // File should now have a valid header
        let content = std::fs::read_to_string(&empty_file).unwrap();
        let lines: Vec<_> = content.lines().filter(|l| !l.is_empty()).collect();
        assert_eq!(lines.len(), 1);
        let header: Value = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(header.get("type").and_then(|t| t.as_str()), Some("session"));
    }

    #[test]
    fn test_create_branched_session_in_memory() {
        let mut sm = SessionManager::in_memory(None);

        let id1 = sm.append_message(serde_json::json!({"role":"user","content":"1","timestamp":1}));
        let id2 = sm.append_message(serde_json::json!({"role":"assistant","content":[{"type":"text","text":"2"}],"timestamp":1}));
        let _id3 = sm.append_message(serde_json::json!({"role":"user","content":"3","timestamp":1}));
        sm.append_message(serde_json::json!({"role":"assistant","content":[{"type":"text","text":"4"}],"timestamp":1}));

        // Branch from id2 (should only have id1, id2)
        let result = sm.create_branched_session(&id2).unwrap();
        assert!(result.is_none()); // in-memory returns None

        let entries = sm.get_entries_ordered();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].id(), id1);
        assert_eq!(entries[1].id(), id2);
    }

    #[test]
    fn test_create_branched_session_nonexistent() {
        let mut sm = SessionManager::in_memory(None);
        sm.append_message(serde_json::json!({"role":"user","content":"hello","timestamp":1}));
        assert!(sm.create_branched_session("nonexistent").is_err());
    }

    // --- build_session_context free function tests ---

    fn make_user_entry(id: &str, parent_id: Option<&str>, text: &str) -> SessionEntry {
        SessionEntry::Message(SessionMessageEntry {
            entry_type: "message".to_string(),
            id: id.to_string(),
            parent_id: parent_id.map(|s| s.to_string()),
            timestamp: "2025-01-01T00:00:00Z".to_string(),
            message: serde_json::json!({
                "role": "user",
                "content": text,
                "timestamp": 1
            }),
        })
    }

    fn make_assistant_entry(id: &str, parent_id: Option<&str>, text: &str) -> SessionEntry {
        SessionEntry::Message(SessionMessageEntry {
            entry_type: "message".to_string(),
            id: id.to_string(),
            parent_id: parent_id.map(|s| s.to_string()),
            timestamp: "2025-01-01T00:00:00Z".to_string(),
            message: serde_json::json!({
                "role": "assistant",
                "content": [{"type": "text", "text": text}],
                "provider": "anthropic",
                "model": "claude-test",
                "timestamp": 1
            }),
        })
    }

    fn make_compaction_entry(
        id: &str,
        parent_id: Option<&str>,
        summary: &str,
        first_kept_entry_id: &str,
    ) -> SessionEntry {
        SessionEntry::Compaction(CompactionEntry {
            entry_type: "compaction".to_string(),
            id: id.to_string(),
            parent_id: parent_id.map(|s| s.to_string()),
            timestamp: "2025-01-01T00:00:00Z".to_string(),
            summary: summary.to_string(),
            first_kept_entry_id: first_kept_entry_id.to_string(),
            tokens_before: 1000,
            details: None,
            from_hook: None,
        })
    }

    #[test]
    fn test_build_context_empty() {
        let ctx = build_session_context(&[], None, None);
        assert!(ctx.messages.is_empty());
        assert_eq!(ctx.thinking_level, "off");
        assert!(ctx.model.is_none());
    }

    #[test]
    fn test_build_context_single_user_message() {
        let entries = vec![make_user_entry("1", None, "hello")];
        let ctx = build_session_context(&entries, None, None);
        assert_eq!(ctx.messages.len(), 1);
        assert_eq!(
            ctx.messages[0].get("role").and_then(|r| r.as_str()),
            Some("user")
        );
    }

    #[test]
    fn test_build_context_simple_conversation() {
        let entries = vec![
            make_user_entry("1", None, "hello"),
            make_assistant_entry("2", Some("1"), "hi there"),
            make_user_entry("3", Some("2"), "how are you"),
            make_assistant_entry("4", Some("3"), "great"),
        ];
        let ctx = build_session_context(&entries, None, None);
        assert_eq!(ctx.messages.len(), 4);
    }

    #[test]
    fn test_build_context_with_compaction() {
        let entries = vec![
            make_user_entry("1", None, "first"),
            make_assistant_entry("2", Some("1"), "response1"),
            make_user_entry("3", Some("2"), "second"),
            make_assistant_entry("4", Some("3"), "response2"),
            make_compaction_entry("5", Some("4"), "Summary of first two turns", "3"),
            make_user_entry("6", Some("5"), "third"),
            make_assistant_entry("7", Some("6"), "response3"),
        ];
        let ctx = build_session_context(&entries, None, None);
        // summary + kept(3,4) + after(6,7) = 5
        assert_eq!(ctx.messages.len(), 5);
        assert_eq!(
            ctx.messages[0].get("role").and_then(|r| r.as_str()),
            Some("compactionSummary")
        );
        assert!(ctx.messages[0]
            .get("summary")
            .and_then(|s| s.as_str())
            .unwrap_or("")
            .contains("Summary of first two turns"));
    }

    #[test]
    fn test_build_context_branches() {
        let entries = vec![
            make_user_entry("1", None, "start"),
            make_assistant_entry("2", Some("1"), "response"),
            make_user_entry("3", Some("2"), "branch A"),
            make_user_entry("4", Some("2"), "branch B"),
        ];

        let ctx_a = build_session_context(&entries, Some(Some("3")), None);
        assert_eq!(ctx_a.messages.len(), 3);
        assert_eq!(
            ctx_a.messages[2].get("content").and_then(|c| c.as_str()),
            Some("branch A")
        );

        let ctx_b = build_session_context(&entries, Some(Some("4")), None);
        assert_eq!(ctx_b.messages.len(), 3);
        assert_eq!(
            ctx_b.messages[2].get("content").and_then(|c| c.as_str()),
            Some("branch B")
        );
    }

    #[test]
    fn test_build_context_null_leaf_returns_empty() {
        let entries = vec![make_user_entry("1", None, "hello")];
        let ctx = build_session_context(&entries, Some(None), None);
        assert!(ctx.messages.is_empty());
    }

    // ── build-context.test.ts (additional cases) ─────────────────────────────

    #[test]
    fn test_build_context_thinking_level_tracked() {
        // ThinkingLevelChange entries update context.thinking_level
        let entries = vec![
            make_user_entry("1", None, "hello"),
            SessionEntry::ThinkingLevelChange(ThinkingLevelChangeEntry {
                entry_type: "thinking_level_change".to_string(),
                id: "2".to_string(),
                parent_id: Some("1".to_string()),
                timestamp: "2025-01-01T00:00:00Z".to_string(),
                thinking_level: "high".to_string(),
            }),
            make_assistant_entry("3", Some("2"), "thinking hard"),
        ];
        let ctx = build_session_context(&entries, None, None);
        assert_eq!(ctx.thinking_level, "high");
        // ThinkingLevelChange entry itself is excluded from messages
        assert_eq!(ctx.messages.len(), 2);
    }

    #[test]
    fn test_build_context_model_from_assistant_message() {
        let entries = vec![
            make_user_entry("1", None, "hello"),
            make_assistant_entry("2", Some("1"), "hi"),
        ];
        let ctx = build_session_context(&entries, None, None);
        assert!(ctx.model.is_some());
        let m = ctx.model.unwrap();
        assert_eq!(m.provider, "anthropic");
        assert_eq!(m.model_id, "claude-test");
    }

    #[test]
    fn test_build_context_multiple_compactions_uses_latest() {
        let entries = vec![
            make_user_entry("1", None, "a"),
            make_assistant_entry("2", Some("1"), "b"),
            make_compaction_entry("3", Some("2"), "First summary", "1"),
            make_user_entry("4", Some("3"), "c"),
            make_assistant_entry("5", Some("4"), "d"),
            make_compaction_entry("6", Some("5"), "Second summary", "4"),
            make_user_entry("7", Some("6"), "e"),
        ];
        let ctx = build_session_context(&entries, None, None);
        // second compaction: summary + kept(4,5) + after(7) = 4
        assert_eq!(ctx.messages.len(), 4);
        assert!(ctx.messages[0]
            .get("summary")
            .and_then(|s| s.as_str())
            .unwrap_or("")
            .contains("Second summary"));
    }

    #[test]
    fn test_build_context_uses_last_entry_when_leaf_not_found() {
        let entries = vec![
            make_user_entry("1", None, "hello"),
            make_assistant_entry("2", Some("1"), "hi"),
        ];
        // nonexistent leaf → fall back to last entry
        let ctx = build_session_context(&entries, Some(Some("nonexistent")), None);
        assert_eq!(ctx.messages.len(), 2);
    }

    // ── save-entry.test.ts ────────────────────────────────────────────────────

    #[test]
    fn test_save_custom_entry_included_in_tree() {
        let mut sm = SessionManager::in_memory(None);

        let msg_id = sm.append_message(serde_json::json!({
            "role": "user",
            "content": "hello",
            "timestamp": 1
        }));

        let custom_id = sm.append_custom_entry("my_data", Some(serde_json::json!({"foo": "bar"})));

        let msg2_id = sm.append_message(serde_json::json!({
            "role": "assistant",
            "content": [{"type": "text", "text": "hi"}],
            "api": "anthropic-messages",
            "provider": "anthropic",
            "model": "test",
            "usage": {"input":1,"output":1,"cacheRead":0,"cacheWrite":0,"totalTokens":2,
                      "cost":{"input":0,"output":0,"cacheRead":0,"cacheWrite":0,"total":0}},
            "stopReason": "stop",
            "timestamp": 2
        }));

        let entries = sm.get_entries();
        assert_eq!(entries.len(), 3);

        let custom_entry = entries.iter().find(|e| matches!(e, SessionEntry::Custom(_)));
        assert!(custom_entry.is_some());
        let SessionEntry::Custom(c) = custom_entry.unwrap() else { panic!() };
        assert_eq!(c.custom_type, "my_data");
        assert_eq!(c.data, Some(serde_json::json!({"foo": "bar"})));
        assert_eq!(c.id, custom_id);
        assert_eq!(c.parent_id.as_deref(), Some(msg_id.as_str()));

        // Branch path: msg → custom → msg2
        let path = sm.get_branch(None);
        assert_eq!(path.len(), 3);
        assert_eq!(path[0].id(), msg_id);
        assert_eq!(path[1].id(), custom_id);
        assert_eq!(path[2].id(), msg2_id);

        // buildSessionContext skips custom entries
        let ctx = sm.build_session_context();
        assert_eq!(ctx.messages.len(), 2);
    }

    // ── labels.test.ts ────────────────────────────────────────────────────────

    #[test]
    fn test_labels_set_and_get() {
        let mut sm = SessionManager::in_memory(None);
        let msg_id = sm.append_message(serde_json::json!({
            "role": "user",
            "content": "hello",
            "timestamp": 1
        }));

        assert!(sm.get_label(&msg_id).is_none());

        let label_id = sm.append_label_change(&msg_id, Some("checkpoint")).unwrap();
        assert_eq!(sm.get_label(&msg_id), Some("checkpoint"));

        // Label entry should be in entries
        let entries = sm.get_entries();
        let label_entry = entries
            .iter()
            .find(|e| matches!(e, SessionEntry::Label(_)));
        assert!(label_entry.is_some());
        let SessionEntry::Label(l) = label_entry.unwrap() else { panic!() };
        assert_eq!(l.id, label_id);
        assert_eq!(l.target_id, msg_id);
        assert_eq!(l.label.as_deref(), Some("checkpoint"));
    }

    #[test]
    fn test_labels_cleared_with_none() {
        let mut sm = SessionManager::in_memory(None);
        let msg_id = sm.append_message(serde_json::json!({
            "role": "user",
            "content": "hello",
            "timestamp": 1
        }));

        sm.append_label_change(&msg_id, Some("checkpoint")).unwrap();
        assert_eq!(sm.get_label(&msg_id), Some("checkpoint"));

        sm.append_label_change(&msg_id, None).unwrap();
        assert!(sm.get_label(&msg_id).is_none());
    }

    #[test]
    fn test_labels_last_wins() {
        let mut sm = SessionManager::in_memory(None);
        let msg_id = sm.append_message(serde_json::json!({
            "role": "user",
            "content": "hello",
            "timestamp": 1
        }));

        sm.append_label_change(&msg_id, Some("first")).unwrap();
        sm.append_label_change(&msg_id, Some("second")).unwrap();
        sm.append_label_change(&msg_id, Some("third")).unwrap();

        assert_eq!(sm.get_label(&msg_id), Some("third"));
    }

    #[test]
    fn test_labels_included_in_tree_nodes() {
        let mut sm = SessionManager::in_memory(None);
        let msg1_id = sm.append_message(serde_json::json!({
            "role": "user",
            "content": "hello",
            "timestamp": 1
        }));
        let msg2_id = sm.append_message(serde_json::json!({
            "role": "assistant",
            "content": [{"type": "text", "text": "hi"}],
            "provider": "anthropic",
            "model": "test",
            "timestamp": 2
        }));

        sm.append_label_change(&msg1_id, Some("start")).unwrap();
        sm.append_label_change(&msg2_id, Some("response")).unwrap();

        let tree = sm.get_tree();

        // Find message nodes (tree root should be msg1)
        let msg1_node = tree.iter().find(|n| n.entry.id() == msg1_id);
        assert!(msg1_node.is_some());
        assert_eq!(msg1_node.unwrap().label.as_deref(), Some("start"));

        // msg2 is a child of msg1 (label entries don't count as message children)
        let msg2_node = msg1_node
            .unwrap()
            .children
            .iter()
            .find(|n| n.entry.id() == msg2_id);
        assert!(msg2_node.is_some());
        assert_eq!(msg2_node.unwrap().label.as_deref(), Some("response"));
    }

    #[test]
    fn test_labels_not_in_build_session_context() {
        let mut sm = SessionManager::in_memory(None);
        let msg_id = sm.append_message(serde_json::json!({
            "role": "user",
            "content": "hello",
            "timestamp": 1
        }));
        sm.append_label_change(&msg_id, Some("checkpoint")).unwrap();

        let ctx = sm.build_session_context();
        assert_eq!(ctx.messages.len(), 1);
        assert_eq!(
            ctx.messages[0].get("role").and_then(|r| r.as_str()),
            Some("user")
        );
    }

    #[test]
    fn test_labels_throws_for_nonexistent_entry() {
        let mut sm = SessionManager::in_memory(None);
        let result = sm.append_label_change("non-existent", Some("label"));
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("non-existent"));
    }

    #[test]
    fn test_labels_preserved_in_create_branched_session() {
        let mut sm = SessionManager::in_memory(None);
        let msg1_id = sm.append_message(serde_json::json!({
            "role": "user",
            "content": "hello",
            "timestamp": 1
        }));
        let msg2_id = sm.append_message(serde_json::json!({
            "role": "assistant",
            "content": [{"type": "text", "text": "hi"}],
            "provider": "anthropic",
            "model": "test",
            "timestamp": 2
        }));

        sm.append_label_change(&msg1_id, Some("important")).unwrap();
        sm.append_label_change(&msg2_id, Some("also-important")).unwrap();

        // Branch from msg2 (in-memory returns None but updates state)
        sm.create_branched_session(&msg2_id).unwrap();

        // Labels should be preserved
        assert_eq!(sm.get_label(&msg1_id), Some("important"));
        assert_eq!(sm.get_label(&msg2_id), Some("also-important"));

        // Two label entries should exist
        let entries = sm.get_entries();
        let label_count = entries.iter().filter(|e| matches!(e, SessionEntry::Label(_))).count();
        assert_eq!(label_count, 2);
    }

    #[test]
    fn test_labels_not_on_path_not_preserved_in_branched_session() {
        let mut sm = SessionManager::in_memory(None);
        let msg1_id = sm.append_message(serde_json::json!({
            "role": "user",
            "content": "hello",
            "timestamp": 1
        }));
        let msg2_id = sm.append_message(serde_json::json!({
            "role": "assistant",
            "content": [{"type": "text", "text": "hi"}],
            "provider": "anthropic",
            "model": "test",
            "timestamp": 2
        }));
        let msg3_id = sm.append_message(serde_json::json!({
            "role": "user",
            "content": "followup",
            "timestamp": 3
        }));

        sm.append_label_change(&msg1_id, Some("first")).unwrap();
        sm.append_label_change(&msg2_id, Some("second")).unwrap();
        sm.append_label_change(&msg3_id, Some("third")).unwrap();

        // Branch from msg2 → excludes msg3
        sm.create_branched_session(&msg2_id).unwrap();

        assert_eq!(sm.get_label(&msg1_id), Some("first"));
        assert_eq!(sm.get_label(&msg2_id), Some("second"));
        // msg3 is no longer on the path
        assert!(sm.get_label(&msg3_id).is_none());
    }
}
