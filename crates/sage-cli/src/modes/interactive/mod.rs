//! Interactive TUI mode for the coding agent.
//!
//! Translated from pi-mono `packages/coding-agent/src/modes/interactive/interactive-mode.ts`.
//!
//! This is the full-screen TUI mode launched when `sage` is invoked without
//! `--print` / `--mode json|rpc`. The implementation here is a structural
//! skeleton; the ratatui rendering details are fleshed out in the `tui` crate.

pub mod approval;
pub mod components;
pub mod theme;

use std::collections::HashMap;
use std::io;
use std::sync::{Arc, Mutex, OnceLock};

use regex::Regex;
use syntect::parsing::{ParseState, ScopeStack, SyntaxReference, SyntaxSet};

use crossterm::{
    event::{
        DisableMouseCapture, EnableMouseCapture, Event, EventStream, KeyCode, KeyModifiers,
        MouseEventKind,
    },
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use futures::StreamExt as _;
use ratatui::{
    Terminal,
    backend::CrosstermBackend,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
};
use tokio::sync::mpsc;

use unicode_width::UnicodeWidthStr as _;

use crate::print_session::AgentDelta;
use crate::core::slash_commands::BUILTIN_SLASH_COMMANDS;
use crate::modes::interactive::approval::{ApprovalRequest, ApprovalResponse};
use crate::modes::interactive::components::diff::render_diff_ratatui;
use crate::modes::interactive::theme::{Theme, ThemeBg, ThemeColor, ThinkingLevel, get_theme};
use crate::utils::clipboard::copy_to_clipboard;
use crate::core::session_manager::{SessionEntry, SessionManager};
use crate::core::settings_manager::SettingsManager;

// ============================================================================
// InteractiveMode
// ============================================================================

/// Options passed to [`InteractiveMode`].
#[derive(Debug, Default)]
pub struct InteractiveModeOptions {
    /// Optional initial message to send on start.
    pub initial_message: Option<String>,
    /// Whether to show verbose startup output.
    pub verbose: bool,
    /// Optional model-fallback warning message.
    pub model_fallback_message: Option<String>,
    /// Optional provider-migration notices.
    pub migrated_providers: Vec<String>,
}

/// State held by the interactive TUI loop.
pub struct InteractiveMode {
    options: InteractiveModeOptions,
    input_buffer: String,
    messages: Vec<ChatMessage>,
    running: bool,
    agent_rx: Option<mpsc::UnboundedReceiver<AgentDelta>>,
    is_thinking: bool,
    provider_id: Option<String>,
    model_id: Option<String>,
    session_input_tokens: u64,
    session_output_tokens: u64,
    session_cache_read_tokens: u64,
    session_cache_write_tokens: u64,
    session_cost_usd: f64,
    /// Current scroll offset in display lines.
    scroll_top: u16,
    /// When true, scroll_top tracks the bottom of content as it grows.
    is_sticky: bool,
    /// Viewport height cached from last render (lines, no border).
    last_viewport_height: u16,
    /// Terminal width cached from last render (columns).
    last_terminal_width: u16,
    /// Tick counter for spinner animation (incremented every ~50 ms).
    tick: u64,
    /// Cached git branch name, refreshed every ~10s (200 ticks).
    git_branch: Option<String>,
    /// Name of the tool currently executing, cleared when done.
    current_tool: Option<String>,
    // ── Approval channel ──────────────────────────────────────────────────
    approval_tx: mpsc::UnboundedSender<ApprovalRequest>,
    approval_rx: mpsc::UnboundedReceiver<ApprovalRequest>,
    pending_approval: Option<ApprovalRequest>,
    /// Per-session tool rules: true=AllowAlways, false=DenyAlways.
    session_rules: Arc<Mutex<HashMap<String, bool>>>,
    // ── Autocomplete (slash commands + @ file) ────────────────────────────
    completion_matches: Vec<(String, String)>, // (primary, hint)
    completion_selected: usize,
    // ── Agent task handle ─────────────────────────────────────────────────
    /// Handle to the running agent task; aborted before spawning a new one.
    agent_handle: Option<tokio::task::JoinHandle<()>>,
    // ── Input history (↑/↓ recall) ────────────────────────────────────────
    /// Sent user messages in chronological order; pushed on every send.
    history: Vec<String>,
    /// Current browse position; `None` means "viewing the current draft".
    history_idx: Option<usize>,
    /// Draft snapshot saved when the user first presses ↑ to browse history.
    history_draft: String,
    /// Current permission mode: "default" | "bypassPermissions" | "plan".
    permission_mode: String,
    /// Thinking/reasoning budget level for the next agent call.
    thinking_level: ThinkingLevel,
    /// Timestamp of session start (for /session duration display).
    session_start: std::time::Instant,
    /// Set to true by /quit; checked after slash dispatch to break the event loop.
    quit_pending: bool,
    // ── Session / Settings ──────────────────────────────────────────────────────
    session_manager: SessionManager,
    settings_manager: SettingsManager,
    session_name: Option<String>,
    /// How many entries in `messages` have already been flushed to the session file.
    /// Compared on each TurnUsage to find newly-completed assistant turns.
    session_assistant_saved_up_to: usize,
    // ── Context window tracking ──────────────────────────────────────────────
    /// Model context window size (tokens); 0 until first TurnUsage.
    context_window: u32,
    /// Full context size from the last API response: input + cache_read + cache_write + output.
    /// Matches CC's `getTokenCountFromUsage()` formula.
    last_turn_context_tokens: u64,
    // ── Ctrl+F search ────────────────────────────────────────────────────────
    search_active: bool,
    search_query: String,
    /// Indices into `messages` that match the current query.
    search_matches: Vec<usize>,
    /// Which match is currently selected (index into search_matches).
    search_idx: usize,
    // ── Settings overlay ─────────────────────────────────────────────────────
    settings_active: bool,
    settings_selected: usize,
    settings_scroll: usize,
    settings_items: Vec<crate::modes::interactive::components::settings_selector::SettingItem>,
    // ── Compact ───────────────────────────────────────────────────────────────
    compact_warned: bool,
    /// LLM-generated summary from the most recent /compact, injected into subsequent turns.
    compact_summary: Option<String>,
    /// True while the compact LLM call is running.
    compacting: bool,
}

/// A single chat turn in the history display.
#[derive(Debug, Clone)]
pub struct ChatMessage {
    pub role: MessageRole,
    pub content: String,
    /// `@path` file references successfully expanded into this message.
    pub at_refs: Vec<AtRef>,
    /// Full untruncated tool output (tool messages only).
    pub full_output: Option<String>,
    /// Whether the full_output is currently expanded (tool messages only).
    pub full_output_expanded: bool,
}

/// An `@path` file reference that was expanded before sending to the LLM.
#[derive(Debug, Clone)]
pub struct AtRef {
    pub path: String,
    pub line_count: usize,
}

/// Speaker for a chat message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MessageRole {
    User,
    Assistant,
    System,
    /// A tool invocation — pending while `pending == true`.
    Tool {
        name: String,
        pending: bool,
        success: bool,
    },
    /// A fatal agent error shown inline.
    Error,
    /// A reasoning/thinking block from extended-thinking models.
    /// Displayed collapsed by default; `t` key toggles `expanded`.
    Thinking {
        duration_ms: u64,
        expanded: bool,
    },
}

/// Session state replayed from a JSONL session file.
struct ReplayedState {
    messages: Vec<ChatMessage>,
    /// Last `thinking_level_change` entry seen, if any.
    thinking_level: Option<ThinkingLevel>,
    /// Last `model_change` model_id seen, if any.
    model_id: Option<String>,
    /// Last `model_change` provider seen, if any.
    provider_id: Option<String>,
    /// Last `session_info` name seen, if any.
    session_name: Option<String>,
}

impl InteractiveMode {
    /// Create a new InteractiveMode with the given options.
    pub fn new(options: InteractiveModeOptions) -> Self {
        let (approval_tx, approval_rx) = mpsc::unbounded_channel();
        let cwd = std::env::current_dir()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|_| ".".to_string());
        let agent_dir = crate::config::get_agent_dir();
        let settings_manager = SettingsManager::create(&cwd, &agent_dir);
        let init_model = settings_manager.get_default_model().map(|s| s.to_string());
        let init_provider = settings_manager.get_default_provider().map(|s| s.to_string());
        let session_manager = SessionManager::create(&cwd, None);
        Self {
            options,
            input_buffer: String::new(),
            messages: Vec::new(),
            running: false,
            agent_rx: None,
            is_thinking: false,
            provider_id: init_provider,
            model_id: init_model,
            session_input_tokens: 0,
            session_output_tokens: 0,
            session_cache_read_tokens: 0,
            session_cache_write_tokens: 0,
            session_cost_usd: 0.0,
            scroll_top: 0,
            is_sticky: true,
            last_viewport_height: 0,
            last_terminal_width: 80,
            tick: 0,
            git_branch: Self::read_git_branch(),
            current_tool: None,
            approval_tx,
            approval_rx,
            pending_approval: None,
            session_rules: Arc::new(Mutex::new(HashMap::new())),
            completion_matches: Vec::new(),
            completion_selected: 0,
            agent_handle: None,
            history: Vec::new(),
            history_idx: None,
            history_draft: String::new(),
            permission_mode: "default".to_string(),
            thinking_level: ThinkingLevel::Off,
            session_start: std::time::Instant::now(),
            quit_pending: false,
            session_manager,
            settings_manager,
            session_name: None,
            session_assistant_saved_up_to: 0,
            context_window: 0,
            last_turn_context_tokens: 0,
            search_active: false,
            search_query: String::new(),
            search_matches: Vec::new(),
            search_idx: 0,
            settings_active: false,
            settings_selected: 0,
            settings_scroll: 0,
            settings_items: Vec::new(),
            compact_warned: false,
            compact_summary: None,
            compacting: false,
        }
    }

    pub fn set_provider(&mut self, provider: Option<String>) {
        self.provider_id = provider;
    }

    pub fn set_model(&mut self, model: Option<String>) {
        self.model_id = model;
    }

    /// Initialise the TUI (equivalent to `interactiveMode.init()` in TS).
    /// Sets up the terminal and renders the initial frame.
    pub async fn init(&mut self) -> anyhow::Result<()> {
        if let Some(msg) = &self.options.initial_message.clone() {
            let (_, at_refs, _) = Self::expand_at_refs(msg);
            self.messages.push(ChatMessage {
                role: MessageRole::User,
                content: msg.clone(),
                at_refs,
                full_output: None,
                full_output_expanded: false,
            });
        }
        Ok(())
    }

    /// Run the interactive TUI event loop (equivalent to `interactiveMode.run()`).
    pub async fn run(&mut self) -> anyhow::Result<()> {
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
        let backend = CrosstermBackend::new(stdout);
        let mut terminal = Terminal::new(backend)?;

        self.running = true;

        // Send initial message to agent if provided
        if let Some(msg) = self.options.initial_message.clone()
            && !msg.is_empty()
        {
            let (expanded, at_refs, warnings) = Self::expand_at_refs(&msg);
            self.messages.push(ChatMessage {
                role: MessageRole::User,
                content: msg.clone(),
                at_refs,
                full_output: None,
                full_output_expanded: false,
            });
            for warn in warnings {
                self.messages.push(ChatMessage {
                    role: MessageRole::System,
                    content: warn,
                    at_refs: Vec::new(),
                    full_output: None,
                    full_output_expanded: false,
                });
            }
            self.messages.push(ChatMessage {
                role: MessageRole::Assistant,
                content: String::new(),
                at_refs: Vec::new(),
                    full_output: None,
                    full_output_expanded: false,
            });
            self.spawn_agent(expanded);
        }

        let mut event_stream = EventStream::new();

        loop {
            // Drain one approval request from agent tasks (show dialog; handle one at a time).
            // Guard: only accept a new request when none is pending, to prevent overwriting
            // a live request (which would silently deny it when the sender is dropped).
            if self.pending_approval.is_none() {
                if let Ok(req) = self.approval_rx.try_recv() {
                    self.pending_approval = Some(req);
                }
            }

            // Drain agent response deltas (disjoint field borrows allow updates inside)
            let mut disconnected = false;
            if let Some(rx) = &mut self.agent_rx {
                loop {
                    match rx.try_recv() {
                        Ok(AgentDelta::Text(delta)) => {
                            if let Some(last) = self.messages.last_mut() {
                                if last.role == MessageRole::Assistant {
                                    last.content.push_str(&delta);
                                } else {
                                    self.messages.push(ChatMessage {
                                        role: MessageRole::Assistant,
                                        content: delta,
                                        at_refs: Vec::new(),
                    full_output: None,
                    full_output_expanded: false,
                                    });
                                }
                            }
                        }
                        Ok(AgentDelta::TurnUsage {
                            usage,
                            model,
                            is_fast,
                            context_window,
                        }) => {
                            self.session_input_tokens += usage.input;
                            self.session_output_tokens += usage.output;
                            self.session_cache_read_tokens += usage.cache_read;
                            self.session_cache_write_tokens += usage.cache_write;
                            self.context_window = context_window;
                            // CC formula: full context = input + cache_read + cache_write + output
                            self.last_turn_context_tokens =
                                usage.input + usage.cache_read + usage.cache_write + usage.output;
                            let cost =
                                ai::model_pricing::calculate_usd_cost(&usage, &model, is_fast);
                            self.session_cost_usd += cost.total;
                            // Warn when context window is within 20k tokens of the limit
                            // (mirrors CC's WARNING_THRESHOLD_BUFFER_TOKENS = 20_000).
                            const WARN_BUFFER: u64 = 20_000;
                            if context_window > 0 && !self.compact_warned {
                                let threshold = context_window as u64;
                                let used = self.last_turn_context_tokens;
                                if used + WARN_BUFFER >= threshold {
                                    let pct = (used * 100 / threshold).min(100);
                                    self.compact_warned = true;
                                    self.messages.push(ChatMessage {
                                        role: MessageRole::System,
                                        content: format!("Context at {pct}% — consider /compact to summarize the conversation"),
                                        at_refs: Vec::new(),
                                        full_output: None,
                                        full_output_expanded: false,
                                    });
                                }
                            }
                            // Persist every assistant turn produced since the last TurnUsage.
                            // Iterating from `saved_up_to` avoids re-saving earlier turns
                            // and correctly captures multi-turn responses interleaved with tools.
                            let saved_up_to = self.session_assistant_saved_up_to;
                            for msg in &self.messages[saved_up_to..] {
                                if msg.role == MessageRole::Assistant && !msg.content.is_empty() {
                                    self.session_manager.append_message(serde_json::json!({
                                        "role": "assistant",
                                        "content": [{"type": "text", "text": msg.content}]
                                    }));
                                }
                            }
                            self.session_assistant_saved_up_to = self.messages.len();
                        }
                        Ok(AgentDelta::ToolStart { name, args_preview }) => {
                            self.current_tool = Some(name.clone());
                            let content = if args_preview.is_empty() {
                                name.clone()
                            } else {
                                format!("{name}({args_preview})")
                            };
                            self.messages.push(ChatMessage {
                                role: MessageRole::Tool {
                                    name,
                                    pending: true,
                                    success: false,
                                },
                                content,
                                at_refs: Vec::new(),
                    full_output: None,
                    full_output_expanded: false,
                            });
                        }
                        Ok(AgentDelta::ToolEnd {
                            name,
                            success,
                            output_preview,
                            full_output,
                        }) => {
                            let pos = self.messages.iter().rposition(|m| {
                                matches!(&m.role, MessageRole::Tool { name: n, pending: true, .. } if n == &name)
                            });
                            if let Some(idx) = pos {
                                if !output_preview.is_empty() {
                                    let existing = self.messages[idx].content.clone();
                                    self.messages[idx].content = format!(
                                        "{existing} · {}",
                                        output_preview.chars().take(80).collect::<String>()
                                    );
                                }
                                self.messages[idx].role = MessageRole::Tool {
                                    name,
                                    pending: false,
                                    success,
                                };
                                if !full_output.is_empty() {
                                    self.messages[idx].full_output = Some(full_output);
                                }
                            }
                            self.current_tool = None;
                        }
                        Ok(AgentDelta::ImageResult { tool_name, base64, mime_type }) => {
                            use tui::terminal_image;
                            let dims = terminal_image::get_image_dimensions(&base64, &mime_type);
                            let content = if let Some(d) = dims {
                                let max_cols = (self.last_terminal_width / 2).max(40) as u32;
                                let opts = terminal_image::ImageRenderOptions {
                                    max_width_cells: Some(max_cols),
                                    max_height_cells: Some(20),
                                    preserve_aspect_ratio: Some(true),
                                    image_id: Some(terminal_image::allocate_image_id()),
                                };
                                match terminal_image::render_image(&base64, d, &opts) {
                                    Some((seq, _, _)) => seq,
                                    None => terminal_image::image_fallback(&mime_type, None, Some(&tool_name)),
                                }
                            } else {
                                terminal_image::image_fallback(&mime_type, None, Some(&tool_name))
                            };
                            self.messages.push(ChatMessage {
                                role: MessageRole::Tool {
                                    name: tool_name,
                                    pending: false,
                                    success: true,
                                },
                                content,
                                at_refs: Vec::new(),
                                full_output: None,
                                full_output_expanded: false,
                            });
                        }
                        Ok(AgentDelta::Error(err)) => {
                            self.is_thinking = false;
                            self.current_tool = None;
                            self.messages.push(ChatMessage {
                                role: MessageRole::Error,
                                content: err,
                                at_refs: Vec::new(),
                    full_output: None,
                    full_output_expanded: false,
                            });
                        }
                        Ok(AgentDelta::ThinkingStart) => {
                            self.messages.push(ChatMessage {
                                role: MessageRole::Thinking {
                                    duration_ms: 0,
                                    expanded: false,
                                },
                                content: String::new(),
                                at_refs: Vec::new(),
                    full_output: None,
                    full_output_expanded: false,
                            });
                        }
                        Ok(AgentDelta::ThinkingEnd {
                            duration_ms,
                            content,
                        }) => {
                            if let Some(msg) = self
                                .messages
                                .iter_mut()
                                .rev()
                                .find(|m| matches!(m.role, MessageRole::Thinking { .. }))
                            {
                                msg.content = content;
                                if let MessageRole::Thinking {
                                    duration_ms: ref mut d,
                                    ..
                                } = msg.role
                                {
                                    *d = duration_ms;
                                }
                            }
                        }
                        Ok(AgentDelta::CompactionDone { summary }) => {
                            // /compact 完成 — LLM 生成的摘要保存下来，下一轮起注入到 system prompt。
                            self.compact_summary = Some(summary);
                            self.compacting = false;
                        }
                        Err(mpsc::error::TryRecvError::Empty) => break,
                        Err(mpsc::error::TryRecvError::Disconnected) => {
                            disconnected = true;
                            break;
                        }
                    }
                }
            }
            if disconnected {
                self.agent_rx = None;
                self.agent_handle = None;
                self.is_thinking = false;
                self.current_tool = None;
            }

            // Sticky scroll: pin to bottom as new content arrives.
            // Must be computed before terminal.draw() so render() is side-effect-free.
            if self.is_sticky {
                let total = Self::total_content_lines(&self.messages, self.last_terminal_width);
                self.scroll_top = total.saturating_sub(self.last_viewport_height);
            }
            terminal.draw(|f| self.render(f))?;

            tokio::select! {
                maybe_event = event_stream.next() => {
                    let Some(Ok(event)) = maybe_event else { continue };
                    match event {
                        Event::Key(key) => {
                            // Approval dialog intercepts most keys when active.
                            if self.pending_approval.is_some() {
                                match (key.code, key.modifiers) {
                                    // Ctrl+C / Ctrl+D always exits, even from the dialog.
                                    (KeyCode::Char('c'), KeyModifiers::CONTROL)
                                    | (KeyCode::Char('d'), KeyModifiers::CONTROL) => {
                                        self.abort_agent();
                                        self.running = false;
                                        break;
                                    }
                                    (KeyCode::Char('y'), _) => {
                                        self.resolve_approval(ApprovalResponse::Allow);
                                    }
                                    (KeyCode::Char('Y'), _) => {
                                        self.resolve_approval(ApprovalResponse::AllowAlways);
                                    }
                                    (KeyCode::Char('n'), _) | (KeyCode::Esc, _) => {
                                        self.resolve_approval(ApprovalResponse::Deny);
                                    }
                                    (KeyCode::Char('N'), _) => {
                                        self.resolve_approval(ApprovalResponse::DenyAlways);
                                    }
                                    _ => {}
                                }
                                continue;
                            }

                            // Settings overlay intercepts all keys when active.
                            if self.settings_active {
                                match (key.code, key.modifiers) {
                                    (KeyCode::Char('c'), KeyModifiers::CONTROL)
                                    | (KeyCode::Char('d'), KeyModifiers::CONTROL) => {
                                        self.abort_agent();
                                        self.running = false;
                                        break;
                                    }
                                    (KeyCode::Up, _) | (KeyCode::Char('k'), _) => {
                                        if self.settings_selected > 0 {
                                            self.settings_selected -= 1;
                                            if self.settings_selected < self.settings_scroll {
                                                self.settings_scroll = self.settings_selected;
                                            }
                                        }
                                    }
                                    (KeyCode::Down, _) | (KeyCode::Char('j'), _) => {
                                        if self.settings_selected + 1 < self.settings_items.len() {
                                            self.settings_selected += 1;
                                            const SETTINGS_VISIBLE: usize = 15;
                                            if self.settings_selected >= self.settings_scroll + SETTINGS_VISIBLE {
                                                self.settings_scroll = self.settings_selected + 1 - SETTINGS_VISIBLE;
                                            }
                                        }
                                    }
                                    (KeyCode::Left, _) | (KeyCode::Char('h'), _) => {
                                        self.settings_cycle_value(false);
                                    }
                                    (KeyCode::Right, _) | (KeyCode::Char('l'), _) => {
                                        self.settings_cycle_value(true);
                                    }
                                    (KeyCode::Enter, _) => {
                                        self.settings_apply_selected();
                                    }
                                    (KeyCode::Esc, _) | (KeyCode::Char('q'), _) => {
                                        self.settings_active = false;
                                    }
                                    _ => {}
                                }
                                continue;
                            }

                            // Search mode intercepts character input and navigation.
                            if self.search_active {
                                match (key.code, key.modifiers) {
                                    (KeyCode::Char('c'), KeyModifiers::CONTROL)
                                    | (KeyCode::Char('d'), KeyModifiers::CONTROL) => {
                                        self.abort_agent();
                                        self.running = false;
                                        break;
                                    }
                                    (KeyCode::Esc, _) => {
                                        self.search_active = false;
                                        self.search_query.clear();
                                        self.search_matches.clear();
                                        self.search_idx = 0;
                                    }
                                    (KeyCode::Backspace, _) => {
                                        self.search_query.pop();
                                        self.refresh_search();
                                    }
                                    (KeyCode::Enter, _) | (KeyCode::Char('n'), _) => {
                                        self.search_next(1);
                                    }
                                    (KeyCode::Char('N'), _) => {
                                        self.search_next(-1);
                                    }
                                    (KeyCode::Char(c), _) => {
                                        self.search_query.push(c);
                                        self.refresh_search();
                                        self.search_next(0);
                                    }
                                    _ => {}
                                }
                                continue;
                            }

                            match (key.code, key.modifiers) {
                                (KeyCode::Char('c'), KeyModifiers::CONTROL)
                                | (KeyCode::Char('d'), KeyModifiers::CONTROL) => {
                                    self.abort_agent();
                                    self.running = false;
                                    break;
                                }
                                // Ctrl+P — cycle through scoped models (same as CC)
                                (KeyCode::Char('p'), KeyModifiers::CONTROL) => {
                                    self.cycle_scoped_model();
                                }
                                // Ctrl+U — clear current input line
                                (KeyCode::Char('u'), KeyModifiers::CONTROL) => {
                                    self.input_buffer.clear();
                                    self.completion_matches.clear();
                                    self.completion_selected = 0;
                                }
                                // Ctrl+W — delete last word
                                (KeyCode::Char('w'), KeyModifiers::CONTROL) => {
                                    // Find last whitespace boundary and truncate there.
                                    let trimmed = self.input_buffer.trim_end_matches(|c: char| c.is_whitespace());
                                    let last_space = trimmed.rfind(|c: char| c.is_whitespace())
                                        .map(|i| i + 1)
                                        .unwrap_or(0);
                                    self.input_buffer.truncate(last_space);
                                    self.update_completion_matches();
                                }
                                // Ctrl+L — force full redraw (useful if terminal is corrupted)
                                (KeyCode::Char('l'), KeyModifiers::CONTROL) => {
                                    terminal.clear()?;
                                }
                                // Scroll shortcuts — active only when input buffer is empty
                                // so j/k/g/G are available for typing when composing a message.
                                (KeyCode::Char('j'), KeyModifiers::NONE)
                                    if self.input_buffer.is_empty() =>
                                {
                                    self.is_sticky = false;
                                    self.scroll_top = self.scroll_top.saturating_add(1);
                                    self.clamp_scroll();
                                }
                                (KeyCode::Char('k'), KeyModifiers::NONE)
                                    if self.input_buffer.is_empty() =>
                                {
                                    self.is_sticky = false;
                                    self.scroll_top = self.scroll_top.saturating_sub(1);
                                }
                                (KeyCode::Char('g'), KeyModifiers::NONE)
                                    if self.input_buffer.is_empty() =>
                                {
                                    self.is_sticky = false;
                                    self.scroll_top = 0;
                                }
                                (KeyCode::Char('G'), modifiers)
                                    if self.input_buffer.is_empty()
                                        && (modifiers.is_empty()
                                            || modifiers == KeyModifiers::SHIFT) =>
                                {
                                    self.is_sticky = true;
                                }
                                // Toggle nearest thinking block expansion
                                (KeyCode::Char('t'), KeyModifiers::NONE)
                                    if self.input_buffer.is_empty() =>
                                {
                                    if let Some(msg) = self.messages.iter_mut().rev().find(|m| {
                                        matches!(m.role, MessageRole::Thinking { .. })
                                    }) {
                                        if let MessageRole::Thinking {
                                            ref mut expanded, ..
                                        } = msg.role
                                        {
                                            *expanded = !*expanded;
                                        }
                                    }
                                }
                                // PageUp/PageDown always scroll regardless of input state
                                (KeyCode::PageDown, _) => {
                                    self.is_sticky = false;
                                    self.scroll_top = self
                                        .scroll_top
                                        .saturating_add(self.last_viewport_height);
                                    self.clamp_scroll();
                                }
                                (KeyCode::PageUp, _) => {
                                    self.is_sticky = false;
                                    self.scroll_top = self
                                        .scroll_top
                                        .saturating_sub(self.last_viewport_height);
                                }
                                // Slash command navigation
                                (KeyCode::Tab, _) | (KeyCode::Down, _)
                                    if !self.completion_matches.is_empty() =>
                                {
                                    let n = self.completion_matches.len();
                                    self.completion_selected = (self.completion_selected + 1) % n;
                                }
                                (KeyCode::Up, _) if !self.completion_matches.is_empty() => {
                                    let n = self.completion_matches.len();
                                    self.completion_selected = if self.completion_selected == 0 {
                                        n - 1
                                    } else {
                                        self.completion_selected - 1
                                    };
                                }
                                // History recall — only when completion menu is closed
                                (KeyCode::Up, _) if self.completion_matches.is_empty() => {
                                    if self.history.is_empty() {
                                        // nothing to recall
                                    } else if let Some(idx) = self.history_idx {
                                        // already browsing: move backwards (towards oldest)
                                        let new_idx = idx.saturating_sub(1);
                                        self.history_idx = Some(new_idx);
                                        self.input_buffer = self.history[new_idx].clone();
                                    } else {
                                        // first press: save draft, jump to latest
                                        self.history_draft =
                                            std::mem::take(&mut self.input_buffer);
                                        let new_idx = self.history.len() - 1;
                                        self.history_idx = Some(new_idx);
                                        self.input_buffer = self.history[new_idx].clone();
                                    }
                                }
                                (KeyCode::Down, _) if self.completion_matches.is_empty() => {
                                    if let Some(idx) = self.history_idx {
                                        if idx + 1 < self.history.len() {
                                            // move forwards (towards newest)
                                            let new_idx = idx + 1;
                                            self.history_idx = Some(new_idx);
                                            self.input_buffer = self.history[new_idx].clone();
                                        } else {
                                            // past the end: restore draft
                                            self.history_idx = None;
                                            self.input_buffer =
                                                std::mem::take(&mut self.history_draft);
                                        }
                                    }
                                }
                                // Input handling
                                (KeyCode::Enter, KeyModifiers::SHIFT) if !self.is_thinking => {
                                    self.input_buffer.push('\n');
                                }
                                (KeyCode::Enter, _) if !self.is_thinking => {
                                    // Completion selection
                                    if !self.completion_matches.is_empty() {
                                        let selected = self.completion_selected
                                            .min(self.completion_matches.len() - 1);
                                        let chosen = self.completion_matches[selected].0.clone();
                                        if self.input_buffer.starts_with('/') {
                                            self.input_buffer = format!("/{chosen} ");
                                        } else if let Some(at_pos) = self.input_buffer.rfind('@') {
                                            self.input_buffer.truncate(at_pos + 1);
                                            self.input_buffer.push_str(&chosen);
                                            self.input_buffer.push(' ');
                                        }
                                        self.completion_matches.clear();
                                        self.completion_selected = 0;
                                        continue;
                                    }
                                    let input = std::mem::take(&mut self.input_buffer);
                                    self.completion_matches.clear();
                                    self.completion_selected = 0;
                                    // Reset history navigation state on send.
                                    self.history_idx = None;
                                    self.history_draft.clear();
                                    if !input.trim().is_empty() {
                                        // Split "/<cmd> <args>" on the first space so that
                                        // "/thinkingfoo" never accidentally matches "/thinking".
                                        let trimmed = input.trim();
                                        let (cmd, args) = match trimmed.split_once(' ') {
                                            Some((c, a)) => (c, a.trim()),
                                            None => (trimmed, ""),
                                        };
                                        if !self.handle_builtin_slash_command(cmd, args) {
                                            self.history.push(input.clone());
                                            self.session_manager.append_message(serde_json::json!({
                                                "role": "user",
                                                "content": [{"type": "text", "text": input}]
                                            }));
                                            let (expanded, at_refs, warnings) =
                                                Self::expand_at_refs(&input);
                                            self.messages.push(ChatMessage {
                                                role: MessageRole::User,
                                                content: input.clone(),
                                                at_refs,
                                                full_output: None,
                                                full_output_expanded: false,
                                            });
                                            for warn in warnings {
                                                self.messages.push(ChatMessage {
                                                    role: MessageRole::System,
                                                    content: warn,
                                                    at_refs: Vec::new(),
                    full_output: None,
                    full_output_expanded: false,
                                                });
                                            }
                                            self.messages.push(ChatMessage {
                                                role: MessageRole::Assistant,
                                                content: String::new(),
                                                at_refs: Vec::new(),
                    full_output: None,
                    full_output_expanded: false,
                                            });
                                            self.is_sticky = true;
                                            self.spawn_agent(expanded);
                                        }
                                        if self.quit_pending {
                                            self.abort_agent();
                                            self.running = false;
                                            break;
                                        }
                                    }
                                }
                                // Ctrl+F — enter search mode
                                (KeyCode::Char('f'), KeyModifiers::CONTROL) => {
                                    self.search_active = true;
                                    self.search_query.clear();
                                    self.search_matches.clear();
                                    self.search_idx = 0;
                                }
                                // x — toggle expand on last tool message (when input empty)
                                (KeyCode::Char('x'), KeyModifiers::NONE)
                                    if self.input_buffer.is_empty() =>
                                {
                                    // Find the last tool message index that has full_output.
                                    let maybe_idx = self.messages.iter().enumerate().rev()
                                        .find(|(_, m)| {
                                            matches!(m.role, MessageRole::Tool { pending: false, .. })
                                                && m.full_output.is_some()
                                        })
                                        .map(|(i, _)| i);
                                    if let Some(idx) = maybe_idx {
                                        self.messages[idx].full_output_expanded =
                                            !self.messages[idx].full_output_expanded;
                                    }
                                }
                                (KeyCode::Backspace, _) => {
                                    self.input_buffer.pop();
                                    self.update_completion_matches();
                                }
                                (KeyCode::Esc, _) if !self.completion_matches.is_empty() => {
                                    self.completion_matches.clear();
                                    self.completion_selected = 0;
                                }
                                // Escape during agent execution cancels it (CC parity)
                                (KeyCode::Esc, _) if self.is_thinking => {
                                    self.abort_agent();
                                    self.is_thinking = false;
                                    self.messages.push(ChatMessage {
                                        role: MessageRole::System,
                                        content: "Interrupted".to_string(),
                                        at_refs: Vec::new(),
                    full_output: None,
                    full_output_expanded: false,
                                    });
                                    self.is_sticky = true;
                                }
                                (KeyCode::Char(c), _) => {
                                    self.input_buffer.push(c);
                                    self.update_completion_matches();
                                }
                                _ => {}
                            }
                        }
                        Event::Mouse(mouse) => match mouse.kind {
                            MouseEventKind::ScrollDown => {
                                self.is_sticky = false;
                                self.scroll_top = self.scroll_top.saturating_add(3);
                                self.clamp_scroll();
                            }
                            MouseEventKind::ScrollUp => {
                                self.is_sticky = false;
                                self.scroll_top = self.scroll_top.saturating_sub(3);
                            }
                            _ => {}
                        },
                        _ => {}
                    }
                }
                _ = tokio::time::sleep(std::time::Duration::from_millis(50)) => {
                    self.tick = self.tick.wrapping_add(1);
                    // Refresh git branch every ~10s (200 ticks × 50ms).
                    if self.tick % 200 == 0 {
                        self.git_branch = Self::read_git_branch();
                    }
                }
            }
        }

        disable_raw_mode()?;
        execute!(
            terminal.backend_mut(),
            LeaveAlternateScreen,
            DisableMouseCapture
        )?;
        terminal.show_cursor()?;
        Ok(())
    }

    /// Abort any running agent task and deny its pending approval (if any).
    /// Cycle to the next model in the scoped-models list (Ctrl+P).
    /// Falls back to a short status message when the list is empty.
    fn cycle_scoped_model(&mut self) {
        let s = self.settings_manager.get_effective_settings();
        let models: Vec<String> = s.enabled_models.unwrap_or_default();
        if models.is_empty() {
            self.messages.push(ChatMessage {
                role: MessageRole::System,
                content: "No scoped models set. Use /scoped-models <pattern> to add one."
                    .to_string(),
                at_refs: Vec::new(),
                    full_output: None,
                    full_output_expanded: false,
            });
            self.is_sticky = true;
            return;
        }
        // Find current model's position and advance by one.
        let current = self.model_id.as_deref().unwrap_or("");
        let next_idx = models
            .iter()
            .position(|m| m == current)
            .map(|i| (i + 1) % models.len())
            .unwrap_or(0);
        let next = models[next_idx].clone();
        self.model_id = Some(next.clone());
        self.messages.push(ChatMessage {
            role: MessageRole::System,
            content: format!("Model: {next}  ({}/{}) — Ctrl+P to cycle", next_idx + 1, models.len()),
            at_refs: Vec::new(),
                    full_output: None,
                    full_output_expanded: false,
        });
        self.is_sticky = true;
    }

    /// Save any assistant messages produced since the last TurnUsage but not yet
    /// persisted. Called before aborting or switching sessions so partial content
    /// is not silently dropped from the session file.
    fn flush_pending_assistant(&mut self) {
        let saved_up_to = self.session_assistant_saved_up_to;
        for msg in &self.messages[saved_up_to..] {
            if msg.role == MessageRole::Assistant && !msg.content.is_empty() {
                self.session_manager.append_message(serde_json::json!({
                    "role": "assistant",
                    "content": [{"type": "text", "text": msg.content}]
                }));
            }
        }
        self.session_assistant_saved_up_to = self.messages.len();
    }

    fn abort_agent(&mut self) {
        // Flush any partial assistant content before dropping the channel.
        self.flush_pending_assistant();
        // Deny pending approval so the agent-side oneshot doesn't hang until timeout.
        if let Some(req) = self.pending_approval.take() {
            let _ = req.response_tx.send(ApprovalResponse::Deny);
        }
        if let Some(handle) = self.agent_handle.take() {
            handle.abort();
        }
        // Drop the receiver immediately so stale deltas from the aborted task
        // are never read by the next event-loop iteration.
        self.agent_rx = None;
        self.current_tool = None;
        self.is_thinking = false;
    }

    /// Handle a built-in slash command. Returns `true` if handled (no agent
    /// should be spawned), `false` if the input should be treated as a normal
    /// message and forwarded to the agent.
    fn handle_builtin_slash_command(&mut self, cmd: &str, args: &str) -> bool {
        match cmd {
            "/permissions" => {
                let (new_mode, msg) = match args {
                    "bypass" => (
                        "bypassPermissions",
                        "⚡ BYPASS mode — all tool approvals skipped".to_string(),
                    ),
                    "plan" => (
                        "plan",
                        "📋 PLAN mode — read-only tools only".to_string(),
                    ),
                    "default" | "" => (
                        "default",
                        "Permissions: default mode (Ask)".to_string(),
                    ),
                    other => (
                        self.permission_mode.as_str(),
                        format!(
                            "Unknown permission mode: \"{other}\". Use: default | bypass | plan"
                        ),
                    ),
                };
                self.permission_mode = new_mode.to_string();
                self.messages.push(ChatMessage {
                    role: MessageRole::System,
                    content: msg,
                    at_refs: Vec::new(),
                    full_output: None,
                    full_output_expanded: false,
                });
                self.is_sticky = true;
                true
            }
            "/thinking" => {
                let (new_level, ok) = if args.is_empty() {
                    (self.thinking_level.cycle(), true)
                } else {
                    match args.parse::<ThinkingLevel>() {
                        Ok(l) => (l, true),
                        Err(_) => (self.thinking_level, false),
                    }
                };
                let msg = if ok {
                    self.thinking_level = new_level;
                    format!("🧠 Thinking level: {}", new_level.as_str())
                } else {
                    format!(
                        "Unknown thinking level: \"{args}\". Use: off | minimal | low | medium | high | xhigh"
                    )
                };
                self.messages.push(ChatMessage {
                    role: MessageRole::System,
                    content: msg,
                    at_refs: Vec::new(),
                    full_output: None,
                    full_output_expanded: false,
                });
                self.is_sticky = true;
                true
            }
            "/compact" => {
                // UI-only trim: removes old messages from the display.
                // The LLM context is not affected — each session is
                // stateless and uses only the current message.
                const COMPACT_KEEP: usize = 20;
                let total = self.messages.len();
                if total > COMPACT_KEEP {
                    self.messages.drain(..total - COMPACT_KEEP);
                    // saved_up_to may now point past the trimmed messages;
                    // clamp it so the next TurnUsage slice doesn't panic.
                    self.session_assistant_saved_up_to =
                        self.session_assistant_saved_up_to.min(self.messages.len());
                }
                self.messages.push(ChatMessage {
                    role: MessageRole::System,
                    content: format!(
                        "✂ Display trimmed — last {COMPACT_KEEP} messages visible (LLM context unchanged)"
                    ),
                    at_refs: Vec::new(),
                    full_output: None,
                    full_output_expanded: false,
                });
                self.is_sticky = true;
                true
            }
            "/model" => {
                if args.is_empty() {
                    let current = self.model_id.as_deref().unwrap_or("claude");
                    self.messages.push(ChatMessage {
                        role: MessageRole::System,
                        content: format!("Current model: {current}. Usage: /model <model-id>"),
                        at_refs: Vec::new(),
                    full_output: None,
                    full_output_expanded: false,
                    });
                } else {
                    self.model_id = Some(args.to_string());
                    self.messages.push(ChatMessage {
                        role: MessageRole::System,
                        content: format!("Model set to: {args} (validated on next request)"),
                        at_refs: Vec::new(),
                    full_output: None,
                    full_output_expanded: false,
                    });
                }
                self.is_sticky = true;
                true
            }
            "/quit" => {
                self.quit_pending = true;
                true
            }
            "/new" => {
                self.abort_agent();
                self.messages.clear();
                self.session_input_tokens = 0;
                self.session_output_tokens = 0;
                self.session_cache_read_tokens = 0;
                self.session_cache_write_tokens = 0;
                self.session_cost_usd = 0.0;
                self.scroll_top = 0;
                self.is_sticky = true;
                self.session_start = std::time::Instant::now();
                self.session_name = None;
                self.session_assistant_saved_up_to = 0;
                self.session_manager.new_session(None);
                self.messages.push(ChatMessage {
                    role: MessageRole::System,
                    content: "✦ New session started".to_string(),
                    at_refs: Vec::new(),
                    full_output: None,
                    full_output_expanded: false,
                });
                true
            }
            "/hotkeys" => {
                let text = "\
⌨  Keyboard Shortcuts

  Enter           Send message
  Shift+Enter     New line in message
  ↑ / ↓           Browse sent-message history
  j / k           Scroll up/down (when input empty)
  g / G           Scroll to top / bottom
  t               Toggle thinking block expand
  PageUp/Down     Scroll one page
  Tab / ↓         Next autocomplete match
  ↑               Previous autocomplete match
  Esc             Close autocomplete menu; cancel agent when running
  Ctrl+P          Cycle through scoped models
  Ctrl+U          Clear input
  Ctrl+W          Delete last word
  Ctrl+L          Force redraw (fix terminal corruption)
  Ctrl+C / D      Exit"
                    .to_string();
                self.messages.push(ChatMessage {
                    role: MessageRole::System,
                    content: text,
                    at_refs: Vec::new(),
                    full_output: None,
                    full_output_expanded: false,
                });
                self.is_sticky = true;
                true
            }
            "/settings" => {
                if args.is_empty() {
                    // No args: open the interactive settings overlay.
                    self.settings_items = self.settings_build_items();
                    self.settings_selected = 0;
                    self.settings_scroll = 0;
                    self.settings_active = true;
                } else {
                    // /settings set <key> <value> — text-based fallback.
                    let parts: Vec<&str> = args.splitn(3, ' ').collect();
                    if parts.len() >= 3 && parts[0] == "set" {
                        let key = parts[1];
                        let value = parts[2];
                        match key {
                            "default_model" | "defaultModel" => {
                                self.settings_manager.set_default_model(value);
                                self.model_id = Some(value.to_string());
                                self.messages.push(ChatMessage {
                                    role: MessageRole::System,
                                    content: format!("✓ default_model set to: {value}"),
                                    at_refs: Vec::new(),
                                    full_output: None,
                                    full_output_expanded: false,
                                });
                            }
                            "default_provider" | "defaultProvider" => {
                                self.settings_manager.set_default_provider(value);
                                self.provider_id = Some(value.to_string());
                                self.messages.push(ChatMessage {
                                    role: MessageRole::System,
                                    content: format!("✓ default_provider set to: {value}"),
                                    at_refs: Vec::new(),
                                    full_output: None,
                                    full_output_expanded: false,
                                });
                            }
                            "theme" => {
                                self.settings_manager.set_theme(value);
                                self.messages.push(ChatMessage {
                                    role: MessageRole::System,
                                    content: format!("✓ theme set to: {value}"),
                                    at_refs: Vec::new(),
                                    full_output: None,
                                    full_output_expanded: false,
                                });
                            }
                            "steering_mode" | "steeringMode" => {
                                self.settings_manager.set_steering_mode(value);
                                self.messages.push(ChatMessage {
                                    role: MessageRole::System,
                                    content: format!("✓ steering_mode set to: {value}"),
                                    at_refs: Vec::new(),
                                    full_output: None,
                                    full_output_expanded: false,
                                });
                            }
                            _ => {
                                self.messages.push(ChatMessage {
                                    role: MessageRole::System,
                                    content: format!(
                                        "Unknown setting: {key}. Supported: default_model, default_provider, theme, steering_mode"
                                    ),
                                    at_refs: Vec::new(),
                                    full_output: None,
                                    full_output_expanded: false,
                                });
                            }
                        }
                    } else {
                        self.messages.push(ChatMessage {
                            role: MessageRole::System,
                            content: "Usage: /settings set <key> <value>. Or just /settings to open the settings menu.".to_string(),
                            at_refs: Vec::new(),
                            full_output: None,
                            full_output_expanded: false,
                        });
                    }
                    self.is_sticky = true;
                }
                true
            }
            "/changelog" => {
                // Read the CHANGELOG.md from the crate root and display it.
                let changelog_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("../../CHANGELOG.md");
                let text = match std::fs::read_to_string(&changelog_path) {
                    Ok(content) => {
                        // Show only the first 60 lines to avoid overwhelming the display.
                        let lines: Vec<&str> = content.lines().take(60).collect();
                        lines.join("\n")
                    }
                    Err(_) => "No CHANGELOG.md found.".to_string(),
                };
                self.messages.push(ChatMessage {
                    role: MessageRole::System,
                    content: text,
                    at_refs: Vec::new(),
                    full_output: None,
                    full_output_expanded: false,
                });
                self.is_sticky = true;
                true
            }
            "/login" => {
                self.messages.push(ChatMessage {
                    role: MessageRole::System,
                    content: "OAuth login: run `sage --login` from the terminal, or set ANTHROPIC_API_KEY.".to_string(),
                    at_refs: Vec::new(),
                    full_output: None,
                    full_output_expanded: false,
                });
                self.is_sticky = true;
                true
            }
            "/logout" => {
                self.messages.push(ChatMessage {
                    role: MessageRole::System,
                    content: "Logout: unset ANTHROPIC_API_KEY or remove the credential from your keychain.".to_string(),
                    at_refs: Vec::new(),
                    full_output: None,
                    full_output_expanded: false,
                });
                self.is_sticky = true;
                true
            }
            "/copy" => {
                let last_assistant = self
                    .messages
                    .iter()
                    .rev()
                    .find(|m| m.role == MessageRole::Assistant && !m.content.is_empty());
                match last_assistant {
                    Some(msg) => {
                        let content = msg.content.clone();
                        let feedback = match copy_to_clipboard(&content) {
                            Ok(()) => "✓ Last response copied to clipboard".to_string(),
                            Err(e) => format!("✘ Copy failed: {e}"),
                        };
                        self.messages.push(ChatMessage {
                            role: MessageRole::System,
                            content: feedback,
                            at_refs: Vec::new(),
                    full_output: None,
                    full_output_expanded: false,
                        });
                    }
                    None => {
                        self.messages.push(ChatMessage {
                            role: MessageRole::System,
                            content: "No assistant response to copy".to_string(),
                            at_refs: Vec::new(),
                    full_output: None,
                    full_output_expanded: false,
                        });
                    }
                }
                self.is_sticky = true;
                true
            }
            "/session" => {
                let elapsed = self.session_start.elapsed();
                let secs = elapsed.as_secs();
                let (h, m, s) = (secs / 3600, (secs % 3600) / 60, secs % 60);
                let duration = if h > 0 {
                    format!("{h}h {m}m {s}s")
                } else if m > 0 {
                    format!("{m}m {s}s")
                } else {
                    format!("{s}s")
                };
                let model = self.model_id.as_deref().unwrap_or("claude");
                let msgs = self.messages.iter().filter(|m| m.role == MessageRole::User).count();
                let session_path = self.session_manager.get_session_file()
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|| "(in-memory)".to_string());
                let cache_line = if self.session_cache_read_tokens > 0 || self.session_cache_write_tokens > 0 {
                    format!(
                        "\n  Cache:   R{} / W{}",
                        Self::format_tokens(self.session_cache_read_tokens),
                        Self::format_tokens(self.session_cache_write_tokens),
                    )
                } else {
                    String::new()
                };
                let text = format!(
                    "📊 Session Stats\n  Model:   {model}\n  Time:    {duration}\n  Turns:   {msgs}\n  Input:   {} tokens\n  Output:  {} tokens{cache_line}\n  Cost:    ${:.4}\n  File:    {session_path}",
                    Self::format_tokens(self.session_input_tokens),
                    Self::format_tokens(self.session_output_tokens),
                    self.session_cost_usd,
                );
                self.messages.push(ChatMessage {
                    role: MessageRole::System,
                    content: text,
                    at_refs: Vec::new(),
                    full_output: None,
                    full_output_expanded: false,
                });
                self.is_sticky = true;
                true
            }
            "/name" => {
                if args.is_empty() {
                    let current = self.session_name.as_deref().unwrap_or("(unnamed)");
                    self.messages.push(ChatMessage {
                        role: MessageRole::System,
                        content: format!(
                            "Session name: {current}. Usage: /name <title>"
                        ),
                        at_refs: Vec::new(),
                    full_output: None,
                    full_output_expanded: false,
                    });
                } else {
                    self.session_name = Some(args.to_string());
                    self.session_manager.append_session_info(args);
                    self.messages.push(ChatMessage {
                        role: MessageRole::System,
                        content: format!("✓ Session name set to: {args}"),
                        at_refs: Vec::new(),
                    full_output: None,
                    full_output_expanded: false,
                    });
                }
                self.is_sticky = true;
                true
            }
            "/resume" => {
                if args.is_empty() {
                    let sessions = self.session_manager.list_sync();
                    if sessions.is_empty() {
                        self.messages.push(ChatMessage {
                            role: MessageRole::System,
                            content: "No saved sessions found. Usage: /resume <n>".to_string(),
                            at_refs: Vec::new(),
                    full_output: None,
                    full_output_expanded: false,
                        });
                    } else {
                        let mut text =
                            "📚 Saved Sessions (use /resume <n> to load):\n".to_string();
                        for (i, s) in sessions.iter().take(10).enumerate() {
                            let name = s.name.as_deref().unwrap_or(&s.first_message);
                            let short: String = name.chars().take(60).collect();
                            let date = s.modified.format("%Y-%m-%d %H:%M").to_string();
                            text.push_str(&format!(
                                "  [{n}] {date}  {short}\n",
                                n = i + 1
                            ));
                        }
                        text.push_str("\nType /resume <n> to load a session.");
                        self.messages.push(ChatMessage {
                            role: MessageRole::System,
                            content: text,
                            at_refs: Vec::new(),
                    full_output: None,
                    full_output_expanded: false,
                        });
                    }
                } else {
                    match args.trim().parse::<usize>() {
                        Ok(n) if n >= 1 => {
                            let sessions = self.session_manager.list_sync();
                            if n > sessions.len() {
                                self.messages.push(ChatMessage {
                                    role: MessageRole::System,
                                    content: format!(
                                        "No session #{n}. Use /resume to see the list."
                                    ),
                                    at_refs: Vec::new(),
                    full_output: None,
                    full_output_expanded: false,
                                });
                            } else {
                                let path = sessions[n - 1].path.clone();
                                self.abort_agent();
                                self.session_manager = SessionManager::open(&path, None);
                                self.messages.clear();
                                self.session_input_tokens = 0;
                                self.session_output_tokens = 0;
                                self.session_cache_read_tokens = 0;
                                self.session_cache_write_tokens = 0;
                                self.session_cost_usd = 0.0;
                                self.scroll_top = 0;
                                self.session_start = std::time::Instant::now();
                                let entries = self.session_manager.get_entries_ordered();
                                let replayed = Self::load_session_entries_into_messages(&entries);
                                let count = replayed.messages.len();
                                self.messages = replayed.messages;
                                self.session_assistant_saved_up_to = self.messages.len();
                                self.session_name = replayed
                                    .session_name
                                    .or_else(|| self.session_manager.get_session_name());
                                if let Some(tl) = replayed.thinking_level {
                                    self.thinking_level = tl;
                                }
                                if let Some(m) = replayed.model_id {
                                    self.model_id = Some(m);
                                }
                                if let Some(p) = replayed.provider_id {
                                    self.provider_id = Some(p);
                                }
                                self.messages.push(ChatMessage {
                                    role: MessageRole::System,
                                    content: format!(
                                        "✓ Session #{n} resumed: {count} messages"
                                    ),
                                    at_refs: Vec::new(),
                    full_output: None,
                    full_output_expanded: false,
                                });
                            }
                        }
                        _ => {
                            self.messages.push(ChatMessage {
                                role: MessageRole::System,
                                content: format!(
                                    "Invalid number: {args}. Use /resume to list sessions."
                                ),
                                at_refs: Vec::new(),
                    full_output: None,
                    full_output_expanded: false,
                            });
                        }
                    }
                }
                self.is_sticky = true;
                true
            }
            "/export" => {
                match self.session_manager.get_session_file() {
                    None => {
                        self.messages.push(ChatMessage {
                            role: MessageRole::System,
                            content:
                                "No session file to export (start a conversation first)."
                                    .to_string(),
                            at_refs: Vec::new(),
                    full_output: None,
                    full_output_expanded: false,
                        });
                    }
                    Some(source) => {
                        let source = source.to_path_buf();
                        let dest = if args.is_empty() {
                            let date = chrono::Local::now()
                                .format("%Y-%m-%d")
                                .to_string();
                            format!("sage-session-{date}.jsonl")
                        } else {
                            args.to_string()
                        };
                        match std::fs::copy(&source, &dest) {
                            Ok(_) => {
                                self.messages.push(ChatMessage {
                                    role: MessageRole::System,
                                    content: format!("✓ Session exported to: {dest}"),
                                    at_refs: Vec::new(),
                    full_output: None,
                    full_output_expanded: false,
                                });
                            }
                            Err(e) => {
                                self.messages.push(ChatMessage {
                                    role: MessageRole::System,
                                    content: format!("✘ Export failed: {e}"),
                                    at_refs: Vec::new(),
                    full_output: None,
                    full_output_expanded: false,
                                });
                            }
                        }
                    }
                }
                self.is_sticky = true;
                true
            }
            "/import" => {
                if args.is_empty() {
                    self.messages.push(ChatMessage {
                        role: MessageRole::System,
                        content: "Usage: /import <path.jsonl>".to_string(),
                        at_refs: Vec::new(),
                    full_output: None,
                    full_output_expanded: false,
                    });
                } else {
                    let path = std::path::Path::new(args);
                    if !path.exists() {
                        self.messages.push(ChatMessage {
                            role: MessageRole::System,
                            content: format!("✘ File not found: {args}"),
                            at_refs: Vec::new(),
                    full_output: None,
                    full_output_expanded: false,
                        });
                    } else if !Self::is_valid_session_file(path) {
                        self.messages.push(ChatMessage {
                            role: MessageRole::System,
                            content: format!(
                                "✘ Not a valid session file: {args}\nExpected a JSONL file with a session header."
                            ),
                            at_refs: Vec::new(),
                    full_output: None,
                    full_output_expanded: false,
                        });
                    } else {
                        self.abort_agent();
                        self.session_manager = SessionManager::open(path, None);
                        self.messages.clear();
                        self.session_input_tokens = 0;
                        self.session_output_tokens = 0;
                        self.session_cache_read_tokens = 0;
                        self.session_cache_write_tokens = 0;
                        self.session_cost_usd = 0.0;
                        self.scroll_top = 0;
                        self.session_start = std::time::Instant::now();
                        let entries = self.session_manager.get_entries_ordered();
                        let replayed = Self::load_session_entries_into_messages(&entries);
                        let count = replayed.messages.len();
                        self.messages = replayed.messages;
                        self.session_assistant_saved_up_to = self.messages.len();
                        self.session_name = replayed
                            .session_name
                            .or_else(|| self.session_manager.get_session_name());
                        if let Some(tl) = replayed.thinking_level {
                            self.thinking_level = tl;
                        }
                        if let Some(m) = replayed.model_id {
                            self.model_id = Some(m);
                        }
                        if let Some(p) = replayed.provider_id {
                            self.provider_id = Some(p);
                        }
                        self.messages.push(ChatMessage {
                            role: MessageRole::System,
                            content: format!(
                                "✓ Session imported: {count} messages from {args}"
                            ),
                            at_refs: Vec::new(),
                    full_output: None,
                    full_output_expanded: false,
                        });
                    }
                }
                self.is_sticky = true;
                true
            }
            "/fork" => {
                let entries = self.session_manager.get_entries_ordered();
                let message_ids: Vec<String> = entries
                    .iter()
                    .filter(|e| {
                        e.is_message()
                            && (e.message_role() == Some("user")
                                || e.message_role() == Some("assistant"))
                    })
                    .map(|e| e.id().to_string())
                    .collect();
                let fork_count = message_ids.len();
                if fork_count == 0 {
                    self.messages.push(ChatMessage {
                        role: MessageRole::System,
                        content: "No messages to fork from. Start a conversation first."
                            .to_string(),
                        at_refs: Vec::new(),
                    full_output: None,
                    full_output_expanded: false,
                    });
                    self.is_sticky = true;
                    return true;
                }
                let branch_idx = if args.is_empty() {
                    fork_count - 1
                } else {
                    match args.parse::<usize>() {
                        Ok(n) if n >= 1 && n <= fork_count => n - 1,
                        _ => {
                            self.messages.push(ChatMessage {
                                role: MessageRole::System,
                                content: format!(
                                    "Usage: /fork [<turn>]. Session has {fork_count} turns."
                                ),
                                at_refs: Vec::new(),
                    full_output: None,
                    full_output_expanded: false,
                            });
                            self.is_sticky = true;
                            return true;
                        }
                    }
                };
                let leaf_id = message_ids[branch_idx].clone();
                match self.session_manager.create_branched_session(&leaf_id) {
                    Ok(_) => {
                        self.abort_agent();
                        let new_entries = self.session_manager.get_entries_ordered();
                        let replayed = Self::load_session_entries_into_messages(&new_entries);
                        let count = replayed.messages.len();
                        self.messages = replayed.messages;
                        self.session_assistant_saved_up_to = self.messages.len();
                        self.session_name = replayed.session_name;
                        if let Some(tl) = replayed.thinking_level {
                            self.thinking_level = tl;
                        }
                        if let Some(m) = replayed.model_id {
                            self.model_id = Some(m);
                        }
                        if let Some(p) = replayed.provider_id {
                            self.provider_id = Some(p);
                        }
                        self.session_input_tokens = 0;
                        self.session_output_tokens = 0;
                        self.session_cache_read_tokens = 0;
                        self.session_cache_write_tokens = 0;
                        self.session_cost_usd = 0.0;
                        self.scroll_top = 0;
                        self.session_start = std::time::Instant::now();
                        self.messages.push(ChatMessage {
                            role: MessageRole::System,
                            content: format!(
                                "✦ Forked from turn {}: {count} messages in new branch",
                                branch_idx + 1
                            ),
                            at_refs: Vec::new(),
                    full_output: None,
                    full_output_expanded: false,
                        });
                    }
                    Err(e) => {
                        self.messages.push(ChatMessage {
                            role: MessageRole::System,
                            content: format!("✘ Fork failed: {e}"),
                            at_refs: Vec::new(),
                    full_output: None,
                    full_output_expanded: false,
                        });
                    }
                }
                self.is_sticky = true;
                true
            }
            "/tree" => {
                let current_file = self
                    .session_manager
                    .get_session_file()
                    .map(|p| p.to_path_buf());
                let session_id = self.session_manager.get_session_id().to_string();
                let name = self.session_name.as_deref().unwrap_or("(unnamed)");
                let entries = self.session_manager.get_entries_ordered();
                let msg_count = entries
                    .iter()
                    .filter(|e| {
                        e.is_message()
                            && (e.message_role() == Some("user")
                                || e.message_role() == Some("assistant"))
                    })
                    .count();

                // List all sessions and find branches of the current one.
                let all_sessions = self.session_manager.list_sync();
                let current_path_str = current_file
                    .as_ref()
                    .map(|p| p.to_string_lossy().to_string());
                let mut text = format!(
                    "🌳 Session Tree\n  Current:  {name}  [{msg_count} msgs]  id={session_id}\n"
                );
                let branches: Vec<_> = all_sessions
                    .iter()
                    .filter(|s| {
                        s.parent_session_path.as_deref() == current_path_str.as_deref()
                    })
                    .collect();
                if branches.is_empty() {
                    text.push_str("  No branches forked from this session.\n");
                } else {
                    text.push_str(&format!("  Branches ({}):\n", branches.len()));
                    for b in branches.iter().take(10) {
                        let bname = b.name.as_deref().unwrap_or(&b.first_message);
                        let short: String = bname.chars().take(50).collect();
                        let date = b.modified.format("%Y-%m-%d %H:%M").to_string();
                        text.push_str(&format!(
                            "    ├─ {date}  {short}  [{} msgs]\n",
                            b.message_count
                        ));
                    }
                }
                text.push_str("\nUse /fork to create a branch. Use /resume to switch sessions.");
                self.messages.push(ChatMessage {
                    role: MessageRole::System,
                    content: text,
                    at_refs: Vec::new(),
                    full_output: None,
                    full_output_expanded: false,
                });
                self.is_sticky = true;
                true
            }
            "/share" => {
                match self.session_manager.get_session_file() {
                    None => {
                        self.messages.push(ChatMessage {
                            role: MessageRole::System,
                            content: "No session file available. Start a conversation first."
                                .to_string(),
                            at_refs: Vec::new(),
                    full_output: None,
                    full_output_expanded: false,
                        });
                    }
                    Some(p) => {
                        let path = p.to_path_buf();
                        self.messages.push(ChatMessage {
                            role: MessageRole::System,
                            content: "📤 Uploading session as secret GitHub Gist…".to_string(),
                            at_refs: Vec::new(),
                    full_output: None,
                    full_output_expanded: false,
                        });
                        match std::process::Command::new("gh")
                            .args(["gist", "create", "--secret", "--filename", "sage-session.jsonl"])
                            .arg(&path)
                            .output()
                        {
                            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                                self.messages.push(ChatMessage {
                                    role: MessageRole::System,
                                    content: format!(
                                        "✘ `gh` not found. Install GitHub CLI (brew install gh) then run:\n  gh gist create --secret {}",
                                        path.display()
                                    ),
                                    at_refs: Vec::new(),
                    full_output: None,
                    full_output_expanded: false,
                                });
                            }
                            Err(e) => {
                                self.messages.push(ChatMessage {
                                    role: MessageRole::System,
                                    content: format!("✘ Share failed: {e}"),
                                    at_refs: Vec::new(),
                    full_output: None,
                    full_output_expanded: false,
                                });
                            }
                            Ok(out) => {
                                if out.status.success() {
                                    let url = String::from_utf8_lossy(&out.stdout)
                                        .trim()
                                        .to_string();
                                    self.messages.push(ChatMessage {
                                        role: MessageRole::System,
                                        content: format!("✓ Session shared: {url}"),
                                        at_refs: Vec::new(),
                    full_output: None,
                    full_output_expanded: false,
                                    });
                                } else {
                                    let stderr = String::from_utf8_lossy(&out.stderr)
                                        .trim()
                                        .to_string();
                                    self.messages.push(ChatMessage {
                                        role: MessageRole::System,
                                        content: format!("✘ gh gist create failed:\n{stderr}"),
                                        at_refs: Vec::new(),
                    full_output: None,
                    full_output_expanded: false,
                                    });
                                }
                            }
                        }
                    }
                }
                self.is_sticky = true;
                true
            }
            "/reload" => {
                self.settings_manager.reload();
                self.messages.push(ChatMessage {
                    role: MessageRole::System,
                    content: "✓ Settings reloaded from disk".to_string(),
                    at_refs: Vec::new(),
                    full_output: None,
                    full_output_expanded: false,
                });
                self.is_sticky = true;
                true
            }
            "/scoped-models" => {
                if args.is_empty() {
                    let s = self.settings_manager.get_effective_settings();
                    let models = s.enabled_models.unwrap_or_default();
                    if models.is_empty() {
                        self.messages.push(ChatMessage {
                            role: MessageRole::System,
                            content: "No scoped models set. Usage: /scoped-models <pattern> to toggle. Models cycle with Ctrl+P.".to_string(),
                            at_refs: Vec::new(),
                    full_output: None,
                    full_output_expanded: false,
                        });
                    } else {
                        let list = models.join(", ");
                        self.messages.push(ChatMessage {
                            role: MessageRole::System,
                            content: format!(
                                "Scoped models (Ctrl+P cycles): {list}\nUsage: /scoped-models <pattern> to toggle"
                            ),
                            at_refs: Vec::new(),
                    full_output: None,
                    full_output_expanded: false,
                        });
                    }
                } else {
                    let s = self.settings_manager.get_effective_settings();
                    let mut models: Vec<String> = s.enabled_models.unwrap_or_default();
                    if let Some(pos) = models.iter().position(|m| m == args) {
                        models.remove(pos);
                        let new_models = if models.is_empty() {
                            None
                        } else {
                            Some(models.clone())
                        };
                        self.settings_manager.set_enabled_models(new_models);
                        let list = if models.is_empty() {
                            "(empty)".to_string()
                        } else {
                            models.join(", ")
                        };
                        self.messages.push(ChatMessage {
                            role: MessageRole::System,
                            content: format!(
                                "✓ Removed {args} from scoped models. Current: {list}"
                            ),
                            at_refs: Vec::new(),
                    full_output: None,
                    full_output_expanded: false,
                        });
                    } else {
                        models.push(args.to_string());
                        self.settings_manager
                            .set_enabled_models(Some(models.clone()));
                        let list = models.join(", ");
                        self.messages.push(ChatMessage {
                            role: MessageRole::System,
                            content: format!(
                                "✓ Added {args} to scoped models. Current: {list}"
                            ),
                            at_refs: Vec::new(),
                    full_output: None,
                    full_output_expanded: false,
                        });
                    }
                }
                self.is_sticky = true;
                true
            }
            "/init" => {
                let mut context = String::new();
                if let Ok(out) = std::process::Command::new("git")
                    .args(["log", "--oneline", "-10"])
                    .output()
                {
                    if !out.stdout.is_empty() {
                        context.push_str("Recent git history:\n");
                        context.push_str(&String::from_utf8_lossy(&out.stdout));
                        context.push('\n');
                    }
                }
                for cargo_file in &["Cargo.toml", "package.json", "pyproject.toml"] {
                    if let Ok(content) = std::fs::read_to_string(cargo_file) {
                        let snippet: String = content.chars().take(1000).collect();
                        context.push_str(&format!(
                            "{cargo_file}:\n```\n{snippet}\n```\n"
                        ));
                        break;
                    }
                }
                for readme in &["README.md", "README.txt", "README"] {
                    if let Ok(content) = std::fs::read_to_string(readme) {
                        let snippet: String = content.chars().take(2000).collect();
                        context.push_str(&format!("{readme}:\n{snippet}\n"));
                        break;
                    }
                }
                let prompt = if context.is_empty() {
                    "Please create a CLAUDE.md file in the current directory. Include: project purpose, directory structure, key commands, and important conventions.".to_string()
                } else {
                    format!(
                        "Please create a CLAUDE.md file in the current directory.\n\nContext:\n{context}\n\nInclude: project purpose, directory structure, key commands, dependencies, and important conventions."
                    )
                };
                self.session_manager.append_message(serde_json::json!({
                    "role": "user",
                    "content": [{"type": "text", "text": "/init (create CLAUDE.md)"}]
                }));
                self.messages.push(ChatMessage {
                    role: MessageRole::System,
                    content: "🚀 Generating CLAUDE.md...".to_string(),
                    at_refs: Vec::new(),
                    full_output: None,
                    full_output_expanded: false,
                });
                self.messages.push(ChatMessage {
                    role: MessageRole::User,
                    content: "/init (create CLAUDE.md)".to_string(),
                    at_refs: Vec::new(),
                    full_output: None,
                    full_output_expanded: false,
                });
                self.messages.push(ChatMessage {
                    role: MessageRole::Assistant,
                    content: String::new(),
                    at_refs: Vec::new(),
                    full_output: None,
                    full_output_expanded: false,
                });
                self.is_sticky = true;
                self.spawn_agent(prompt);
                true
            }
            _ => false,
        }
    }

    /// Load session entries, replaying messages and the most recent state-change
    /// entries (ThinkingLevelChange, ModelChange, SessionInfo).
    fn load_session_entries_into_messages(entries: &[SessionEntry]) -> ReplayedState {
        let mut messages = Vec::new();
        let mut thinking_level: Option<ThinkingLevel> = None;
        let mut model_id: Option<String> = None;
        let mut provider_id: Option<String> = None;
        let mut session_name: Option<String> = None;

        for entry in entries {
            match entry {
                SessionEntry::Message(e) => {
                    let role =
                        e.message.get("role").and_then(|r| r.as_str()).unwrap_or("");
                    let content = Self::extract_message_text(&e.message);
                    let msg_role = match role {
                        "user" => MessageRole::User,
                        "assistant" => MessageRole::Assistant,
                        _ => continue,
                    };
                    if !content.is_empty() {
                        messages.push(ChatMessage {
                            role: msg_role,
                            content,
                            at_refs: Vec::new(),
                    full_output: None,
                    full_output_expanded: false,
                        });
                    }
                }
                SessionEntry::ThinkingLevelChange(e) => {
                    thinking_level = e.thinking_level.parse().ok();
                }
                SessionEntry::ModelChange(e) => {
                    model_id = Some(e.model_id.clone());
                    provider_id = Some(e.provider.clone());
                }
                SessionEntry::SessionInfo(e) => {
                    if e.name.is_some() {
                        session_name = e.name.clone();
                    }
                }
                // Compaction, BranchSummary, Custom, CustomMessage, Label: no TUI state to replay.
                _ => {}
            }
        }

        ReplayedState {
            messages,
            thinking_level,
            model_id,
            provider_id,
            session_name,
        }
    }

    /// Extract text content from a session message JSON value.
    fn extract_message_text(message: &serde_json::Value) -> String {
        match message.get("content") {
            Some(serde_json::Value::String(s)) => s.clone(),
            Some(serde_json::Value::Array(arr)) => arr
                .iter()
                .filter_map(|b| {
                    if b.get("type")?.as_str()? == "text" {
                        b.get("text")?.as_str().map(|s| s.to_string())
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>()
                .join(""),
            _ => String::new(),
        }
    }

    /// Check if a file looks like a valid sage session JSONL.
    /// Reads only the first line to avoid loading large files.
    fn is_valid_session_file(path: &std::path::Path) -> bool {
        use std::io::{BufRead, BufReader};
        let Ok(file) = std::fs::File::open(path) else {
            return false;
        };
        let mut reader = BufReader::new(file);
        let mut first_line = String::new();
        if reader.read_line(&mut first_line).is_err() {
            return false;
        }
        let Ok(v) = serde_json::from_str::<serde_json::Value>(first_line.trim()) else {
            return false;
        };
        // A session file's first entry must have a "type" field (any SessionEntry variant).
        v.get("type").and_then(|t| t.as_str()).is_some()
    }

    fn spawn_agent(&mut self, message: String) {
        // Abort any prior orphan task before starting a new one.
        self.abort_agent();

        let (tx, rx) = mpsc::unbounded_channel::<AgentDelta>();
        self.agent_rx = Some(rx);
        self.is_thinking = true;

        let provider_id = self.provider_id.clone();
        let model_id = self.model_id.clone();
        let permission_mode = self.permission_mode.clone();
        let thinking_level = self.thinking_level;
        let error_tx = tx.clone();
        let approval_tx = self.approval_tx.clone();
        let session_rules = Arc::clone(&self.session_rules);

        let compact_summary = self.compact_summary.clone();
        let handle = tokio::spawn(async move {
            if let Err(e) = crate::print_session::run_agent_session_to_channel(
                message,
                model_id,
                provider_id,
                None,
                tx,
                permission_mode,
                thinking_level,
                Some(approval_tx),
                session_rules,
                compact_summary,
            )
            .await
            {
                let _ = error_tx.send(AgentDelta::Error(e.to_string()));
            }
        });
        self.agent_handle = Some(handle);
    }

    /// Resolve the current pending approval and send response to agent.
    fn resolve_approval(&mut self, response: ApprovalResponse) {
        if let Some(req) = self.pending_approval.take() {
            let _ = req.response_tx.send(response);
        }
    }

    /// Update completion_matches based on current input_buffer.
    ///
    /// - `/prefix` → builtin slash commands starting with prefix
    /// - `…@prefix` → files matching prefix* in the working directory
    fn update_completion_matches(&mut self) {
        if self.input_buffer.starts_with('/') {
            let prefix = &self.input_buffer[1..];
            self.completion_matches = BUILTIN_SLASH_COMMANDS
                .iter()
                .filter(|c| c.name.starts_with(prefix))
                .map(|c| (c.name.to_string(), c.description.to_string()))
                .collect();
        } else if let Some(at_pos) = self.input_buffer.rfind('@') {
            // Only trigger on `@` that is preceded by whitespace (or at start).
            // `at_pos` from `rfind('@')` is always a valid char boundary (@ is ASCII).
            // `map_or(true, …)` handles the start-of-string case (empty slice → None → true).
            let preceded_by_space = self.input_buffer[..at_pos]
                .chars()
                .next_back()
                .map_or(true, |c| c.is_whitespace());
            if preceded_by_space {
                let file_prefix = &self.input_buffer[at_pos + 1..];
                self.completion_matches = Self::file_completions_for(file_prefix);
            } else {
                self.completion_matches.clear();
            }
        } else {
            self.completion_matches.clear();
        }
        // Always reset selection so the list doesn't "jump" when items change.
        self.completion_selected = 0;
    }

    // ── Search helpers ────────────────────────────────────────────────────────

    fn refresh_search(&mut self) {
        if self.search_query.is_empty() {
            self.search_matches.clear();
            self.search_idx = 0;
            return;
        }
        let q = self.search_query.to_lowercase();
        self.search_matches = self.messages.iter().enumerate()
            .filter(|(_, m)| m.content.to_lowercase().contains(&q))
            .map(|(i, _)| i)
            .collect();
    }

    fn search_next(&mut self, direction: i32) {
        let n = self.search_matches.len();
        if n == 0 {
            return;
        }
        match direction {
            d if d > 0 => {
                self.search_idx = (self.search_idx + 1) % n;
            }
            d if d < 0 => {
                self.search_idx = if self.search_idx == 0 { n - 1 } else { self.search_idx - 1 };
            }
            _ => {}
        }
        // Jump scroll to the matched message.
        self.scroll_to_message(self.search_matches[self.search_idx]);
    }

    fn scroll_to_message(&mut self, msg_idx: usize) {
        // Count display lines for all messages before msg_idx.
        let theme = get_theme();
        let w = self.last_terminal_width;
        let line_offset: u16 = self.messages[..msg_idx]
            .iter()
            .map(|m| Self::message_to_lines(m, &theme, w).len() as u16)
            .sum();
        let target = line_offset.saturating_sub(self.last_viewport_height / 3);
        self.scroll_top = target;
        self.is_sticky = false;
        self.clamp_scroll();
    }

    // ── Settings helpers ──────────────────────────────────────────────────────

    fn settings_build_items(&self) -> Vec<crate::modes::interactive::components::settings_selector::SettingItem> {
        use crate::modes::interactive::components::settings_selector::SettingItem;
        let s = self.settings_manager.get_effective_settings();
        let bool_str = |b: bool| if b { "true" } else { "false" };
        vec![
            // ── UI / Interaction ──────────────────────────────────────────
            SettingItem {
                id: "thinking-level",
                label: "Thinking level",
                description: "Extended thinking budget for complex problems",
                current_value: self.thinking_level.as_str().to_string(),
                values: vec!["off".into(), "low".into(), "medium".into(), "high".into()],
            },
            SettingItem {
                id: "permission-mode",
                label: "Permission mode",
                description: "Tool execution permission level",
                current_value: self.permission_mode.clone(),
                values: vec!["default".into(), "plan".into(), "bypassPermissions".into()],
            },
            SettingItem {
                id: "steering-mode",
                label: "Steering mode",
                description: "Enter while streaming queues steering messages",
                current_value: s.steering_mode.as_deref().unwrap_or("one-at-a-time").to_string(),
                values: vec!["one-at-a-time".into(), "all".into()],
            },
            SettingItem {
                id: "follow-up-mode",
                label: "Follow-up mode",
                description: "Alt+Enter queues follow-up messages until agent stops",
                current_value: s.follow_up_mode.as_deref().unwrap_or("one-at-a-time").to_string(),
                values: vec!["one-at-a-time".into(), "all".into()],
            },
            SettingItem {
                id: "transport",
                label: "Transport",
                description: "Preferred transport for providers that support multiple",
                current_value: s.transport.as_deref().unwrap_or("sse").to_string(),
                values: vec!["sse".into(), "websocket".into(), "auto".into()],
            },
            // ── Display ───────────────────────────────────────────────────
            SettingItem {
                id: "hide-thinking",
                label: "Hide thinking",
                description: "Hide thinking blocks in assistant responses",
                current_value: bool_str(s.hide_thinking_block.unwrap_or(false)).to_string(),
                values: vec!["false".into(), "true".into()],
            },
            SettingItem {
                id: "collapse-changelog",
                label: "Collapse changelog",
                description: "Show condensed changelog after updates",
                current_value: bool_str(s.collapse_changelog.unwrap_or(false)).to_string(),
                values: vec!["false".into(), "true".into()],
            },
            SettingItem {
                id: "show-images",
                label: "Show images",
                description: "Render images inline in terminal",
                current_value: bool_str(s.terminal.as_ref().and_then(|t| t.show_images).unwrap_or(true)).to_string(),
                values: vec!["true".into(), "false".into()],
            },
            SettingItem {
                id: "auto-resize-images",
                label: "Auto-resize images",
                description: "Resize large images to 2000×2000 max",
                current_value: bool_str(s.images.as_ref().and_then(|i| i.auto_resize).unwrap_or(true)).to_string(),
                values: vec!["true".into(), "false".into()],
            },
            SettingItem {
                id: "block-images",
                label: "Block images",
                description: "Prevent images from being sent to LLM providers",
                current_value: bool_str(s.images.as_ref().and_then(|i| i.block_images).unwrap_or(false)).to_string(),
                values: vec!["false".into(), "true".into()],
            },
            SettingItem {
                id: "show-hardware-cursor",
                label: "Hardware cursor",
                description: "Show terminal cursor for IME support",
                current_value: bool_str(s.show_hardware_cursor.unwrap_or(false)).to_string(),
                values: vec!["false".into(), "true".into()],
            },
            SettingItem {
                id: "editor-padding",
                label: "Editor padding",
                description: "Horizontal padding for input editor (0–3)",
                current_value: s.editor_padding_x.unwrap_or(0).to_string(),
                values: vec!["0".into(), "1".into(), "2".into(), "3".into()],
            },
            SettingItem {
                id: "autocomplete-max",
                label: "Autocomplete max",
                description: "Max visible items in autocomplete dropdown",
                current_value: s.autocomplete_max_visible.unwrap_or(5).to_string(),
                values: vec!["3".into(), "5".into(), "7".into(), "10".into(), "15".into(), "20".into()],
            },
            // ── Session ───────────────────────────────────────────────────
            SettingItem {
                id: "autocompact",
                label: "Auto-compact",
                description: "Automatically compact context when it gets too large",
                current_value: bool_str(s.compaction.as_ref().and_then(|c| c.enabled).unwrap_or(true)).to_string(),
                values: vec!["true".into(), "false".into()],
            },
            SettingItem {
                id: "quiet-startup",
                label: "Quiet startup",
                description: "Disable verbose printing at startup",
                current_value: bool_str(s.quiet_startup.unwrap_or(false)).to_string(),
                values: vec!["false".into(), "true".into()],
            },
            // ── Commands ──────────────────────────────────────────────────
            SettingItem {
                id: "skill-commands",
                label: "Skill commands",
                description: "Register skills as /skill:name commands",
                current_value: bool_str(s.enable_skill_commands.unwrap_or(false)).to_string(),
                values: vec!["false".into(), "true".into()],
            },
            SettingItem {
                id: "double-escape",
                label: "Double-escape action",
                description: "Action when pressing Escape twice with empty editor",
                current_value: s.double_escape_action.as_deref().unwrap_or("none").to_string(),
                values: vec!["none".into(), "tree".into(), "fork".into()],
            },
            SettingItem {
                id: "tree-filter",
                label: "Tree filter mode",
                description: "Default filter when opening /tree",
                current_value: s.tree_filter_mode.as_deref().unwrap_or("default").to_string(),
                values: vec!["default".into(), "no-tools".into(), "user-only".into(), "labeled-only".into(), "all".into()],
            },
        ]
    }

    fn settings_cycle_value(&mut self, forward: bool) {
        let idx = self.settings_selected;
        if idx >= self.settings_items.len() {
            return;
        }
        let item = &self.settings_items[idx];
        let n = item.values.len();
        if n == 0 {
            return;
        }
        let cur_pos = item.values.iter().position(|v| v == &item.current_value).unwrap_or(0);
        let new_pos = if forward {
            (cur_pos + 1) % n
        } else {
            if cur_pos == 0 { n - 1 } else { cur_pos - 1 }
        };
        self.settings_items[idx].current_value = self.settings_items[idx].values[new_pos].clone();
    }

    fn settings_apply_selected(&mut self) {
        let idx = self.settings_selected;
        if idx >= self.settings_items.len() {
            return;
        }
        let item = self.settings_items[idx].clone();
        match item.id {
            "thinking-level" => {
                self.thinking_level = match item.current_value.as_str() {
                    "low" => ThinkingLevel::Low,
                    "medium" => ThinkingLevel::Medium,
                    "high" => ThinkingLevel::High,
                    _ => ThinkingLevel::Off,
                };
            }
            "permission-mode" => {
                self.permission_mode = item.current_value.clone();
            }
            "steering-mode" => {
                self.settings_manager.set_steering_mode(&item.current_value);
            }
            "follow-up-mode" => {
                self.settings_manager.set_follow_up_mode(&item.current_value);
            }
            "transport" => {
                self.settings_manager.set_transport(&item.current_value);
            }
            "hide-thinking" => {
                self.settings_manager.set_hide_thinking_block(item.current_value == "true");
            }
            "collapse-changelog" => {
                self.settings_manager.set_collapse_changelog(item.current_value == "true");
            }
            "show-images" => {
                self.settings_manager.set_show_images(item.current_value == "true");
            }
            "auto-resize-images" => {
                self.settings_manager.set_image_auto_resize(item.current_value == "true");
            }
            "block-images" => {
                self.settings_manager.set_block_images(item.current_value == "true");
            }
            "skill-commands" => {
                self.settings_manager.set_enable_skill_commands(item.current_value == "true");
            }
            "show-hardware-cursor" => {
                self.settings_manager.set_show_hardware_cursor(item.current_value == "true");
            }
            "editor-padding" => {
                if let Ok(v) = item.current_value.parse::<u32>() {
                    self.settings_manager.set_editor_padding_x(v);
                }
            }
            "autocomplete-max" => {
                if let Ok(v) = item.current_value.parse::<u32>() {
                    self.settings_manager.set_autocomplete_max_visible(v);
                }
            }
            "autocompact" => {
                self.settings_manager.set_compaction_enabled(item.current_value == "true");
            }
            "quiet-startup" => {
                self.settings_manager.set_quiet_startup(item.current_value == "true");
            }
            "double-escape" => {
                self.settings_manager.set_double_escape_action(&item.current_value);
            }
            "tree-filter" => {
                self.settings_manager.set_tree_filter_mode(&item.current_value);
            }
            _ => {}
        }
    }

    /// List files/dirs that start with `prefix` (up to MAX_COMPLETIONS results).
    ///
    /// Escapes glob metacharacters in the prefix and restricts results to paths
    /// that are inside (or equal to) the current working directory.
    fn file_completions_for(prefix: &str) -> Vec<(String, String)> {
        const MAX_COMPLETIONS: usize = 5;
        let escaped = glob::Pattern::escape(prefix);
        let pattern = format!("{escaped}*");
        // Canonicalize cwd so that symlink paths (e.g. macOS /tmp → /private/tmp)
        // are resolved before prefix-checking canonicalized child paths.
        let cwd = std::env::current_dir().and_then(|c| c.canonicalize()).ok();
        match glob::glob(&pattern) {
            Ok(paths) => {
                let mut results: Vec<(String, String)> = paths
                    .filter_map(|e| e.ok())
                    .filter(|p| {
                        // Reject paths that escape the working directory.
                        if let Some(cwd) = &cwd {
                            p.canonicalize()
                                .map(|c| c.starts_with(cwd))
                                .unwrap_or(false)
                        } else {
                            true
                        }
                    })
                    .take(MAX_COMPLETIONS)
                    .map(|p| {
                        let hint = if p.is_dir() {
                            "dir".to_string()
                        } else {
                            "file".to_string()
                        };
                        (p.display().to_string(), hint)
                    })
                    .collect();
                results.sort_by(|a, b| a.0.cmp(&b.0));
                results
            }
            Err(_) => Vec::new(),
        }
    }

    /// Scan `input` for `@path` tokens, read each file, and inject its content
    /// as an XML block prefix before the user's message.
    ///
    /// Returns `(expanded_message, at_refs, warnings)`:
    /// - `expanded_message`: the text to send to the LLM (with `<file>` blocks prepended)
    /// - `at_refs`: successfully loaded references (for TUI annotation display)
    /// - `warnings`: human-readable error strings for files that could not be read
    fn expand_at_refs(input: &str) -> (String, Vec<AtRef>, Vec<String>) {
        static RE: OnceLock<Regex> = OnceLock::new();
        let re = RE.get_or_init(|| Regex::new(r"@(\S+)").unwrap());

        const MAX_FILE_BYTES: usize = 100 * 1024;
        const MAX_TRUNCATE_LINES: usize = 200;

        let mut at_refs: Vec<AtRef> = Vec::new();
        let mut warnings: Vec<String> = Vec::new();
        let mut file_blocks = String::new();
        let mut seen = std::collections::HashSet::new();

        for cap in re.captures_iter(input) {
            let path = cap[1].to_string();
            if !seen.insert(path.clone()) {
                continue;
            }
            match std::fs::read_to_string(&path) {
                Ok(content) => {
                    let line_count = content.lines().count();
                    let file_content = if content.len() > MAX_FILE_BYTES {
                        let head: Vec<&str> = content.lines().take(MAX_TRUNCATE_LINES).collect();
                        format!(
                            "{}\n[... truncated: showing first {} of {} lines]",
                            head.join("\n"),
                            MAX_TRUNCATE_LINES,
                            line_count
                        )
                    } else {
                        content.trim_end().to_string()
                    };
                    file_blocks.push_str(&format!(
                        "<file path=\"{path}\">\n{file_content}\n</file>\n"
                    ));
                    at_refs.push(AtRef { path, line_count });
                }
                Err(e) => {
                    warnings.push(format!("@{path}: {e}"));
                }
            }
        }

        let expanded = if file_blocks.is_empty() {
            input.to_string()
        } else {
            format!("{file_blocks}{input}")
        };

        (expanded, at_refs, warnings)
    }

    /// Stop the interactive mode (e.g., for startup benchmarks).
    pub fn stop(&mut self) {
        self.running = false;
    }

    // ── Scroll helpers ──────────────────────────────────────────────────────

    /// Estimate display height of one message in terminal rows.
    ///
    /// Each content line is divided into chunks of `effective_width` columns.
    /// The prefix (`  ❯ ` or `  ◆ `, 4 cols) is part of the first line's budget.
    fn compute_message_height(msg: &ChatMessage, inner_width: u16) -> u16 {
        // Thinking blocks: collapsed = 1 line; expanded = 1 header + N content lines.
        if let MessageRole::Thinking { expanded, .. } = msg.role {
            if !expanded {
                return 1;
            }
            let content_rows = msg.content.lines().count().max(1) as u16;
            return 1 + content_rows;
        }

        let first_line = msg.content.lines().next().unwrap_or("");
        if first_line.starts_with("diff --git") {
            // Diff branch renders: 1 prefix line + diff lines (each with 4-space indent).
            // Accumulate as usize to avoid u16 overflow on large diffs.
            let diff_rows: usize = msg
                .content
                .lines()
                .map(|l| {
                    let cols = l.width().saturating_add(4); // 4 = indent prefix
                    let w = (inner_width.max(1)) as usize;
                    cols.div_ceil(w).max(1)
                })
                .sum();
            return 1 + u16::try_from(diff_rows).unwrap_or(u16::MAX);
        }
        // prefix occupies the first ratatui row only; subsequent wrapped rows use full width.
        let prefix_len: u16 = 4; // "  ❯ " / "  ◆ " / etc. — all 4 display cols
        let first_capacity = inner_width.saturating_sub(prefix_len).max(1);
        let count: u16 = msg
            .content
            .lines()
            .map(|line| {
                let n = line.width().min(u16::MAX as usize) as u16;
                if n == 0 || n <= first_capacity {
                    1
                } else {
                    1 + (n - first_capacity).div_ceil(inner_width.max(1))
                }
            })
            .sum();
        // Each @ref annotation occupies one additional display row.
        count.max(1) + msg.at_refs.len() as u16
    }

    fn total_content_lines(messages: &[ChatMessage], inner_width: u16) -> u16 {
        messages
            .iter()
            .map(|m| Self::compute_message_height(m, inner_width))
            .sum()
    }

    /// Parse a text line into styled ratatui spans, handling inline Markdown:
    /// **bold**, *italic*, `code`, [link](url), with list prefix substitution.
    fn parse_inline_markdown(text: &str, theme: &Theme) -> Vec<Span<'static>> {
        static RE: OnceLock<Regex> = OnceLock::new();
        let re = RE.get_or_init(|| {
            // Match: [text](url), `code`, **bold**, ~~strikethrough~~, *italic* (in precedence order)
            Regex::new(r"\[([^\]]+)\]\(([^)]+)\)|`([^`]+)`|\*\*([^*]+)\*\*|~~([^~]+)~~|\*([^*\s][^*]*)\*").unwrap()
        });

        let mut spans: Vec<Span<'static>> = Vec::new();
        let mut last = 0;

        for cap in re.captures_iter(text) {
            let m = cap.get(0).unwrap();
            if m.start() > last {
                spans.push(Span::raw(text[last..m.start()].to_string()));
            }
            if let (Some(link_text), Some(_url)) = (cap.get(1), cap.get(2)) {
                // Render link text underlined in accent color; URL is omitted (terminal can't click)
                spans.push(Span::styled(
                    link_text.as_str().to_string(),
                    Style::default()
                        .fg(theme.ratatui_fg(ThemeColor::Accent))
                        .add_modifier(Modifier::UNDERLINED),
                ));
            } else if let Some(code) = cap.get(3) {
                let style = Style::default()
                    .fg(theme.ratatui_fg(ThemeColor::Muted))
                    .bg(theme.ratatui_bg(ThemeBg::CodeBg));
                spans.push(Span::styled(code.as_str().to_string(), style));
            } else if let Some(bold) = cap.get(4) {
                spans.push(Span::styled(
                    bold.as_str().to_string(),
                    Style::default().add_modifier(Modifier::BOLD),
                ));
            } else if let Some(strike) = cap.get(5) {
                spans.push(Span::styled(
                    strike.as_str().to_string(),
                    Style::default().add_modifier(Modifier::CROSSED_OUT),
                ));
            } else if let Some(italic) = cap.get(6) {
                spans.push(Span::styled(
                    italic.as_str().to_string(),
                    Style::default().add_modifier(Modifier::ITALIC),
                ));
            }
            last = m.end();
        }
        if last < text.len() {
            spans.push(Span::raw(text[last..].to_string()));
        }
        if spans.is_empty() {
            spans.push(Span::raw(text.to_string()));
        }
        spans
    }

    /// Convert a `ChatMessage` to ratatui `Line`s, handling embedded newlines.
    ///
    /// First content line: `  ❯ {text}` (user) or `  ◆ {text}` (assistant).
    /// Subsequent lines: indented by 4 spaces to align under the text.
    /// Diff content (detected by leading `diff --git` header) is rendered
    /// with ANSI color codes via `render_diff()`.
    /// For user messages with `@path` refs, a `📎 path (N lines)` annotation
    /// line is appended per successfully loaded file.
    fn message_to_lines(
        msg: &ChatMessage,
        theme: &Theme,
        terminal_width: u16,
    ) -> Vec<Line<'static>> {
        // Thinking blocks have their own rendering — collapsed or expanded.
        if let MessageRole::Thinking {
            duration_ms,
            expanded,
        } = msg.role
        {
            return Self::thinking_to_lines(msg, theme, duration_ms, expanded);
        }

        let (indicator, indicator_color, bg_color) = match &msg.role {
            MessageRole::User => (
                "❯",
                theme.ratatui_fg(ThemeColor::Accent),
                Some(theme.ratatui_bg(ThemeBg::UserMessageBg)),
            ),
            MessageRole::Assistant => ("◆", theme.ratatui_fg(ThemeColor::Accent), None),
            MessageRole::System => ("✦", theme.ratatui_fg(ThemeColor::Warning), None),
            MessageRole::Tool { pending: true, .. } => {
                ("⏺", theme.ratatui_fg(ThemeColor::Warning), None)
            }
            MessageRole::Tool { success: true, .. } => {
                ("✓", theme.ratatui_fg(ThemeColor::Muted), None)
            }
            MessageRole::Tool { .. } => ("✘", theme.ratatui_fg(ThemeColor::Error), None),
            MessageRole::Error => ("✘", theme.ratatui_fg(ThemeColor::Error), None),
            MessageRole::Thinking { .. } => unreachable!("handled above"),
        };

        let prefix = format!("  {indicator} ");
        let indent = "    ".to_string();
        let mut lines: Vec<Line<'static>> = Vec::new();

        // Detect unified diff content (requires the canonical `diff --git` header).
        let first_line = msg.content.lines().next().unwrap_or("");
        let looks_like_diff = first_line.starts_with("diff --git");

        if looks_like_diff && bg_color.is_none() {
            // Render prefix on its own line, then diff lines with ratatui-native colors
            // (no ANSI escape strings — ratatui uses Style/Span, not raw escape codes).
            lines.push(Line::from(Span::styled(
                prefix.clone(),
                Style::default().fg(indicator_color),
            )));
            for mut diff_line in render_diff_ratatui(&msg.content, theme) {
                // Prepend 4-space indent to each span in the line.
                diff_line.spans.insert(0, Span::raw("    "));
                lines.push(diff_line);
            }
            return lines;
        }

        // For completed tool messages with full output, handle expand/collapse.
        let tool_full_content_buf: String;
        let effective_content: &str = if matches!(msg.role, MessageRole::Tool { pending: false, .. })
            && msg.full_output.is_some()
        {
            if msg.full_output_expanded {
                tool_full_content_buf = msg.full_output.as_deref().unwrap_or("").to_string();
                &tool_full_content_buf
            } else {
                &msg.content
            }
        } else {
            &msg.content
        };

        let content_lines: Vec<&str> = if effective_content.is_empty() {
            vec![""]
        } else {
            // Limit expanded output to 200 lines.
            let all: Vec<&str> = effective_content.lines().collect();
            if msg.full_output_expanded && all.len() > 200 {
                all[..200].to_vec()
            } else {
                all
            }
        };

        let mut in_code_block = false;
        let mut parse_state_slot: Option<ParseState> = None;

        for (i, text_line) in content_lines.iter().enumerate() {
            let lead_text = if i == 0 {
                prefix.clone()
            } else {
                indent.clone()
            };
            let lead = Span::styled(lead_text.clone(), Style::default().fg(indicator_color));
            let content_span = Span::styled(text_line.to_string(), Style::default());

            if let Some(bg) = bg_color {
                // Pad the line to terminal width for full-width background.
                let used = lead_text.width() + text_line.width();
                let pad = (terminal_width as usize).saturating_sub(used);
                let padding = Span::styled(" ".repeat(pad), Style::default().bg(bg));
                let lead_bg = Span::styled(lead_text, Style::default().fg(indicator_color).bg(bg));
                let content_bg = Span::styled(text_line.to_string(), Style::default().bg(bg));
                lines.push(Line::from(vec![lead_bg, content_bg, padding]));
            } else if matches!(msg.role, MessageRole::Assistant) {
                // Code fence detection — opening/closing ``` markers.
                if let Some(rest) = text_line.strip_prefix("```") {
                    let fence_style =
                        Style::default().fg(theme.ratatui_fg(ThemeColor::MdCodeBlockBorder));
                    if in_code_block {
                        in_code_block = false;
                        parse_state_slot = None;
                        lines.push(Line::from(vec![
                            lead,
                            Span::styled("```".to_string(), fence_style),
                        ]));
                    } else {
                        in_code_block = true;
                        let lang_tag = rest.trim();
                        let ss = syntax_set();
                        parse_state_slot = find_syntax_for_lang(ss, lang_tag).map(ParseState::new);
                        let fence_text = if lang_tag.is_empty() {
                            "```".to_string()
                        } else {
                            format!("```{lang_tag}")
                        };
                        lines.push(Line::from(vec![
                            lead,
                            Span::styled(fence_text, fence_style),
                        ]));
                    }
                    continue;
                }

                if in_code_block {
                    let ss = syntax_set();
                    let code_spans = if let Some(state) = parse_state_slot.as_mut() {
                        highlight_code_line(text_line, state, ss, theme)
                    } else {
                        vec![Span::styled(
                            text_line.to_string(),
                            Style::default().fg(theme.ratatui_fg(ThemeColor::Muted)),
                        )]
                    };
                    let mut line_spans = vec![lead];
                    line_spans.extend(code_spans);
                    lines.push(Line::from(line_spans));
                    continue;
                }

                // Render inline Markdown for assistant messages.

                // Headings: # H1, ## H2, ### H3
                let heading_level = if text_line.starts_with("### ") {
                    Some((3usize, &text_line[4..]))
                } else if text_line.starts_with("## ") {
                    Some((2, &text_line[3..]))
                } else if text_line.starts_with("# ") {
                    Some((1, &text_line[2..]))
                } else {
                    None
                };
                if let Some((level, heading_text)) = heading_level {
                    let heading_style = if level == 1 {
                        Style::default()
                            .fg(theme.ratatui_fg(ThemeColor::Accent))
                            .add_modifier(Modifier::BOLD)
                    } else if level == 2 {
                        Style::default().add_modifier(Modifier::BOLD)
                    } else {
                        Style::default()
                            .fg(theme.ratatui_fg(ThemeColor::Muted))
                            .add_modifier(Modifier::BOLD)
                    };
                    let mut line_spans = vec![lead];
                    line_spans.push(Span::styled(heading_text.to_string(), heading_style));
                    lines.push(Line::from(line_spans));
                    continue;
                }

                // Horizontal rules: --- or *** or === (three or more chars, only those chars)
                let trimmed = text_line.trim();
                if (trimmed.len() >= 3)
                    && (trimmed.chars().all(|c| c == '-')
                        || trimmed.chars().all(|c| c == '*')
                        || trimmed.chars().all(|c| c == '='))
                    && !trimmed.is_empty()
                {
                    let rule = "─".repeat((terminal_width as usize).saturating_sub(4));
                    lines.push(Line::from(vec![
                        lead,
                        Span::styled(rule, Style::default().fg(theme.ratatui_fg(ThemeColor::BorderMuted))),
                    ]));
                    continue;
                }

                // Blockquotes: > text
                if let Some(rest) = text_line.strip_prefix("> ").or_else(|| text_line.strip_prefix(">")) {
                    let bar = Span::styled(
                        "│ ".to_string(),
                        Style::default().fg(theme.ratatui_fg(ThemeColor::BorderMuted)),
                    );
                    let mut line_spans = vec![lead, bar];
                    line_spans.extend(Self::parse_inline_markdown(rest, theme));
                    lines.push(Line::from(line_spans));
                    continue;
                }

                // Markdown table rows: | col | col |
                if text_line.trim_start().starts_with('|') && text_line.trim_end().ends_with('|') {
                    let cells: Vec<&str> = text_line
                        .trim()
                        .trim_matches('|')
                        .split('|')
                        .map(str::trim)
                        .collect();
                    // Skip separator rows like |---|---|
                    let is_separator = cells.iter().all(|c| c.chars().all(|ch| ch == '-' || ch == ':' || ch == ' '));
                    if is_separator {
                        let rule = "─".repeat((terminal_width as usize).saturating_sub(4));
                        lines.push(Line::from(vec![
                            lead,
                            Span::styled(rule, Style::default().fg(theme.ratatui_fg(ThemeColor::BorderMuted))),
                        ]));
                    } else {
                        let mut line_spans = vec![lead];
                        for (ci, cell) in cells.iter().enumerate() {
                            if ci > 0 {
                                line_spans.push(Span::styled(
                                    "  │  ".to_string(),
                                    Style::default().fg(theme.ratatui_fg(ThemeColor::BorderMuted)),
                                ));
                            }
                            line_spans.extend(Self::parse_inline_markdown(cell, theme));
                        }
                        lines.push(Line::from(line_spans));
                    }
                    continue;
                }

                // Ordered list: `1. `, `2. `, etc.
                let ordered_bullet = {
                    let num_end = text_line.find(". ").unwrap_or(0);
                    if num_end > 0 && num_end <= 3 && text_line[..num_end].chars().all(|c| c.is_ascii_digit()) {
                        let num_str = &text_line[..num_end + 2]; // "1. "
                        let rest = &text_line[num_end + 2..];
                        Some((num_str, rest))
                    } else {
                        None
                    }
                };

                // Unordered list markers (`- ` / `* `) are replaced with a styled bullet.
                let (body, list_prefix) = if let Some(rest) = text_line
                    .strip_prefix("- ")
                    .or_else(|| text_line.strip_prefix("* "))
                {
                    let bullet = Span::styled(
                        "• ".to_string(),
                        Style::default().fg(theme.ratatui_fg(ThemeColor::MdListBullet)),
                    );
                    (rest, Some(bullet))
                } else if let Some((num, rest)) = ordered_bullet {
                    let bullet = Span::styled(
                        num.to_string(),
                        Style::default().fg(theme.ratatui_fg(ThemeColor::MdListBullet)),
                    );
                    (rest, Some(bullet))
                } else {
                    (*text_line, None)
                };
                let mut line_spans = vec![lead];
                if let Some(b) = list_prefix {
                    line_spans.push(b);
                }
                line_spans.extend(Self::parse_inline_markdown(body, theme));
                lines.push(Line::from(line_spans));
            } else if matches!(msg.role, MessageRole::System) {
                // System messages get inline markdown rendering (bold, code, italic).
                let mut line_spans = vec![lead];
                line_spans.extend(Self::parse_inline_markdown(text_line, theme));
                lines.push(Line::from(line_spans));
            } else {
                lines.push(Line::from(vec![lead, content_span]));
            }
        }

        // Annotation lines for expanded @refs (user messages only).
        for at_ref in &msg.at_refs {
            let annotation = format!("📎 {} ({} lines)", at_ref.path, at_ref.line_count);
            lines.push(Line::from(vec![
                Span::raw(indent.clone()),
                Span::styled(annotation, Style::default().fg(Color::DarkGray)),
            ]));
        }

        // Append expand/collapse hint for tool messages with full output.
        if matches!(msg.role, MessageRole::Tool { pending: false, .. })
            && msg.full_output.is_some()
        {
            let hint = if msg.full_output_expanded {
                "    [x collapse]"
            } else {
                "    … [x expand]"
            };
            lines.push(Line::from(Span::styled(
                hint.to_string(),
                Style::default().fg(theme.ratatui_fg(ThemeColor::Dim)),
            )));
        }

        lines
    }

    /// Render a thinking block as ratatui lines.
    ///
    /// Collapsed (default):  `  ◆ ▶ Thinking (1.2s) ···`
    /// Expanded:             `  ◆ ▼ Thinking`
    ///                       `    │ content line …`
    fn thinking_to_lines(
        msg: &ChatMessage,
        theme: &Theme,
        duration_ms: u64,
        expanded: bool,
    ) -> Vec<Line<'static>> {
        let dim_color = theme.ratatui_fg(ThemeColor::Dim);
        let thinking_color = theme.ratatui_fg(ThemeColor::ThinkingText);
        let indicator_style = Style::default().fg(dim_color);
        let text_style = Style::default()
            .fg(thinking_color)
            .add_modifier(Modifier::ITALIC);

        let duration_label = if duration_ms > 0 {
            let secs = duration_ms as f64 / 1000.0;
            format!(" ({:.1}s)", secs)
        } else {
            String::new()
        };

        if !expanded {
            // Collapsed: single line showing duration and ellipsis
            let header = format!("  ◆ ▶ Thinking{duration_label} ···");
            return vec![Line::from(Span::styled(header, text_style))];
        }

        // Expanded: header line + content lines with │ prefix
        let mut lines = Vec::new();
        let header = format!("  ◆ ▼ Thinking{duration_label}");
        lines.push(Line::from(Span::styled(header, text_style)));

        if msg.content.is_empty() {
            lines.push(Line::from(vec![Span::styled(
                "    │ ".to_string(),
                indicator_style,
            )]));
        } else {
            for content_line in msg.content.lines() {
                lines.push(Line::from(vec![
                    Span::styled("    │ ".to_string(), indicator_style),
                    Span::styled(content_line.to_string(), text_style),
                ]));
            }
        }

        lines
    }

    /// Clamp `scroll_top` so we never scroll past the last line of content.
    fn clamp_scroll(&mut self) {
        let inner_width = self.last_terminal_width; // borderless — full width
        let total = Self::total_content_lines(&self.messages, inner_width);
        let max = total.saturating_sub(self.last_viewport_height);
        self.scroll_top = self.scroll_top.min(max);
    }

    // ── Format helpers ──────────────────────────────────────────────────────

    fn read_git_branch() -> Option<String> {
        std::process::Command::new("git")
            .args(["rev-parse", "--abbrev-ref", "HEAD"])
            .output()
            .ok()
            .filter(|o| o.status.success())
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty() && s != "HEAD")
    }

    fn format_tokens(n: u64) -> String {
        if n >= 1_000_000 {
            format!("{:.1}M", n as f64 / 1_000_000.0)
        } else if n >= 1_000 {
            format!("{:.1}k", n as f64 / 1_000.0)
        } else {
            n.to_string()
        }
    }

    fn format_cost(usd: f64) -> String {
        if usd >= 1.0 {
            format!("${:.2}", usd)
        } else {
            format!("${:.4}", usd)
        }
    }

    // ── Layout helpers ──────────────────────────────────────────────────────

    /// Compute a centered rect of fixed height inside `r`, as wide as `percent_x` percent.
    fn centered_rect(percent_x: u16, height: u16, r: Rect) -> Rect {
        let vert = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(0),
                Constraint::Length(height),
                Constraint::Min(0),
            ])
            .split(r);

        Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage((100 - percent_x) / 2),
                Constraint::Percentage(percent_x),
                Constraint::Percentage((100 - percent_x) / 2),
            ])
            .split(vert[1])[1]
    }

    // ── Render ──────────────────────────────────────────────────────────────

    /// Number of lines the input box should occupy (1–6).
    fn input_display_height(&self) -> u16 {
        if self.is_thinking {
            return 1;
        }
        let newlines = self.input_buffer.chars().filter(|&c| c == '\n').count() as u16;
        (newlines + 1).min(6)
    }

    /// Render the TUI frame — CC-style borderless layout.
    fn render(&mut self, f: &mut ratatui::Frame) {
        let size = f.area();
        let width = size.width;
        let theme = get_theme();

        // Dynamic layout: add slash menu row if matches exist.
        let menu_rows = if self.completion_matches.is_empty() {
            0u16
        } else {
            (self.completion_matches.len().min(5) + 2) as u16
        };

        let input_height = self.input_display_height();

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),            // header
                Constraint::Min(0),               // messages
                Constraint::Length(menu_rows),    // slash menu (0 if empty)
                Constraint::Length(1),            // divider
                Constraint::Length(input_height), // input prompt
            ])
            .split(size);

        let viewport_height = chunks[1].height;

        // Cache dimensions used by key/mouse handlers between frames.
        self.last_terminal_width = width;
        self.last_viewport_height = viewport_height;

        // ── Header ────────────────────────────────────────────────────────
        const SPINNER: &[char] = &['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];
        let model_label = self.model_id.as_deref().unwrap_or("claude");
        let branch_suffix = self
            .git_branch
            .as_deref()
            .map(|b| format!("  ({b})"))
            .unwrap_or_default();
        let name_suffix = self
            .session_name
            .as_deref()
            .map(|n| format!("  ·  {n}"))
            .unwrap_or_default();
        let header_left = format!("  sage  {model_label}{branch_suffix}{name_suffix}");
        let cache_suffix = if self.session_cache_read_tokens > 0 || self.session_cache_write_tokens > 0 {
            format!(
                "  R{}  W{}",
                Self::format_tokens(self.session_cache_read_tokens),
                Self::format_tokens(self.session_cache_write_tokens),
            )
        } else {
            String::new()
        };
        let (ctx_span, ctx_span_len) = if self.context_window > 0 {
            let total = self.session_input_tokens + self.session_output_tokens;
            let pct = (total * 100 / self.context_window as u64).min(100);
            let kw = self.context_window / 1000;
            let text = format!("  {pct}%/{kw}k");
            let len = text.width() as u16;
            let (color, bold) = match pct {
                0..=49 => (theme.ratatui_fg(ThemeColor::Muted), false),
                50..=79 => (theme.ratatui_fg(ThemeColor::Warning), false),
                _ => (theme.ratatui_fg(ThemeColor::Error), true),
            };
            let style = if bold {
                Style::default().fg(color).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(color)
            };
            (Some(Span::styled(text, style)), len)
        } else {
            (None, 0u16)
        };
        let stats = format!(
            "↑{}  ↓{}{}  {}  ",
            Self::format_tokens(self.session_input_tokens),
            Self::format_tokens(self.session_output_tokens),
            cache_suffix,
            Self::format_cost(self.session_cost_usd),
        );
        let left_len = header_left.width().min(u16::MAX as usize) as u16;
        let stats_len = stats.width().min(u16::MAX as usize) as u16;

        // Permission mode badge (right of model label)
        let (mode_badge_span, badge_len) = match self.permission_mode.as_str() {
            "bypassPermissions" => {
                let label = "  ⚡ BYPASS".to_string();
                let len = label.width() as u16;
                (
                    Some(Span::styled(
                        label,
                        Style::default()
                            .fg(theme.ratatui_fg(ThemeColor::Error))
                            .add_modifier(Modifier::BOLD),
                    )),
                    len,
                )
            }
            "plan" => {
                let label = "  📋 PLAN".to_string();
                let len = label.width() as u16;
                (
                    Some(Span::styled(
                        label,
                        Style::default()
                            .fg(theme.ratatui_fg(ThemeColor::Accent))
                            .add_modifier(Modifier::BOLD),
                    )),
                    len,
                )
            }
            _ => (None, 0u16),
        };

        // Thinking level badge (right of permission badge, only when not Off)
        let (think_badge_span, think_badge_len) = if self.thinking_level != ThinkingLevel::Off {
            let label = format!("  🧠 {}", self.thinking_level.as_str());
            let len = label.width() as u16;
            let color = theme.ratatui_fg(theme.thinking_border_color(self.thinking_level));
            (
                Some(Span::styled(
                    label,
                    Style::default().fg(color).add_modifier(Modifier::BOLD),
                )),
                len,
            )
        } else {
            (None, 0u16)
        };

        let (tool_span, tool_len) = if let Some(name) = &self.current_tool {
            let label = format!("⚙ {name}  ");
            let len = label.width() as u16;
            (
                Some(Span::styled(
                    label,
                    Style::default().fg(theme.ratatui_fg(ThemeColor::Warning)),
                )),
                len,
            )
        } else {
            (None, 0u16)
        };
        let header_line = if self.is_thinking {
            let frame = SPINNER[(self.tick as usize) % SPINNER.len()];
            let thinking_text = format!("{frame} Thinking…  ");
            let thinking_len = thinking_text.width() as u16;
            let gap = width.saturating_sub(left_len + badge_len + think_badge_len + thinking_len + tool_len + ctx_span_len + stats_len);
            let mut spans = vec![
                Span::styled(
                    "  sage  ".to_string(),
                    Style::default()
                        .fg(theme.ratatui_fg(ThemeColor::Accent))
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    model_label.to_string(),
                    Style::default().fg(theme.ratatui_fg(ThemeColor::Muted)),
                ),
            ];
            if !branch_suffix.is_empty() {
                spans.push(Span::styled(
                    branch_suffix.clone(),
                    Style::default().fg(theme.ratatui_fg(ThemeColor::Dim)),
                ));
            }
            if let Some(bs) = mode_badge_span {
                spans.push(bs);
            }
            if let Some(tbs) = think_badge_span {
                spans.push(tbs);
            }
            spans.push(Span::raw(" ".repeat(gap as usize)));
            spans.push(Span::styled(
                thinking_text,
                Style::default().fg(theme.ratatui_fg(ThemeColor::Accent)),
            ));
            if let Some(ts) = tool_span {
                spans.push(ts);
            }
            if let Some(cs) = ctx_span {
                spans.push(cs);
            }
            spans.push(Span::styled(
                stats,
                Style::default().fg(theme.ratatui_fg(ThemeColor::Muted)),
            ));
            Line::from(spans)
        } else {
            let gap = width.saturating_sub(left_len + badge_len + think_badge_len + tool_len + ctx_span_len + stats_len);
            let mut spans = vec![
                Span::styled(
                    "  sage  ".to_string(),
                    Style::default()
                        .fg(theme.ratatui_fg(ThemeColor::Accent))
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    model_label.to_string(),
                    Style::default().fg(theme.ratatui_fg(ThemeColor::Muted)),
                ),
            ];
            if !branch_suffix.is_empty() {
                spans.push(Span::styled(
                    branch_suffix,
                    Style::default().fg(theme.ratatui_fg(ThemeColor::Dim)),
                ));
            }
            if let Some(bs) = mode_badge_span {
                spans.push(bs);
            }
            if let Some(tbs) = think_badge_span {
                spans.push(tbs);
            }
            spans.push(Span::raw(" ".repeat(gap as usize)));
            if let Some(ts) = tool_span {
                spans.push(ts);
            }
            if let Some(cs) = ctx_span {
                spans.push(cs);
            }
            spans.push(Span::styled(
                stats,
                Style::default().fg(theme.ratatui_fg(ThemeColor::Muted)),
            ));
            Line::from(spans)
        };
        f.render_widget(Paragraph::new(vec![header_line]), chunks[0]);

        // ── Messages ──────────────────────────────────────────────────────
        let mut message_lines: Vec<Line> = self
            .messages
            .iter()
            .flat_map(|m| Self::message_to_lines(m, &theme, width))
            .collect();

        // Empty state: welcome prompt when no conversation has started.
        if message_lines.is_empty() && !self.is_thinking {
            message_lines = vec![
                Line::from(""),
                Line::from(vec![
                    Span::styled(
                        "  ◆ ",
                        Style::default().fg(theme.ratatui_fg(ThemeColor::Accent)),
                    ),
                    Span::styled(
                        "What can I help you with?",
                        Style::default().fg(theme.ratatui_fg(ThemeColor::Muted)),
                    ),
                ]),
                Line::from(""),
                Line::from(vec![
                    Span::raw("    "),
                    Span::styled(
                        "Tip: ",
                        Style::default().fg(theme.ratatui_fg(ThemeColor::Accent)),
                    ),
                    Span::styled(
                        "Type / for commands, @ to reference files, Esc to cancel",
                        Style::default().fg(theme.ratatui_fg(ThemeColor::Dim)),
                    ),
                ]),
            ];
        }

        let total_lines = Self::total_content_lines(&self.messages, width);
        let messages_widget = Paragraph::new(message_lines)
            .wrap(Wrap { trim: false })
            .scroll((self.scroll_top, 0));
        f.render_widget(messages_widget, chunks[1]);

        // ── Completion menu (slash commands or @ files) ───────────────────
        let is_slash_mode = self.input_buffer.starts_with('/');
        if menu_rows > 0 {
            let visible: Vec<_> = self.completion_matches.iter().take(5).collect();
            let menu_lines: Vec<Line> = visible
                .iter()
                .enumerate()
                .map(|(i, (primary, hint))| {
                    let selected = i == self.completion_selected;
                    let bg = if selected {
                        theme.ratatui_bg(ThemeBg::SelectedBg)
                    } else {
                        Color::Reset
                    };
                    let label = if is_slash_mode {
                        format!("  /{primary:<12}")
                    } else {
                        format!("  {primary:<14}")
                    };
                    Line::from(vec![
                        Span::styled(
                            label,
                            Style::default()
                                .fg(theme.ratatui_fg(ThemeColor::Accent))
                                .bg(bg),
                        ),
                        Span::styled(
                            format!("  {hint}"),
                            Style::default()
                                .fg(theme.ratatui_fg(ThemeColor::Muted))
                                .bg(bg),
                        ),
                    ])
                })
                .collect();
            let menu_block = Block::default()
                .borders(Borders::TOP | Borders::BOTTOM)
                .border_style(Style::default().fg(theme.ratatui_fg(ThemeColor::BorderMuted)));
            f.render_widget(Paragraph::new(menu_lines).block(menu_block), chunks[2]);
        }

        // ── Divider — with scroll indicator when content overflows ────────
        let divider_str = if !self.is_sticky
            && total_lines > viewport_height
            && self.scroll_top < total_lines.saturating_sub(viewport_height)
        {
            let remaining = total_lines
                .saturating_sub(viewport_height)
                .saturating_sub(self.scroll_top);
            let suffix = format!(" ↓{remaining} ");
            let dash_count = (width as usize).saturating_sub(suffix.width());
            format!("{}{}", "─".repeat(dash_count), suffix)
        } else {
            "─".repeat(width as usize)
        };
        let divider_line = Line::from(Span::styled(
            divider_str,
            Style::default().fg(theme.ratatui_fg(ThemeColor::BorderMuted)),
        ));
        f.render_widget(Paragraph::new(vec![divider_line]), chunks[3]);

        // ── Input / spinner ───────────────────────────────────────────────
        let input_lines: Vec<Line> = if self.is_thinking {
            let frame = SPINNER[(self.tick as usize) % SPINNER.len()];
            vec![Line::from(vec![
                Span::styled(
                    format!("  {frame} "),
                    Style::default().fg(theme.ratatui_fg(ThemeColor::Accent)),
                ),
                Span::styled(
                    "Thinking…",
                    Style::default().fg(theme.ratatui_fg(ThemeColor::Muted)),
                ),
            ])]
        } else {
            let all_lines: Vec<&str> = self.input_buffer.split('\n').collect();
            let total = all_lines.len();
            let visible: Vec<&str> = if total > 6 {
                all_lines[total - 6..].to_vec()
            } else {
                all_lines
            };
            let visible_count = visible.len();
            // Cursor blinks every 10 ticks (~500ms).
            let show_cursor = (self.tick / 10) & 1 == 0;
            let cursor = if show_cursor { "▋" } else { " " };
            visible
                .into_iter()
                .enumerate()
                .map(|(i, text)| {
                    let is_last = i == visible_count - 1;
                    let cursor_span = if is_last {
                        Some(Span::styled(
                            cursor.to_string(),
                            Style::default().add_modifier(Modifier::REVERSED),
                        ))
                    } else {
                        None
                    };
                    if i == 0 {
                        let mut spans = vec![
                            Span::styled(
                                "  ❯ ",
                                Style::default().fg(theme.ratatui_fg(ThemeColor::Accent)),
                            ),
                            Span::raw(text.to_string()),
                        ];
                        spans.extend(cursor_span);
                        Line::from(spans)
                    } else {
                        let mut spans =
                            vec![Span::raw("    "), Span::raw(text.to_string())];
                        spans.extend(cursor_span);
                        Line::from(spans)
                    }
                })
                .collect()
        };
        f.render_widget(Paragraph::new(input_lines), chunks[4]);

        // ── Permission approval dialog ────────────────────────────────────
        if let Some(approval) = &self.pending_approval {
            let dialog_height = 8u16;
            let popup_area = Self::centered_rect(62, dialog_height, size);
            f.render_widget(Clear, popup_area);

            let tool_name = approval.tool_name.clone();
            let msg_preview: String = {
                // Flatten control chars inline — no intermediate String allocation.
                let mut chars = approval
                    .message
                    .chars()
                    .map(|c| if c.is_control() { ' ' } else { c });
                let mut s: String = chars.by_ref().take(60).collect();
                if chars.next().is_some() {
                    s.push('…');
                }
                s
            };

            let dialog_lines = vec![
                Line::from(""),
                Line::from(vec![
                    Span::styled(
                        "  Tool: ",
                        Style::default().fg(theme.ratatui_fg(ThemeColor::Muted)),
                    ),
                    Span::styled(
                        tool_name,
                        Style::default()
                            .fg(theme.ratatui_fg(ThemeColor::Accent))
                            .add_modifier(Modifier::BOLD),
                    ),
                ]),
                Line::from(vec![Span::styled(
                    format!("  {msg_preview}"),
                    Style::default().fg(theme.ratatui_fg(ThemeColor::Muted)),
                )]),
                Line::from(""),
                Line::from(vec![
                    Span::styled(
                        "  [y] Allow   ",
                        Style::default().fg(theme.ratatui_fg(ThemeColor::Success)),
                    ),
                    Span::styled(
                        "[Y] Always",
                        Style::default().fg(theme.ratatui_fg(ThemeColor::Success)),
                    ),
                ]),
                Line::from(vec![
                    Span::styled(
                        "  [n] Deny    ",
                        Style::default().fg(theme.ratatui_fg(ThemeColor::Error)),
                    ),
                    Span::styled(
                        "[N] Never",
                        Style::default().fg(theme.ratatui_fg(ThemeColor::Error)),
                    ),
                ]),
            ];

            let dialog = Paragraph::new(dialog_lines)
                .block(
                    Block::default()
                        .title(" ⚠ Permission Required ")
                        .title_alignment(Alignment::Center)
                        .borders(Borders::ALL)
                        .border_style(Style::default().fg(theme.ratatui_fg(ThemeColor::Warning))),
                )
                .wrap(Wrap { trim: false });
            f.render_widget(dialog, popup_area);
        }

        // ── Settings overlay ──────────────────────────────────────────────
        if self.settings_active {
            const SETTINGS_VISIBLE: usize = 15;
            let visible_count = SETTINGS_VISIBLE.min(self.settings_items.len());
            let overlay_h = (visible_count as u16 + 5).min(size.height);
            let overlay_area = Self::centered_rect(70, overlay_h, size);
            f.render_widget(Clear, overlay_area);

            let scroll = self.settings_scroll;
            let items_text: Vec<Line> = self.settings_items.iter().enumerate()
                .skip(scroll)
                .take(SETTINGS_VISIBLE)
                .map(|(i, item)| {
                    let selected = i == self.settings_selected;
                    if selected {
                        Line::from(Span::styled(
                            format!("  {:<24} {}", item.label, item.current_value),
                            Style::default()
                                .fg(theme.ratatui_fg(ThemeColor::Accent))
                                .add_modifier(Modifier::REVERSED),
                        ))
                    } else {
                        let mut spans = vec![
                            Span::styled(
                                format!("  {:<24} ", item.label),
                                Style::default().fg(theme.ratatui_fg(ThemeColor::Muted)),
                            ),
                            Span::styled(
                                item.current_value.clone(),
                                Style::default().fg(theme.ratatui_fg(ThemeColor::Accent)),
                            ),
                        ];
                        if !item.description.is_empty() {
                            let desc: String = item.description.chars().take(30).collect();
                            spans.push(Span::styled(
                                format!("  {desc}"),
                                Style::default().fg(theme.ratatui_fg(ThemeColor::Dim)),
                            ));
                        }
                        Line::from(spans)
                    }
                }).collect();

            let scroll_indicator = if self.settings_items.len() > SETTINGS_VISIBLE {
                format!(" Settings ({}/{}) ", self.settings_selected + 1, self.settings_items.len())
            } else {
                " Settings ".to_string()
            };

            let mut all_lines = items_text;
            all_lines.push(Line::from(""));
            all_lines.push(Line::from(vec![
                Span::styled(
                    "  ↑↓ select  ←→ change  Enter apply  Esc close",
                    Style::default().fg(theme.ratatui_fg(ThemeColor::Dim)),
                ),
            ]));

            let settings_widget = Paragraph::new(all_lines)
                .block(
                    Block::default()
                        .title(scroll_indicator)
                        .title_alignment(Alignment::Center)
                        .borders(Borders::ALL)
                        .border_style(Style::default().fg(theme.ratatui_fg(ThemeColor::Accent))),
                )
                .wrap(Wrap { trim: false });
            f.render_widget(settings_widget, overlay_area);
        }

        // ── Search bar ────────────────────────────────────────────────────
        if self.search_active {
            let bar_area = Rect {
                x: 0,
                y: size.height.saturating_sub(1),
                width: size.width,
                height: 1,
            };
            let match_info = if self.search_matches.is_empty() {
                if self.search_query.is_empty() {
                    String::new()
                } else {
                    "  (no matches)".to_string()
                }
            } else {
                format!("  [{}/{}]", self.search_idx + 1, self.search_matches.len())
            };
            let bar_line = Line::from(vec![
                Span::styled(
                    format!(" / {}", self.search_query),
                    Style::default()
                        .fg(theme.ratatui_fg(ThemeColor::Accent))
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    match_info,
                    Style::default().fg(theme.ratatui_fg(ThemeColor::Muted)),
                ),
                Span::styled(
                    "  [n/N jump  Esc close]",
                    Style::default().fg(theme.ratatui_fg(ThemeColor::Dim)),
                ),
            ]);
            f.render_widget(Clear, bar_area);
            f.render_widget(Paragraph::new(vec![bar_line]), bar_area);
        }
    }
}

// ── Syntax highlighting helpers ─────────────────────────────────────────────

fn syntax_set() -> &'static SyntaxSet {
    static SS: OnceLock<SyntaxSet> = OnceLock::new();
    SS.get_or_init(SyntaxSet::load_defaults_newlines)
}

fn find_syntax_for_lang<'a>(ss: &'a SyntaxSet, lang: &str) -> Option<&'a SyntaxReference> {
    if lang.is_empty() {
        return None;
    }
    ss.find_syntax_by_extension(lang).or_else(|| {
        let ext = match lang {
            "rust" => "rs",
            "python" | "py3" => "py",
            "javascript" | "node" | "ecmascript" => "js",
            "typescript" => "ts",
            "bash" | "shell" | "zsh" => "sh",
            "cpp" | "c++" => "cpp",
            "csharp" | "c#" => "cs",
            "golang" => "go",
            _ => return ss.find_syntax_by_name(lang),
        };
        ss.find_syntax_by_extension(ext)
    })
}

fn scope_to_ratatui_color(stack: &ScopeStack, theme: &Theme) -> Color {
    for scope in stack.as_slice().iter().rev() {
        let name = scope.build_string();
        let tc = if name.starts_with("comment") {
            ThemeColor::SyntaxComment
        } else if name.starts_with("keyword.operator") {
            ThemeColor::SyntaxOperator
        } else if name.starts_with("keyword")
            || name.starts_with("storage.type")
            || name.starts_with("storage.modifier")
        {
            ThemeColor::SyntaxKeyword
        } else if name.starts_with("entity.name.function") {
            ThemeColor::SyntaxFunction
        } else if name.starts_with("entity.name.type")
            || name.starts_with("support.type")
            || name.starts_with("support.class")
        {
            ThemeColor::SyntaxType
        } else if name.starts_with("string") {
            ThemeColor::SyntaxString
        } else if name.starts_with("constant.numeric") {
            ThemeColor::SyntaxNumber
        } else if name.starts_with("variable") {
            ThemeColor::SyntaxVariable
        } else if name.starts_with("punctuation") {
            ThemeColor::SyntaxPunctuation
        } else {
            continue;
        };
        return theme.ratatui_fg(tc);
    }
    theme.ratatui_fg(ThemeColor::MdCodeBlock)
}

fn highlight_code_line(
    line: &str,
    parse_state: &mut ParseState,
    ss: &SyntaxSet,
    theme: &Theme,
) -> Vec<Span<'static>> {
    let Ok(ops) = parse_state.parse_line(line, ss) else {
        return vec![Span::styled(
            line.to_string(),
            Style::default().fg(theme.ratatui_fg(ThemeColor::MdCodeBlock)),
        )];
    };
    let mut stack = ScopeStack::new();
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut last_pos = 0usize;
    for (byte_offset, op) in &ops {
        let byte_offset = *byte_offset;
        if byte_offset > last_pos {
            let text = line[last_pos..byte_offset].to_string();
            if !text.is_empty() {
                let color = scope_to_ratatui_color(&stack, theme);
                spans.push(Span::styled(text, Style::default().fg(color)));
            }
        }
        let _ = stack.apply(op);
        last_pos = byte_offset;
    }
    if last_pos < line.len() {
        let text = line[last_pos..].to_string();
        if !text.is_empty() {
            let color = scope_to_ratatui_color(&stack, theme);
            spans.push(Span::styled(text, Style::default().fg(color)));
        }
    }
    if spans.is_empty() {
        spans.push(Span::styled(
            line.to_string(),
            Style::default().fg(theme.ratatui_fg(ThemeColor::MdCodeBlock)),
        ));
    }
    spans
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_interactive_mode_empty_state() {
        let mode = InteractiveMode::new(InteractiveModeOptions::default());
        assert!(mode.input_buffer.is_empty());
        assert!(mode.messages.is_empty());
        assert!(!mode.running);
    }

    #[tokio::test]
    async fn init_with_initial_message() {
        let mut mode = InteractiveMode::new(InteractiveModeOptions {
            initial_message: Some("Hello".to_string()),
            ..Default::default()
        });
        mode.init().await.unwrap();
        assert_eq!(mode.messages.len(), 1);
        assert_eq!(mode.messages[0].role, MessageRole::User);
        assert_eq!(mode.messages[0].content, "Hello");
    }

    #[tokio::test]
    async fn init_without_initial_message() {
        let mut mode = InteractiveMode::new(InteractiveModeOptions::default());
        mode.init().await.unwrap();
        assert!(mode.messages.is_empty());
    }

    #[test]
    fn stop_sets_running_false() {
        let mut mode = InteractiveMode::new(InteractiveModeOptions::default());
        mode.running = true;
        mode.stop();
        assert!(!mode.running);
    }

    #[test]
    fn message_role_equality() {
        assert_eq!(MessageRole::User, MessageRole::User);
        assert_ne!(MessageRole::User, MessageRole::Assistant);
    }

    #[test]
    fn chat_message_clone() {
        let msg = ChatMessage {
            role: MessageRole::Assistant,
            content: "Hello world".to_string(),
            at_refs: Vec::new(),
                    full_output: None,
                    full_output_expanded: false,
        };
        let cloned = msg.clone();
        assert_eq!(cloned.content, "Hello world");
    }

    #[test]
    fn options_default() {
        let opts = InteractiveModeOptions::default();
        assert!(opts.initial_message.is_none());
        assert!(!opts.verbose);
        assert!(opts.model_fallback_message.is_none());
        assert!(opts.migrated_providers.is_empty());
    }

    #[test]
    fn scroll_state_defaults() {
        let mode = InteractiveMode::new(InteractiveModeOptions::default());
        assert_eq!(mode.scroll_top, 0);
        assert!(mode.is_sticky);
    }

    #[test]
    fn compute_message_height_single_line() {
        let msg = ChatMessage {
            role: MessageRole::User,
            content: "short".to_string(),
            at_refs: Vec::new(),
                    full_output: None,
                    full_output_expanded: false,
        };
        // "  ❯ short" fits in 80 cols → 1 row
        assert_eq!(InteractiveMode::compute_message_height(&msg, 80), 1);
    }

    #[test]
    fn compute_message_height_multiline_content() {
        let msg = ChatMessage {
            role: MessageRole::Assistant,
            content: "line one\nline two\nline three".to_string(),
            at_refs: Vec::new(),
                    full_output: None,
                    full_output_expanded: false,
        };
        // 3 content lines → 3 display rows (each fits in wide terminal)
        assert_eq!(InteractiveMode::compute_message_height(&msg, 80), 3);
    }

    #[test]
    fn compute_message_height_wrapping() {
        let msg = ChatMessage {
            role: MessageRole::User,
            content: "a".repeat(100),
            at_refs: Vec::new(),
                    full_output: None,
                    full_output_expanded: false,
        };
        // first_capacity=16, overflow=84 chars at full width 20: 1+ceil(84/20)=6
        let h = InteractiveMode::compute_message_height(&msg, 20);
        assert_eq!(h, 6);
    }

    #[test]
    fn compute_message_height_with_at_refs() {
        let msg = ChatMessage {
            role: MessageRole::User,
            content: "@Cargo.toml explain this".to_string(),
            at_refs: vec![AtRef {
                path: "Cargo.toml".to_string(),
                line_count: 42,
            }],
            full_output: None,
            full_output_expanded: false,
        };
        // 1 content line + 1 annotation = 2 rows
        assert_eq!(InteractiveMode::compute_message_height(&msg, 80), 2);
    }

    #[test]
    fn total_content_lines_sums_messages() {
        let msgs = vec![
            ChatMessage {
                role: MessageRole::User,
                content: "hi".to_string(),
                at_refs: Vec::new(),
                    full_output: None,
                    full_output_expanded: false,
            },
            ChatMessage {
                role: MessageRole::Assistant,
                content: "hello".to_string(),
                at_refs: Vec::new(),
                    full_output: None,
                    full_output_expanded: false,
            },
        ];
        let total = InteractiveMode::total_content_lines(&msgs, 80);
        assert_eq!(total, 2);
    }

    #[test]
    fn message_to_lines_empty_content() {
        let theme = theme::dark_theme(theme::ColorMode::Truecolor);
        let msg = ChatMessage {
            role: MessageRole::User,
            content: String::new(),
            at_refs: Vec::new(),
                    full_output: None,
                    full_output_expanded: false,
        };
        let lines = InteractiveMode::message_to_lines(&msg, &theme, 80);
        assert_eq!(lines.len(), 1);
    }

    #[test]
    fn message_to_lines_multiline() {
        let theme = theme::dark_theme(theme::ColorMode::Truecolor);
        let msg = ChatMessage {
            role: MessageRole::Assistant,
            content: "first\nsecond\nthird".to_string(),
            at_refs: Vec::new(),
                    full_output: None,
                    full_output_expanded: false,
        };
        let lines = InteractiveMode::message_to_lines(&msg, &theme, 80);
        assert_eq!(lines.len(), 3);
    }

    #[test]
    fn message_to_lines_with_at_refs_shows_annotations() {
        let theme = theme::dark_theme(theme::ColorMode::Truecolor);
        let msg = ChatMessage {
            role: MessageRole::User,
            content: "@Cargo.toml explain this".to_string(),
            at_refs: vec![AtRef {
                path: "Cargo.toml".to_string(),
                line_count: 42,
            }],
            full_output: None,
            full_output_expanded: false,
        };
        let lines = InteractiveMode::message_to_lines(&msg, &theme, 80);
        // 1 content line + 1 annotation line
        assert_eq!(lines.len(), 2);
        let annotation_text: String = lines[1].spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(annotation_text.contains("📎"));
        assert!(annotation_text.contains("Cargo.toml"));
        assert!(annotation_text.contains("42 lines"));
    }

    #[test]
    fn clamp_scroll_does_not_exceed_max() {
        let mut mode = InteractiveMode::new(InteractiveModeOptions::default());
        mode.messages.push(ChatMessage {
            role: MessageRole::User,
            content: "hello".to_string(),
            at_refs: Vec::new(),
                    full_output: None,
                    full_output_expanded: false,
        });
        mode.last_terminal_width = 80;
        mode.last_viewport_height = 24;
        mode.scroll_top = 9999;
        mode.clamp_scroll();
        // 1 message = 1 line; max scroll = 1 - 24 = 0 (saturating)
        assert_eq!(mode.scroll_top, 0);
    }

    #[test]
    fn update_completion_matches_finds_commands() {
        let mut mode = InteractiveMode::new(InteractiveModeOptions::default());
        mode.input_buffer = "/co".to_string();
        mode.update_completion_matches();
        // "compact" and "copy" start with "co"
        assert!(!mode.completion_matches.is_empty());
        assert!(mode.completion_matches.iter().any(|(n, _)| *n == "compact"));
        assert!(mode.completion_matches.iter().any(|(n, _)| *n == "copy"));
    }

    #[test]
    fn update_completion_matches_clears_when_no_slash() {
        let mut mode = InteractiveMode::new(InteractiveModeOptions::default());
        mode.input_buffer = "hello".to_string();
        mode.update_completion_matches();
        assert!(mode.completion_matches.is_empty());
    }

    #[test]
    fn parse_inline_markdown_bold() {
        let theme = theme::dark_theme(theme::ColorMode::Truecolor);
        let spans = InteractiveMode::parse_inline_markdown("hello **world** end", &theme);
        // Expect: "hello ", styled "world", " end"
        assert_eq!(spans.len(), 3);
        assert_eq!(spans[0].content, "hello ");
        assert_eq!(spans[1].content, "world");
        assert!(
            spans[1]
                .style
                .add_modifier
                .contains(ratatui::style::Modifier::BOLD)
        );
        assert_eq!(spans[2].content, " end");
    }

    #[test]
    fn parse_inline_markdown_italic() {
        let theme = theme::dark_theme(theme::ColorMode::Truecolor);
        let spans = InteractiveMode::parse_inline_markdown("*italic*", &theme);
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].content, "italic");
        assert!(
            spans[0]
                .style
                .add_modifier
                .contains(ratatui::style::Modifier::ITALIC)
        );
    }

    #[test]
    fn parse_inline_markdown_code() {
        let theme = theme::dark_theme(theme::ColorMode::Truecolor);
        let spans = InteractiveMode::parse_inline_markdown("use `foo()` here", &theme);
        assert_eq!(spans.len(), 3);
        assert_eq!(spans[1].content, "foo()");
        // Code span should have a background color set
        assert_ne!(spans[1].style.bg, None);
    }

    #[test]
    fn parse_inline_markdown_plain_text() {
        let theme = theme::dark_theme(theme::ColorMode::Truecolor);
        let spans = InteractiveMode::parse_inline_markdown("plain text", &theme);
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].content, "plain text");
    }

    #[test]
    fn message_to_lines_assistant_bold() {
        let theme = theme::dark_theme(theme::ColorMode::Truecolor);
        let msg = ChatMessage {
            role: MessageRole::Assistant,
            content: "**bold** text".to_string(),
            at_refs: Vec::new(),
                    full_output: None,
                    full_output_expanded: false,
        };
        let lines = InteractiveMode::message_to_lines(&msg, &theme, 80);
        assert_eq!(lines.len(), 1);
        // Line has: indicator span + bold span + rest span
        let spans = &lines[0].spans;
        assert!(spans.len() >= 2);
        // Find the bold span
        let bold_span = spans.iter().find(|s| s.content == "bold");
        assert!(bold_span.is_some());
        assert!(
            bold_span
                .unwrap()
                .style
                .add_modifier
                .contains(ratatui::style::Modifier::BOLD)
        );
    }

    #[test]
    fn message_to_lines_assistant_unordered_list() {
        let theme = theme::dark_theme(theme::ColorMode::Truecolor);
        let msg = ChatMessage {
            role: MessageRole::Assistant,
            content: "- item one".to_string(),
            at_refs: Vec::new(),
                    full_output: None,
                    full_output_expanded: false,
        };
        let lines = InteractiveMode::message_to_lines(&msg, &theme, 80);
        assert_eq!(lines.len(), 1);
        let all_text: String = lines[0].spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(all_text.contains('•'));
        assert!(!all_text.contains("- "));
    }

    #[test]
    fn message_to_lines_assistant_ordered_list() {
        let theme = theme::dark_theme(theme::ColorMode::Truecolor);
        let msg = ChatMessage {
            role: MessageRole::Assistant,
            content: "1. first item".to_string(),
            at_refs: Vec::new(),
                    full_output: None,
                    full_output_expanded: false,
        };
        let lines = InteractiveMode::message_to_lines(&msg, &theme, 80);
        assert_eq!(lines.len(), 1);
        let all_text: String = lines[0].spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(all_text.contains("1."));
        assert!(all_text.contains("first item"));
    }

    #[test]
    fn message_to_lines_assistant_heading1() {
        let theme = theme::dark_theme(theme::ColorMode::Truecolor);
        let msg = ChatMessage {
            role: MessageRole::Assistant,
            content: "# My Title".to_string(),
            at_refs: Vec::new(),
                    full_output: None,
                    full_output_expanded: false,
        };
        let lines = InteractiveMode::message_to_lines(&msg, &theme, 80);
        assert_eq!(lines.len(), 1);
        let all_text: String = lines[0].spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(all_text.contains("My Title"));
        // Heading span should be bold
        let heading_span = lines[0].spans.iter().find(|s| s.content.contains("My Title")).unwrap();
        assert!(heading_span.style.add_modifier.contains(ratatui::style::Modifier::BOLD));
    }

    #[test]
    fn message_to_lines_assistant_horizontal_rule() {
        let theme = theme::dark_theme(theme::ColorMode::Truecolor);
        let msg = ChatMessage {
            role: MessageRole::Assistant,
            content: "---".to_string(),
            at_refs: Vec::new(),
                    full_output: None,
                    full_output_expanded: false,
        };
        let lines = InteractiveMode::message_to_lines(&msg, &theme, 80);
        assert_eq!(lines.len(), 1);
        let all_text: String = lines[0].spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(all_text.contains('─'));
    }

    #[test]
    fn message_to_lines_assistant_blockquote() {
        let theme = theme::dark_theme(theme::ColorMode::Truecolor);
        let msg = ChatMessage {
            role: MessageRole::Assistant,
            content: "> quoted text".to_string(),
            at_refs: Vec::new(),
                    full_output: None,
                    full_output_expanded: false,
        };
        let lines = InteractiveMode::message_to_lines(&msg, &theme, 80);
        assert_eq!(lines.len(), 1);
        let all_text: String = lines[0].spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(all_text.contains('│'));
        assert!(all_text.contains("quoted text"));
    }

    #[test]
    fn message_to_lines_assistant_table_row() {
        let theme = theme::dark_theme(theme::ColorMode::Truecolor);
        let msg = ChatMessage {
            role: MessageRole::Assistant,
            content: "| Name | Value |".to_string(),
            at_refs: Vec::new(),
                    full_output: None,
                    full_output_expanded: false,
        };
        let lines = InteractiveMode::message_to_lines(&msg, &theme, 80);
        assert_eq!(lines.len(), 1);
        let all_text: String = lines[0].spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(all_text.contains("Name"));
        assert!(all_text.contains("Value"));
        assert!(all_text.contains('│'));
    }

    #[test]
    fn message_to_lines_assistant_table_separator_renders_as_rule() {
        let theme = theme::dark_theme(theme::ColorMode::Truecolor);
        let msg = ChatMessage {
            role: MessageRole::Assistant,
            content: "|---|---|".to_string(),
            at_refs: Vec::new(),
                    full_output: None,
                    full_output_expanded: false,
        };
        let lines = InteractiveMode::message_to_lines(&msg, &theme, 80);
        assert_eq!(lines.len(), 1);
        let all_text: String = lines[0].spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(all_text.contains('─'));
    }

    #[test]
    fn expand_at_refs_no_refs() {
        let (expanded, at_refs, warnings) =
            InteractiveMode::expand_at_refs("hello world, no refs here");
        assert_eq!(expanded, "hello world, no refs here");
        assert!(at_refs.is_empty());
        assert!(warnings.is_empty());
    }

    #[test]
    fn expand_at_refs_missing_file_produces_warning() {
        let (expanded, at_refs, warnings) =
            InteractiveMode::expand_at_refs("@nonexistent_file_xyz.txt explain");
        assert_eq!(expanded, "@nonexistent_file_xyz.txt explain");
        assert!(at_refs.is_empty());
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("nonexistent_file_xyz.txt"));
    }

    #[test]
    fn expand_at_refs_deduplicates_same_path() {
        let (_, at_refs, warnings) = InteractiveMode::expand_at_refs("@foo.txt and @foo.txt again");
        assert!(at_refs.is_empty());
        assert_eq!(warnings.len(), 1);
    }

    #[test]
    fn expand_at_refs_real_file() {
        use std::io::Write as _;
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        writeln!(tmp, "line one").unwrap();
        writeln!(tmp, "line two").unwrap();
        let path = tmp.path().to_str().unwrap().to_string();

        let input = format!("@{path} summarize");
        let (expanded, at_refs, warnings) = InteractiveMode::expand_at_refs(&input);

        assert!(warnings.is_empty());
        assert_eq!(at_refs.len(), 1);
        assert_eq!(at_refs[0].path, path);
        assert_eq!(at_refs[0].line_count, 2);
        assert!(expanded.contains("<file path="));
        assert!(expanded.contains("line one"));
        assert!(expanded.contains("line two"));
        assert!(expanded.ends_with(&input));
    }

    // ── History recall tests ─────────────────────────────────────────────────

    #[test]
    fn history_empty_on_new() {
        let mode = InteractiveMode::new(InteractiveModeOptions::default());
        assert!(mode.history.is_empty());
        assert!(mode.history_idx.is_none());
        assert!(mode.history_draft.is_empty());
    }

    #[test]
    fn history_up_on_empty_history_is_noop() {
        let mut mode = InteractiveMode::new(InteractiveModeOptions::default());
        mode.input_buffer = "draft".to_string();
        // Simulate pressing ↑ with no history — nothing should change.
        if mode.history.is_empty() {
            // noop branch
        } else if let Some(idx) = mode.history_idx {
            mode.history_idx = Some(idx.saturating_sub(1));
            mode.input_buffer = mode.history[idx.saturating_sub(1)].clone();
        } else {
            mode.history_draft = std::mem::take(&mut mode.input_buffer);
            let new_idx = mode.history.len() - 1;
            mode.history_idx = Some(new_idx);
            mode.input_buffer = mode.history[new_idx].clone();
        }
        assert_eq!(mode.input_buffer, "draft");
        assert!(mode.history_idx.is_none());
    }

    #[test]
    fn history_up_saves_draft_and_loads_latest() {
        let mut mode = InteractiveMode::new(InteractiveModeOptions::default());
        mode.history = vec!["first".to_string(), "second".to_string()];
        mode.input_buffer = "my draft".to_string();

        // First ↑ press
        mode.history_draft = std::mem::take(&mut mode.input_buffer);
        let new_idx = mode.history.len() - 1;
        mode.history_idx = Some(new_idx);
        mode.input_buffer = mode.history[new_idx].clone();

        assert_eq!(mode.history_draft, "my draft");
        assert_eq!(mode.history_idx, Some(1));
        assert_eq!(mode.input_buffer, "second");
    }

    #[test]
    fn history_up_navigates_backwards() {
        let mut mode = InteractiveMode::new(InteractiveModeOptions::default());
        mode.history = vec!["first".to_string(), "second".to_string()];
        mode.history_idx = Some(1);
        mode.input_buffer = "second".to_string();

        // Second ↑ press
        let idx = mode.history_idx.unwrap();
        let new_idx = idx.saturating_sub(1);
        mode.history_idx = Some(new_idx);
        mode.input_buffer = mode.history[new_idx].clone();

        assert_eq!(mode.history_idx, Some(0));
        assert_eq!(mode.input_buffer, "first");
    }

    #[test]
    fn history_up_saturates_at_oldest() {
        let mut mode = InteractiveMode::new(InteractiveModeOptions::default());
        mode.history = vec!["only".to_string()];
        mode.history_idx = Some(0);
        mode.input_buffer = "only".to_string();

        // ↑ again — saturates at 0
        let idx = mode.history_idx.unwrap();
        let new_idx = idx.saturating_sub(1);
        mode.history_idx = Some(new_idx);
        mode.input_buffer = mode.history[new_idx].clone();

        assert_eq!(mode.history_idx, Some(0));
        assert_eq!(mode.input_buffer, "only");
    }

    #[test]
    fn history_down_advances_and_restores_draft() {
        let mut mode = InteractiveMode::new(InteractiveModeOptions::default());
        mode.history = vec!["first".to_string(), "second".to_string()];
        mode.history_idx = Some(0);
        mode.history_draft = "saved draft".to_string();
        mode.input_buffer = "first".to_string();

        // ↓ from idx=0 → idx=1
        {
            let idx = mode.history_idx.unwrap();
            if idx + 1 < mode.history.len() {
                let new_idx = idx + 1;
                mode.history_idx = Some(new_idx);
                mode.input_buffer = mode.history[new_idx].clone();
            }
        }
        assert_eq!(mode.history_idx, Some(1));
        assert_eq!(mode.input_buffer, "second");

        // ↓ from idx=1 → restore draft
        {
            let idx = mode.history_idx.unwrap();
            if idx + 1 < mode.history.len() {
                let new_idx = idx + 1;
                mode.history_idx = Some(new_idx);
                mode.input_buffer = mode.history[new_idx].clone();
            } else {
                mode.history_idx = None;
                mode.input_buffer = std::mem::take(&mut mode.history_draft);
            }
        }
        assert!(mode.history_idx.is_none());
        assert_eq!(mode.input_buffer, "saved draft");
        assert!(mode.history_draft.is_empty());
    }

    #[test]
    fn history_down_noop_when_not_browsing() {
        let mut mode = InteractiveMode::new(InteractiveModeOptions::default());
        mode.input_buffer = "typing".to_string();
        // history_idx is None — ↓ should do nothing
        let was_none = mode.history_idx.is_none();
        assert!(was_none);
        assert_eq!(mode.input_buffer, "typing");
    }

    // ── Slash command dispatch tests ─────────────────────────────────────────

    #[test]
    fn permission_mode_defaults_to_default() {
        let mode = InteractiveMode::new(InteractiveModeOptions::default());
        assert_eq!(mode.permission_mode, "default");
    }

    #[test]
    fn slash_permissions_bypass() {
        let mut mode = InteractiveMode::new(InteractiveModeOptions::default());
        assert!(mode.handle_builtin_slash_command("/permissions", "bypass"));
        assert_eq!(mode.permission_mode, "bypassPermissions");
        assert_eq!(mode.messages.len(), 1);
        assert!(mode.messages[0].content.contains("BYPASS"));
    }

    #[test]
    fn slash_permissions_plan() {
        let mut mode = InteractiveMode::new(InteractiveModeOptions::default());
        assert!(mode.handle_builtin_slash_command("/permissions", "plan"));
        assert_eq!(mode.permission_mode, "plan");
        assert!(mode.messages[0].content.contains("PLAN"));
    }

    #[test]
    fn slash_permissions_default_restores() {
        let mut mode = InteractiveMode::new(InteractiveModeOptions::default());
        mode.permission_mode = "bypassPermissions".to_string();
        assert!(mode.handle_builtin_slash_command("/permissions", "default"));
        assert_eq!(mode.permission_mode, "default");
    }

    #[test]
    fn slash_permissions_unknown_arg_keeps_mode() {
        let mut mode = InteractiveMode::new(InteractiveModeOptions::default());
        mode.permission_mode = "plan".to_string();
        assert!(mode.handle_builtin_slash_command("/permissions", "superpower"));
        assert_eq!(mode.permission_mode, "plan");
        assert!(mode.messages[0].content.contains("Unknown permission mode"));
    }

    #[test]
    fn slash_thinking_no_arg_cycles() {
        let mut mode = InteractiveMode::new(InteractiveModeOptions::default());
        assert_eq!(mode.thinking_level, ThinkingLevel::Off);
        assert!(mode.handle_builtin_slash_command("/thinking", ""));
        assert_eq!(mode.thinking_level, ThinkingLevel::Low);
        assert!(mode.messages[0].content.contains("low"));
    }

    #[test]
    fn slash_thinking_explicit_level() {
        let mut mode = InteractiveMode::new(InteractiveModeOptions::default());
        assert!(mode.handle_builtin_slash_command("/thinking", "high"));
        assert_eq!(mode.thinking_level, ThinkingLevel::High);
        assert!(mode.messages[0].content.contains("high"));
    }

    #[test]
    fn slash_thinking_minimal() {
        let mut mode = InteractiveMode::new(InteractiveModeOptions::default());
        assert!(mode.handle_builtin_slash_command("/thinking", "minimal"));
        assert_eq!(mode.thinking_level, ThinkingLevel::Minimal);
        assert!(mode.messages[0].content.contains("minimal"));
    }

    #[test]
    fn slash_thinking_invalid_arg_leaves_level_and_shows_error() {
        let mut mode = InteractiveMode::new(InteractiveModeOptions::default());
        mode.thinking_level = ThinkingLevel::Medium;
        assert!(mode.handle_builtin_slash_command("/thinking", "turbo"));
        assert_eq!(mode.thinking_level, ThinkingLevel::Medium);
        let msg = &mode.messages[0].content;
        assert!(msg.contains("Unknown thinking level"));
        assert!(msg.contains("minimal"));
    }

    #[test]
    fn slash_compact_trims_display() {
        let mut mode = InteractiveMode::new(InteractiveModeOptions::default());
        for i in 0..30 {
            mode.messages.push(ChatMessage {
                role: MessageRole::User,
                content: format!("msg {i}"),
                at_refs: Vec::new(),
                    full_output: None,
                    full_output_expanded: false,
            });
        }
        assert!(mode.handle_builtin_slash_command("/compact", ""));
        // 20 kept + 1 system notice
        assert_eq!(mode.messages.len(), 21);
        assert!(mode.messages.last().unwrap().content.contains("✂"));
    }

    #[test]
    fn slash_compact_no_trim_when_few_messages() {
        let mut mode = InteractiveMode::new(InteractiveModeOptions::default());
        for i in 0..5 {
            mode.messages.push(ChatMessage {
                role: MessageRole::User,
                content: format!("msg {i}"),
                at_refs: Vec::new(),
                    full_output: None,
                    full_output_expanded: false,
            });
        }
        assert!(mode.handle_builtin_slash_command("/compact", ""));
        // All 5 kept + 1 notice
        assert_eq!(mode.messages.len(), 6);
    }

    #[test]
    fn compact_clamps_session_saved_up_to_so_turn_usage_does_not_panic() {
        let mut mode = InteractiveMode::new(InteractiveModeOptions::default());
        for i in 0..30 {
            mode.messages.push(ChatMessage {
                role: MessageRole::Assistant,
                content: format!("reply {i}"),
                at_refs: Vec::new(),
                    full_output: None,
                    full_output_expanded: false,
            });
        }
        // Simulate all 30 messages having been saved already.
        mode.session_assistant_saved_up_to = 30;
        assert!(mode.handle_builtin_slash_command("/compact", ""));
        // After compact: 20 kept + 1 system notice = 21 entries.
        // saved_up_to must not exceed messages.len() or the TurnUsage slice panics.
        assert!(
            mode.session_assistant_saved_up_to <= mode.messages.len(),
            "saved_up_to {} > messages.len() {}",
            mode.session_assistant_saved_up_to,
            mode.messages.len()
        );
    }

    #[test]
    fn abort_agent_clears_agent_rx_so_stale_deltas_cannot_pollute_new_session() {
        let mut mode = InteractiveMode::new(InteractiveModeOptions::default());
        // Give the mode a fake agent channel so agent_rx is Some.
        let (_tx, rx) = mpsc::unbounded_channel::<crate::print_session::AgentDelta>();
        mode.agent_rx = Some(rx);
        mode.abort_agent();
        assert!(
            mode.agent_rx.is_none(),
            "abort_agent must clear agent_rx to prevent stale delta pollution"
        );
        assert!(mode.current_tool.is_none());
    }

    #[test]
    fn slash_model_no_arg_shows_current() {
        let mut mode = InteractiveMode::new(InteractiveModeOptions::default());
        assert!(mode.handle_builtin_slash_command("/model", ""));
        assert!(mode.messages[0].content.contains("Current model:"));
    }

    #[test]
    fn slash_model_sets_model_id() {
        let mut mode = InteractiveMode::new(InteractiveModeOptions::default());
        assert!(mode.handle_builtin_slash_command("/model", "gpt-5"));
        assert_eq!(mode.model_id, Some("gpt-5".to_string()));
        assert!(mode.messages[0].content.contains("gpt-5"));
    }

    #[test]
    fn slash_unknown_returns_false() {
        let mut mode = InteractiveMode::new(InteractiveModeOptions::default());
        // "/thinkingfoo" must NOT be handled as a slash command
        assert!(!mode.handle_builtin_slash_command("/thinkingfoo", ""));
        assert!(mode.messages.is_empty());
    }

    #[test]
    fn slash_plain_message_returns_false() {
        let mut mode = InteractiveMode::new(InteractiveModeOptions::default());
        assert!(!mode.handle_builtin_slash_command("hello world", ""));
        assert!(mode.messages.is_empty());
    }

    // ── extract_message_text tests ───────────────────────────────────────────

    #[test]
    fn extract_message_text_string_content() {
        let v = serde_json::json!({"role": "user", "content": "hello world"});
        assert_eq!(InteractiveMode::extract_message_text(&v), "hello world");
    }

    #[test]
    fn extract_message_text_array_content() {
        let v = serde_json::json!({
            "role": "assistant",
            "content": [{"type": "text", "text": "part one"}, {"type": "text", "text": " part two"}]
        });
        assert_eq!(
            InteractiveMode::extract_message_text(&v),
            "part one part two"
        );
    }

    #[test]
    fn extract_message_text_array_skips_non_text_blocks() {
        let v = serde_json::json!({
            "content": [
                {"type": "tool_use", "id": "x"},
                {"type": "text", "text": "answer"}
            ]
        });
        assert_eq!(InteractiveMode::extract_message_text(&v), "answer");
    }

    #[test]
    fn extract_message_text_missing_content_returns_empty() {
        let v = serde_json::json!({"role": "user"});
        assert_eq!(InteractiveMode::extract_message_text(&v), "");
    }

    // ── load_session_entries_into_messages tests ─────────────────────────────

    #[test]
    fn load_session_entries_empty_input_produces_no_messages() {
        let replayed = InteractiveMode::load_session_entries_into_messages(&[]);
        assert!(replayed.messages.is_empty());
        assert!(replayed.thinking_level.is_none());
        assert!(replayed.model_id.is_none());
    }

    #[test]
    fn load_session_entries_loads_user_and_assistant() {
        use crate::core::session_manager::{SessionEntry, SessionMessageEntry};
        let entries = vec![
            SessionEntry::Message(SessionMessageEntry {
                entry_type: "message".to_string(),
                id: "1".to_string(),
                parent_id: None,
                timestamp: "2024-01-01T00:00:00Z".to_string(),
                message: serde_json::json!({"role": "user", "content": "hi"}),
            }),
            SessionEntry::Message(SessionMessageEntry {
                entry_type: "message".to_string(),
                id: "2".to_string(),
                parent_id: Some("1".to_string()),
                timestamp: "2024-01-01T00:00:01Z".to_string(),
                message: serde_json::json!({
                    "role": "assistant",
                    "content": [{"type": "text", "text": "hello"}]
                }),
            }),
        ];
        let replayed = InteractiveMode::load_session_entries_into_messages(&entries);
        assert_eq!(replayed.messages.len(), 2);
        assert_eq!(replayed.messages[0].role, MessageRole::User);
        assert_eq!(replayed.messages[0].content, "hi");
        assert_eq!(replayed.messages[1].role, MessageRole::Assistant);
        assert_eq!(replayed.messages[1].content, "hello");
    }

    #[test]
    fn load_session_entries_skips_empty_content() {
        use crate::core::session_manager::{SessionEntry, SessionMessageEntry};
        let entries = vec![SessionEntry::Message(SessionMessageEntry {
            entry_type: "message".to_string(),
            id: "1".to_string(),
            parent_id: None,
            timestamp: "2024-01-01T00:00:00Z".to_string(),
            message: serde_json::json!({"role": "user", "content": ""}),
        })];
        let replayed = InteractiveMode::load_session_entries_into_messages(&entries);
        assert!(replayed.messages.is_empty());
    }

    #[test]
    fn load_session_entries_replays_thinking_level_and_model_change() {
        use crate::core::session_manager::{
            ModelChangeEntry, SessionEntry, ThinkingLevelChangeEntry,
        };
        let entries = vec![
            SessionEntry::ThinkingLevelChange(ThinkingLevelChangeEntry {
                entry_type: "thinking_level_change".to_string(),
                id: "1".to_string(),
                parent_id: None,
                timestamp: "2024-01-01T00:00:00Z".to_string(),
                thinking_level: "high".to_string(),
            }),
            SessionEntry::ModelChange(ModelChangeEntry {
                entry_type: "model_change".to_string(),
                id: "2".to_string(),
                parent_id: Some("1".to_string()),
                timestamp: "2024-01-01T00:00:01Z".to_string(),
                provider: "anthropic".to_string(),
                model_id: "claude-opus-4-7".to_string(),
            }),
        ];
        let replayed = InteractiveMode::load_session_entries_into_messages(&entries);
        assert!(replayed.messages.is_empty());
        assert_eq!(replayed.thinking_level, Some(ThinkingLevel::High));
        assert_eq!(replayed.model_id.as_deref(), Some("claude-opus-4-7"));
        assert_eq!(replayed.provider_id.as_deref(), Some("anthropic"));
    }

    // ── /settings set tests ──────────────────────────────────────────────────

    #[test]
    fn slash_settings_set_model_updates_model_id() {
        let mut mode = InteractiveMode::new(InteractiveModeOptions::default());
        assert!(mode.handle_builtin_slash_command(
            "/settings",
            "set default_model claude-opus-4-7"
        ));
        assert_eq!(mode.model_id, Some("claude-opus-4-7".to_string()));
        assert!(mode.messages[0].content.contains("claude-opus-4-7"));
    }

    #[test]
    fn slash_settings_set_provider_updates_provider_id() {
        let mut mode = InteractiveMode::new(InteractiveModeOptions::default());
        assert!(mode.handle_builtin_slash_command(
            "/settings",
            "set default_provider anthropic"
        ));
        assert_eq!(mode.provider_id, Some("anthropic".to_string()));
    }

    #[test]
    fn slash_settings_set_unknown_key_shows_error() {
        let mut mode = InteractiveMode::new(InteractiveModeOptions::default());
        assert!(mode.handle_builtin_slash_command("/settings", "set badkey value"));
        assert!(mode.messages[0].content.contains("Unknown setting"));
    }

    #[test]
    fn slash_settings_set_missing_value_shows_usage() {
        let mut mode = InteractiveMode::new(InteractiveModeOptions::default());
        assert!(mode.handle_builtin_slash_command("/settings", "set default_model"));
        assert!(mode.messages[0].content.contains("Usage"));
    }

    // ── /resume boundary tests ───────────────────────────────────────────────

    #[test]
    fn slash_resume_no_arg_shows_list_header() {
        let mut mode = InteractiveMode::new(InteractiveModeOptions::default());
        assert!(mode.handle_builtin_slash_command("/resume", ""));
        // With no saved sessions the message should say "No saved sessions" or list header
        assert!(!mode.messages.is_empty());
    }

    #[test]
    fn slash_resume_zero_is_invalid() {
        let mut mode = InteractiveMode::new(InteractiveModeOptions::default());
        assert!(mode.handle_builtin_slash_command("/resume", "0"));
        assert!(
            mode.messages[0].content.contains("Invalid")
                || mode.messages[0].content.contains("invalid")
        );
    }

    #[test]
    fn slash_resume_out_of_bounds_shows_error() {
        let mut mode = InteractiveMode::new(InteractiveModeOptions::default());
        assert!(mode.handle_builtin_slash_command("/resume", "9999"));
        // Either "No session #9999" or show list — not a panic
        assert!(!mode.messages.is_empty());
    }
}
