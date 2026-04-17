// Agent — Phase 4
// Agent struct with state management, steering/follow-up queues, and hooks.

use crate::compaction::{CompactionSettings, FileOperations};
use crate::llm::LlmProvider;
use crate::llm::types::*;
use crate::tools::ToolRegistry;
use crate::tools::policy::ToolPolicy;
use crate::types::*;
use std::collections::VecDeque;

/// Configuration for the agent loop.
#[derive(Debug, Clone)]
pub struct AgentLoopConfig {
    /// Human-readable agent name — forwarded to [`StopContext`] on session end.
    pub name: String,
    pub model: Model,
    pub system_prompt: String,
    pub max_turns: usize,
    pub tool_execution_mode: ToolExecutionMode,
    /// Optional tool policy for enforcing binary/path whitelists.
    /// When None, all tool calls are allowed (unrestricted mode).
    pub tool_policy: Option<ToolPolicy>,
    /// Compaction settings for context window management.
    pub compaction: CompactionSettings,
}

/// Hook called before a tool is executed.
#[async_trait::async_trait]
pub trait BeforeToolCallHook: Send + Sync {
    async fn before_tool_call(&self, ctx: &BeforeToolCallContext) -> BeforeToolCallResult;
}

/// Hook called after a tool is executed.
#[async_trait::async_trait]
pub trait AfterToolCallHook: Send + Sync {
    async fn after_tool_call(&self, ctx: &AfterToolCallContext) -> AfterToolCallResult;
}

/// Hook called before each LLM call to transform the message history.
///
/// Use this to inject memory, filter sensitive content, or apply custom compression.
/// The hook receives a mutable reference to the agent's message slice and may
/// add, remove, or modify messages in place.
///
/// # Example
/// ```rust,ignore
/// struct MemoryInjector { memories: Vec<String> }
///
/// #[async_trait::async_trait]
/// impl TransformContextHook for MemoryInjector {
///     async fn transform_context(&self, messages: &mut Vec<AgentMessage>) {
///         // Inject a memory recap as a system message before the last user turn
///         // ...
///     }
/// }
/// ```
#[async_trait::async_trait]
pub trait TransformContextHook: Send + Sync {
    async fn transform_context(&self, messages: &mut Vec<AgentMessage>);
}

/// Context passed to a [`StopHook`] when the agent is about to stop.
pub struct StopContext {
    /// The reason the agent is stopping.
    pub stop_reason: crate::types::StopReason,
    /// Unique session identifier (e.g., ULID).
    pub session_id: String,
    /// Unique task identifier within the session.
    pub task_id: String,
    /// Number of turns completed in this session.
    pub turn_count: usize,
    /// Name of the agent (from config).
    pub agent_name: String,
    /// Model identifier used for this session.
    pub model: String,
    /// The last assistant message produced before stopping.
    pub last_assistant_message: String,
}

/// The action a [`StopHook`] instructs the agent loop to take after the hook fires.
#[derive(Debug)]
pub enum StopAction {
    /// Accept the stop — the agent session ends normally.
    Pass,
    /// Inject `feedback` as a user message and restart the agent loop.
    /// Used by the Harness to give the agent another chance.
    Continue(String),
    /// Mark the harness run as failed with the given reason.
    Fail(String),
}

/// Hook called when the agent is about to stop.
///
/// Returning [`StopAction::Continue`] injects feedback and restarts the loop —
/// this is the core mechanism used by the Harness for evaluation-driven iteration.
#[async_trait::async_trait]
pub trait StopHook: Send + Sync {
    async fn on_stop(&self, ctx: &StopContext) -> StopAction;
}

/// The Agent — owns config, provider, tools, message history, queues, and hooks.
pub struct Agent {
    config: AgentLoopConfig,
    provider: Box<dyn LlmProvider>,
    tools: ToolRegistry,
    messages: Vec<AgentMessage>,
    streaming: bool,
    steering: VecDeque<AgentMessage>,
    follow_up_queue: VecDeque<AgentMessage>,
    before_hook: Option<Box<dyn BeforeToolCallHook>>,
    after_hook: Option<Box<dyn AfterToolCallHook>>,
    transform_context_hook: Option<Box<dyn TransformContextHook>>,
    stop_hook: Option<Box<dyn StopHook>>,
    /// Cumulative file operations tracked across compaction rounds.
    compaction_file_ops: FileOperations,
    /// Previous compaction summary (for iterative update prompt).
    previous_compaction_summary: Option<String>,
}

impl Agent {
    pub fn new(
        config: AgentLoopConfig,
        provider: Box<dyn LlmProvider>,
        tools: ToolRegistry,
    ) -> Self {
        Self {
            config,
            provider,
            tools,
            messages: Vec::new(),
            streaming: false,
            steering: VecDeque::new(),
            follow_up_queue: VecDeque::new(),
            before_hook: None,
            after_hook: None,
            transform_context_hook: None,
            stop_hook: None,
            compaction_file_ops: FileOperations::default(),
            previous_compaction_summary: None,
        }
    }

    pub fn config(&self) -> &AgentLoopConfig {
        &self.config
    }

    pub fn messages(&self) -> &[AgentMessage] {
        &self.messages
    }

    pub fn is_streaming(&self) -> bool {
        self.streaming
    }

    pub fn set_streaming(&mut self, v: bool) {
        self.streaming = v;
    }

    pub fn push_message(&mut self, msg: AgentMessage) {
        self.messages.push(msg);
    }

    pub fn provider(&self) -> &dyn LlmProvider {
        self.provider.as_ref()
    }

    pub fn tools(&self) -> &ToolRegistry {
        &self.tools
    }

    pub fn messages_mut(&mut self) -> &mut Vec<AgentMessage> {
        &mut self.messages
    }

    // -- compaction state --

    pub fn compaction_file_ops(&self) -> &FileOperations {
        &self.compaction_file_ops
    }

    pub fn compaction_file_ops_mut(&mut self) -> &mut FileOperations {
        &mut self.compaction_file_ops
    }

    pub fn previous_compaction_summary(&self) -> Option<&str> {
        self.previous_compaction_summary.as_deref()
    }

    pub fn set_previous_compaction_summary(&mut self, summary: Option<String>) {
        self.previous_compaction_summary = summary;
    }

    // -- steering queue --

    pub fn steer(&mut self, msg: AgentMessage) {
        self.steering.push_back(msg);
    }

    pub fn drain_steering(&mut self) -> Vec<AgentMessage> {
        self.steering.drain(..).collect()
    }

    // -- follow-up queue --

    pub fn follow_up(&mut self, msg: AgentMessage) {
        self.follow_up_queue.push_back(msg);
    }

    pub fn drain_follow_up(&mut self) -> Vec<AgentMessage> {
        self.follow_up_queue.drain(..).collect()
    }

    pub fn has_queued_messages(&self) -> bool {
        !self.steering.is_empty() || !self.follow_up_queue.is_empty()
    }

    // -- hooks --

    pub fn set_before_tool_call(&mut self, hook: Box<dyn BeforeToolCallHook>) {
        self.before_hook = Some(hook);
    }

    pub fn set_after_tool_call(&mut self, hook: Box<dyn AfterToolCallHook>) {
        self.after_hook = Some(hook);
    }

    pub fn set_transform_context(&mut self, hook: Box<dyn TransformContextHook>) {
        self.transform_context_hook = Some(hook);
    }

    /// Call the transform context hook if present; no-op if none is set.
    ///
    /// Uses `take()`/restore to sidestep the borrow checker:
    /// we cannot hold `&self.transform_context_hook` and `&mut self.messages` simultaneously.
    pub async fn call_transform_context(&mut self) {
        if let Some(hook) = self.transform_context_hook.take() {
            hook.transform_context(&mut self.messages).await;
            self.transform_context_hook = Some(hook);
        }
    }

    pub fn has_before_tool_call_hook(&self) -> bool {
        self.before_hook.is_some()
    }

    pub fn has_after_tool_call_hook(&self) -> bool {
        self.after_hook.is_some()
    }

    pub async fn call_before_tool_call(&self, ctx: &BeforeToolCallContext) -> BeforeToolCallResult {
        match &self.before_hook {
            Some(hook) => hook.before_tool_call(ctx).await,
            None => BeforeToolCallResult {
                block: false,
                reason: None,
            },
        }
    }

    pub async fn call_after_tool_call(&self, ctx: &AfterToolCallContext) -> AfterToolCallResult {
        match &self.after_hook {
            Some(hook) => hook.after_tool_call(ctx).await,
            None => AfterToolCallResult {
                content: None,
                is_error: None,
            },
        }
    }

    // -- stop hook --

    pub fn set_stop_hook(&mut self, hook: Box<dyn StopHook>) {
        self.stop_hook = Some(hook);
    }

    pub async fn call_stop_hook(&self, ctx: &StopContext) -> StopAction {
        match &self.stop_hook {
            Some(hook) => hook.on_stop(ctx).await,
            None => StopAction::Pass,
        }
    }
}

/// Convenience constructor for AssistantMessage.
impl AssistantMessage {
    pub fn from_text(text: &str) -> Self {
        Self::new(text.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::LlmProvider;
    use crate::llm::types::*;
    use crate::tools::{AgentTool, ToolOutput, ToolRegistry};
    use crate::types::*;
    use serde_json::json;

    // ---------------------------------------------------------------
    // Mock provider for testing
    // ---------------------------------------------------------------

    struct TestProvider;

    #[async_trait::async_trait]
    impl LlmProvider for TestProvider {
        async fn complete(
            &self,
            _model: &Model,
            _context: &LlmContext,
            _tools: &[LlmTool],
        ) -> Vec<AssistantMessageEvent> {
            vec![AssistantMessageEvent::Done {
                stop_reason: StopReason::Stop,
            }]
        }
    }

    use crate::test_helpers::test_model;

    fn test_config() -> AgentLoopConfig {
        AgentLoopConfig {
            name: "test-agent".into(),
            model: test_model(),
            system_prompt: "You are a test agent.".into(),
            max_turns: 10,
            tool_execution_mode: ToolExecutionMode::Parallel,
            tool_policy: None,
            compaction: CompactionSettings::default(),
        }
    }

    fn test_agent() -> Agent {
        Agent::new(test_config(), Box::new(TestProvider), ToolRegistry::new())
    }

    // ===============================================================
    // Agent construction & initial state
    // ===============================================================

    #[test]
    fn test_agent_new_has_empty_messages() {
        let agent = test_agent();
        assert!(agent.messages().is_empty());
    }

    #[test]
    fn test_agent_initial_state_not_streaming() {
        let agent = test_agent();
        assert!(!agent.is_streaming());
    }

    #[test]
    fn test_agent_config_model_id() {
        let agent = test_agent();
        assert_eq!(agent.config().model.id, "test-model");
    }

    #[test]
    fn test_agent_config_system_prompt() {
        let agent = test_agent();
        assert_eq!(agent.config().system_prompt, "You are a test agent.");
    }

    #[test]
    fn test_agent_config_max_turns() {
        let agent = test_agent();
        assert_eq!(agent.config().max_turns, 10);
    }

    #[test]
    fn test_agent_config_tool_execution_mode() {
        let agent = test_agent();
        assert_eq!(
            agent.config().tool_execution_mode,
            ToolExecutionMode::Parallel
        );
    }

    // ===============================================================
    // Steering queue
    // ===============================================================

    #[test]
    fn test_steer_adds_message() {
        let mut agent = test_agent();
        agent.steer(AgentMessage::User(UserMessage::from_text("hello")));
        assert!(agent.has_queued_messages());
    }

    #[test]
    fn test_steer_multiple_messages() {
        let mut agent = test_agent();
        agent.steer(AgentMessage::User(UserMessage::from_text("one")));
        agent.steer(AgentMessage::User(UserMessage::from_text("two")));
        let drained = agent.drain_steering();
        assert_eq!(drained.len(), 2);
    }

    #[test]
    fn test_drain_steering_returns_messages() {
        let mut agent = test_agent();
        agent.steer(AgentMessage::User(UserMessage::from_text("hello")));
        let drained = agent.drain_steering();
        assert_eq!(drained.len(), 1);
        match &drained[0] {
            AgentMessage::User(u) => {
                assert_eq!(
                    u.content[0],
                    Content::Text {
                        text: "hello".into()
                    }
                );
            }
            _ => panic!("expected User message"),
        }
    }

    #[test]
    fn test_drain_steering_clears_queue() {
        let mut agent = test_agent();
        agent.steer(AgentMessage::User(UserMessage::from_text("msg")));
        let _ = agent.drain_steering();
        assert!(!agent.has_queued_messages());
        let drained_again = agent.drain_steering();
        assert!(drained_again.is_empty());
    }

    #[test]
    fn test_drain_steering_empty_returns_empty() {
        let mut agent = test_agent();
        let drained = agent.drain_steering();
        assert!(drained.is_empty());
    }

    #[test]
    fn test_steer_preserves_order() {
        let mut agent = test_agent();
        agent.steer(AgentMessage::User(UserMessage::from_text("first")));
        agent.steer(AgentMessage::User(UserMessage::from_text("second")));
        agent.steer(AgentMessage::User(UserMessage::from_text("third")));
        let drained = agent.drain_steering();
        let texts: Vec<String> = drained
            .iter()
            .map(|m| match m {
                AgentMessage::User(u) => match &u.content[0] {
                    Content::Text { text } => text.clone(),
                    _ => String::new(),
                },
                _ => String::new(),
            })
            .collect();
        assert_eq!(texts, vec!["first", "second", "third"]);
    }

    // ===============================================================
    // Follow-up queue
    // ===============================================================

    #[test]
    fn test_follow_up_adds_message() {
        let mut agent = test_agent();
        agent.follow_up(AgentMessage::User(UserMessage::from_text("follow")));
        assert!(agent.has_queued_messages());
    }

    #[test]
    fn test_follow_up_multiple_messages() {
        let mut agent = test_agent();
        agent.follow_up(AgentMessage::User(UserMessage::from_text("a")));
        agent.follow_up(AgentMessage::User(UserMessage::from_text("b")));
        let drained = agent.drain_follow_up();
        assert_eq!(drained.len(), 2);
    }

    #[test]
    fn test_drain_follow_up_returns_messages() {
        let mut agent = test_agent();
        agent.follow_up(AgentMessage::User(UserMessage::from_text("fu")));
        let drained = agent.drain_follow_up();
        assert_eq!(drained.len(), 1);
    }

    #[test]
    fn test_drain_follow_up_clears_queue() {
        let mut agent = test_agent();
        agent.follow_up(AgentMessage::User(UserMessage::from_text("fu")));
        let _ = agent.drain_follow_up();
        let again = agent.drain_follow_up();
        assert!(again.is_empty());
    }

    #[test]
    fn test_drain_follow_up_empty_returns_empty() {
        let mut agent = test_agent();
        let drained = agent.drain_follow_up();
        assert!(drained.is_empty());
    }

    // ===============================================================
    // has_queued_messages
    // ===============================================================

    #[test]
    fn test_has_queued_messages_initially_false() {
        let agent = test_agent();
        assert!(!agent.has_queued_messages());
    }

    #[test]
    fn test_has_queued_messages_after_steer() {
        let mut agent = test_agent();
        agent.steer(AgentMessage::User(UserMessage::from_text("x")));
        assert!(agent.has_queued_messages());
    }

    #[test]
    fn test_has_queued_messages_after_follow_up() {
        let mut agent = test_agent();
        agent.follow_up(AgentMessage::User(UserMessage::from_text("x")));
        assert!(agent.has_queued_messages());
    }

    #[test]
    fn test_has_queued_messages_false_after_drain_both() {
        let mut agent = test_agent();
        agent.steer(AgentMessage::User(UserMessage::from_text("s")));
        agent.follow_up(AgentMessage::User(UserMessage::from_text("f")));
        let _ = agent.drain_steering();
        let _ = agent.drain_follow_up();
        assert!(!agent.has_queued_messages());
    }

    #[test]
    fn test_has_queued_messages_true_with_only_follow_up_after_drain_steering() {
        let mut agent = test_agent();
        agent.steer(AgentMessage::User(UserMessage::from_text("s")));
        agent.follow_up(AgentMessage::User(UserMessage::from_text("f")));
        let _ = agent.drain_steering();
        // follow_up still has messages
        assert!(agent.has_queued_messages());
    }

    // ===============================================================
    // Hooks
    // ===============================================================

    struct BlockAllHook;

    #[async_trait::async_trait]
    impl BeforeToolCallHook for BlockAllHook {
        async fn before_tool_call(&self, _ctx: &BeforeToolCallContext) -> BeforeToolCallResult {
            BeforeToolCallResult {
                block: true,
                reason: Some("blocked by test".into()),
            }
        }
    }

    struct NoopAfterHook;

    #[async_trait::async_trait]
    impl AfterToolCallHook for NoopAfterHook {
        async fn after_tool_call(&self, _ctx: &AfterToolCallContext) -> AfterToolCallResult {
            AfterToolCallResult {
                content: None,
                is_error: None,
            }
        }
    }

    #[test]
    fn test_set_before_tool_call_hook() {
        let mut agent = test_agent();
        agent.set_before_tool_call(Box::new(BlockAllHook));
        assert!(agent.has_before_tool_call_hook());
    }

    #[test]
    fn test_set_after_tool_call_hook() {
        let mut agent = test_agent();
        agent.set_after_tool_call(Box::new(NoopAfterHook));
        assert!(agent.has_after_tool_call_hook());
    }

    #[test]
    fn test_no_hooks_initially() {
        let agent = test_agent();
        assert!(!agent.has_before_tool_call_hook());
        assert!(!agent.has_after_tool_call_hook());
    }

    #[tokio::test]
    async fn test_before_tool_call_hook_called() {
        let mut agent = test_agent();
        agent.set_before_tool_call(Box::new(BlockAllHook));
        let ctx = BeforeToolCallContext {
            tool_name: "bash".into(),
            tool_call_id: "tc1".into(),
            args: json!({"command": "ls"}),
        };
        let result = agent.call_before_tool_call(&ctx).await;
        assert!(result.block, "BlockAllHook should block");
        assert_eq!(result.reason.unwrap(), "blocked by test");
    }

    #[tokio::test]
    async fn test_after_tool_call_hook_called() {
        let mut agent = test_agent();
        agent.set_after_tool_call(Box::new(NoopAfterHook));
        let ctx = AfterToolCallContext {
            tool_name: "bash".into(),
            tool_call_id: "tc1".into(),
            args: json!({}),
            is_error: false,
        };
        let result = agent.call_after_tool_call(&ctx).await;
        assert!(result.content.is_none());
        assert!(result.is_error.is_none());
    }

    #[tokio::test]
    async fn test_no_before_hook_returns_allow() {
        let agent = test_agent();
        let ctx = BeforeToolCallContext {
            tool_name: "bash".into(),
            tool_call_id: "tc1".into(),
            args: json!({}),
        };
        let result = agent.call_before_tool_call(&ctx).await;
        assert!(!result.block, "no hook should allow execution");
    }

    #[tokio::test]
    async fn test_no_after_hook_returns_noop() {
        let agent = test_agent();
        let ctx = AfterToolCallContext {
            tool_name: "bash".into(),
            tool_call_id: "tc1".into(),
            args: json!({}),
            is_error: false,
        };
        let result = agent.call_after_tool_call(&ctx).await;
        assert!(result.content.is_none());
        assert!(result.is_error.is_none());
    }

    // ===============================================================
    // Message types in queues
    // ===============================================================

    #[test]
    fn test_steer_accepts_user_message() {
        let mut agent = test_agent();
        agent.steer(AgentMessage::User(UserMessage::from_text("user msg")));
        let drained = agent.drain_steering();
        assert!(matches!(drained[0], AgentMessage::User(_)));
    }

    #[test]
    fn test_steer_accepts_assistant_message() {
        let mut agent = test_agent();
        let msg = AssistantMessage::from_text("assistant response");
        agent.steer(AgentMessage::Assistant(msg));
        let drained = agent.drain_steering();
        assert!(matches!(drained[0], AgentMessage::Assistant(_)));
    }

    #[test]
    fn test_follow_up_accepts_user_message() {
        let mut agent = test_agent();
        agent.follow_up(AgentMessage::User(UserMessage::from_text("follow")));
        let drained = agent.drain_follow_up();
        assert!(matches!(drained[0], AgentMessage::User(_)));
    }

    // ===============================================================
    // Follow-up FIFO ordering
    // ===============================================================

    #[test]
    fn test_follow_up_preserves_order() {
        let mut agent = test_agent();
        agent.follow_up(AgentMessage::User(UserMessage::from_text("first")));
        agent.follow_up(AgentMessage::User(UserMessage::from_text("second")));
        agent.follow_up(AgentMessage::User(UserMessage::from_text("third")));
        let drained = agent.drain_follow_up();
        let texts: Vec<String> = drained
            .iter()
            .map(|m| match m {
                AgentMessage::User(u) => match &u.content[0] {
                    Content::Text { text } => text.clone(),
                    _ => String::new(),
                },
                _ => String::new(),
            })
            .collect();
        assert_eq!(texts, vec!["first", "second", "third"]);
    }

    // ===============================================================
    // Hook replacement semantics
    // ===============================================================

    #[tokio::test]
    async fn test_set_before_hook_twice_replaces_previous() {
        struct AllowAllHook;

        #[async_trait::async_trait]
        impl BeforeToolCallHook for AllowAllHook {
            async fn before_tool_call(&self, _ctx: &BeforeToolCallContext) -> BeforeToolCallResult {
                BeforeToolCallResult {
                    block: false,
                    reason: None,
                }
            }
        }

        let mut agent = test_agent();
        // First: block all
        agent.set_before_tool_call(Box::new(BlockAllHook));
        // Second: allow all (should replace)
        agent.set_before_tool_call(Box::new(AllowAllHook));

        let ctx = BeforeToolCallContext {
            tool_name: "bash".into(),
            tool_call_id: "tc1".into(),
            args: json!({}),
        };
        let result = agent.call_before_tool_call(&ctx).await;
        assert!(!result.block, "second hook should replace the first");
    }

    #[tokio::test]
    async fn test_set_after_hook_twice_replaces_previous() {
        struct ContentHook;

        #[async_trait::async_trait]
        impl AfterToolCallHook for ContentHook {
            async fn after_tool_call(&self, _ctx: &AfterToolCallContext) -> AfterToolCallResult {
                AfterToolCallResult {
                    content: Some(vec![Content::Text {
                        text: "replaced".into(),
                    }]),
                    is_error: None,
                }
            }
        }

        let mut agent = test_agent();
        agent.set_after_tool_call(Box::new(NoopAfterHook));
        agent.set_after_tool_call(Box::new(ContentHook));

        let ctx = AfterToolCallContext {
            tool_name: "bash".into(),
            tool_call_id: "tc1".into(),
            args: json!({}),
            is_error: false,
        };
        let result = agent.call_after_tool_call(&ctx).await;
        assert!(
            result.content.is_some(),
            "second hook should replace the first"
        );
    }

    // ===============================================================
    // ToolResult in queues
    // ===============================================================

    #[test]
    fn test_steer_accepts_tool_result_message() {
        let mut agent = test_agent();
        let tr = ToolResultMessage {
            tool_call_id: "tc1".into(),
            tool_name: "bash".into(),
            content: vec![Content::Text {
                text: "output".into(),
            }],
            is_error: false,
            timestamp: 0,
        };
        agent.steer(AgentMessage::ToolResult(tr));
        let drained = agent.drain_steering();
        assert!(matches!(drained[0], AgentMessage::ToolResult(_)));
    }

    #[test]
    fn test_follow_up_accepts_tool_result_message() {
        let mut agent = test_agent();
        let tr = ToolResultMessage {
            tool_call_id: "tc1".into(),
            tool_name: "bash".into(),
            content: vec![Content::Text {
                text: "output".into(),
            }],
            is_error: false,
            timestamp: 0,
        };
        agent.follow_up(AgentMessage::ToolResult(tr));
        let drained = agent.drain_follow_up();
        assert!(matches!(drained[0], AgentMessage::ToolResult(_)));
    }

    // ===============================================================
    // Large queue
    // ===============================================================

    #[test]
    fn test_steer_large_batch_does_not_panic() {
        let mut agent = test_agent();
        for i in 0..200 {
            agent.steer(AgentMessage::User(UserMessage::from_text(&format!(
                "msg-{i}"
            ))));
        }
        let drained = agent.drain_steering();
        assert_eq!(drained.len(), 200);
    }

    // ===============================================================
    // Sprint 4 H2 — StopHook
    // ===============================================================

    struct PassHook;

    #[async_trait::async_trait]
    impl StopHook for PassHook {
        async fn on_stop(&self, _ctx: &StopContext) -> StopAction {
            StopAction::Pass
        }
    }

    struct ContinueHook {
        message: String,
    }

    #[async_trait::async_trait]
    impl StopHook for ContinueHook {
        async fn on_stop(&self, _ctx: &StopContext) -> StopAction {
            StopAction::Continue(self.message.clone())
        }
    }

    struct FailHook {
        reason: String,
    }

    #[async_trait::async_trait]
    impl StopHook for FailHook {
        async fn on_stop(&self, _ctx: &StopContext) -> StopAction {
            StopAction::Fail(self.reason.clone())
        }
    }

    fn test_stop_context() -> StopContext {
        StopContext {
            stop_reason: StopReason::Stop,
            session_id: "sess-test".into(),
            task_id: "task-test".into(),
            turn_count: 3,
            agent_name: "test-agent".into(),
            model: "test-model".into(),
            last_assistant_message: "Done.".into(),
        }
    }

    // ── Initial state ─────────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_no_stop_hook_initially_returns_pass() {
        // With no hook set, call_stop_hook must return Pass (the safe default).
        let agent = test_agent();
        let action = agent.call_stop_hook(&test_stop_context()).await;
        assert!(matches!(action, StopAction::Pass));
    }

    #[tokio::test]
    async fn test_set_stop_hook_takes_effect() {
        // After set_stop_hook, call_stop_hook must use the installed hook.
        let mut agent = test_agent();
        agent.set_stop_hook(Box::new(FailHook { reason: "sentinel".into() }));
        let action = agent.call_stop_hook(&test_stop_context()).await;
        assert!(matches!(action, StopAction::Fail(_)));
    }

    // ── StopAction variants ───────────────────────────────────────────────────

    #[tokio::test]
    async fn test_stop_hook_pass_returned() {
        let mut agent = test_agent();
        agent.set_stop_hook(Box::new(PassHook));
        let action = agent.call_stop_hook(&test_stop_context()).await;
        assert!(matches!(action, StopAction::Pass));
    }

    #[tokio::test]
    async fn test_stop_hook_continue_returns_message() {
        let mut agent = test_agent();
        agent.set_stop_hook(Box::new(ContinueHook {
            message: "please try again".into(),
        }));
        let action = agent.call_stop_hook(&test_stop_context()).await;
        match action {
            StopAction::Continue(msg) => {
                assert_eq!(msg, "please try again");
            }
            other => panic!("expected Continue, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_stop_hook_fail_returns_reason() {
        let mut agent = test_agent();
        agent.set_stop_hook(Box::new(FailHook {
            reason: "task did not complete successfully".into(),
        }));
        let action = agent.call_stop_hook(&test_stop_context()).await;
        match action {
            StopAction::Fail(reason) => {
                assert_eq!(reason, "task did not complete successfully");
            }
            other => panic!("expected Fail, got {other:?}"),
        }
    }

    // ── No hook → default Pass ────────────────────────────────────────────────

    #[tokio::test]
    async fn test_no_stop_hook_returns_pass() {
        let agent = test_agent();
        let action = agent.call_stop_hook(&test_stop_context()).await;
        assert!(
            matches!(action, StopAction::Pass),
            "no stop hook must return Pass, got {action:?}"
        );
    }

    // ── Hook replacement ──────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_set_stop_hook_twice_replaces_previous() {
        let mut agent = test_agent();
        // First: Fail hook
        agent.set_stop_hook(Box::new(FailHook {
            reason: "first hook".into(),
        }));
        // Second: Pass hook (should replace)
        agent.set_stop_hook(Box::new(PassHook));
        let action = agent.call_stop_hook(&test_stop_context()).await;
        assert!(
            matches!(action, StopAction::Pass),
            "second stop hook must replace the first"
        );
    }

    // ── Context fields are forwarded correctly ────────────────────────────────

    #[tokio::test]
    async fn test_stop_hook_receives_correct_context() {
        use std::sync::{Arc, Mutex};

        struct CaptureHook {
            captured: Arc<Mutex<Option<(String, String, String, usize, String, String, String)>>>,
        }

        #[async_trait::async_trait]
        impl StopHook for CaptureHook {
            async fn on_stop(&self, ctx: &StopContext) -> StopAction {
                *self.captured.lock().unwrap() = Some((
                    ctx.session_id.clone(),
                    ctx.task_id.clone(),
                    ctx.agent_name.clone(),
                    ctx.turn_count,
                    ctx.last_assistant_message.clone(),
                    ctx.model.clone(),
                    format!("{}", ctx.stop_reason),
                ));
                StopAction::Pass
            }
        }

        let captured = Arc::new(Mutex::new(None));
        let mut agent = test_agent();
        agent.set_stop_hook(Box::new(CaptureHook {
            captured: captured.clone(),
        }));

        let ctx = StopContext {
            stop_reason: StopReason::Stop,
            session_id: "sess-abc".into(),
            task_id: "task-xyz".into(),
            turn_count: 7,
            agent_name: "my-agent".into(),
            model: "claude-haiku".into(),
            last_assistant_message: "Task complete.".into(),
        };
        agent.call_stop_hook(&ctx).await;

        let (session_id, task_id, name, turns, last_msg, model, stop_reason) = captured
            .lock()
            .unwrap()
            .take()
            .expect("hook should have been called");
        assert_eq!(session_id, "sess-abc", "session_id must be forwarded");
        assert_eq!(task_id, "task-xyz", "task_id must be forwarded");
        assert_eq!(name, "my-agent", "agent_name must be forwarded");
        assert_eq!(turns, 7, "turn_count must be forwarded");
        assert_eq!(last_msg, "Task complete.", "last_assistant_message must be forwarded");
        assert_eq!(model, "claude-haiku", "model must be forwarded");
        assert_eq!(stop_reason, "stop", "stop_reason must be forwarded as display string");
    }

    // ── StopReason variants reach the hook ────────────────────────────────────

    #[tokio::test]
    async fn test_stop_hook_receives_max_turns_stop_reason() {
        use std::sync::{Arc, Mutex};

        struct ReasonCapture {
            captured: Arc<Mutex<Option<StopReason>>>,
        }

        #[async_trait::async_trait]
        impl StopHook for ReasonCapture {
            async fn on_stop(&self, ctx: &StopContext) -> StopAction {
                *self.captured.lock().unwrap() = Some(ctx.stop_reason.clone());
                StopAction::Pass
            }
        }

        let captured = Arc::new(Mutex::new(None));
        let mut agent = test_agent();
        agent.set_stop_hook(Box::new(ReasonCapture {
            captured: captured.clone(),
        }));

        let ctx = StopContext {
            stop_reason: StopReason::Length,
            session_id: "s".into(),
            task_id: "t".into(),
            turn_count: 30,
            agent_name: "a".into(),
            model: "m".into(),
            last_assistant_message: "".into(),
        };
        agent.call_stop_hook(&ctx).await;

        let reason = captured.lock().unwrap().take().expect("hook must be called");
        assert!(
            matches!(reason, StopReason::Length),
            "hook must receive Length stop reason"
        );
    }

    // ── Edge: empty last_assistant_message ────────────────────────────────────

    #[tokio::test]
    async fn test_stop_hook_empty_last_message_does_not_panic() {
        let mut agent = test_agent();
        agent.set_stop_hook(Box::new(PassHook));
        let ctx = StopContext {
            stop_reason: StopReason::Stop,
            session_id: "s".into(),
            task_id: "t".into(),
            turn_count: 0,
            agent_name: "a".into(),
            model: "m".into(),
            last_assistant_message: "".into(), // empty — valid for first turn abort
        };
        let action = agent.call_stop_hook(&ctx).await;
        assert!(matches!(action, StopAction::Pass));
    }

    // ── All StopReason variants reach the hook ────────────────────────────────

    #[tokio::test]
    async fn test_stop_hook_receives_error_stop_reason() {
        use std::sync::{Arc, Mutex};

        struct ReasonCapture {
            captured: Arc<Mutex<Option<StopReason>>>,
        }

        #[async_trait::async_trait]
        impl StopHook for ReasonCapture {
            async fn on_stop(&self, ctx: &StopContext) -> StopAction {
                *self.captured.lock().unwrap() = Some(ctx.stop_reason.clone());
                StopAction::Pass
            }
        }

        let captured = Arc::new(Mutex::new(None));
        let mut agent = test_agent();
        agent.set_stop_hook(Box::new(ReasonCapture {
            captured: captured.clone(),
        }));

        let ctx = StopContext {
            stop_reason: StopReason::Error,
            session_id: "s".into(),
            task_id: "t".into(),
            turn_count: 1,
            agent_name: "a".into(),
            model: "m".into(),
            last_assistant_message: "".into(),
        };
        agent.call_stop_hook(&ctx).await;
        let reason = captured.lock().unwrap().take().unwrap();
        assert!(matches!(reason, StopReason::Error));
    }

    #[tokio::test]
    async fn test_stop_hook_receives_aborted_stop_reason() {
        use std::sync::{Arc, Mutex};

        struct ReasonCapture {
            captured: Arc<Mutex<Option<StopReason>>>,
        }

        #[async_trait::async_trait]
        impl StopHook for ReasonCapture {
            async fn on_stop(&self, ctx: &StopContext) -> StopAction {
                *self.captured.lock().unwrap() = Some(ctx.stop_reason.clone());
                StopAction::Pass
            }
        }

        let captured = Arc::new(Mutex::new(None));
        let mut agent = test_agent();
        agent.set_stop_hook(Box::new(ReasonCapture {
            captured: captured.clone(),
        }));

        let ctx = StopContext {
            stop_reason: StopReason::Aborted,
            session_id: "s".into(),
            task_id: "t".into(),
            turn_count: 2,
            agent_name: "a".into(),
            model: "m".into(),
            last_assistant_message: "Partially done.".into(),
        };
        agent.call_stop_hook(&ctx).await;
        let reason = captured.lock().unwrap().take().unwrap();
        assert!(matches!(reason, StopReason::Aborted));
    }

    #[tokio::test]
    async fn test_stop_hook_receives_tool_use_stop_reason() {
        use std::sync::{Arc, Mutex};

        struct ReasonCapture {
            captured: Arc<Mutex<Option<StopReason>>>,
        }

        #[async_trait::async_trait]
        impl StopHook for ReasonCapture {
            async fn on_stop(&self, ctx: &StopContext) -> StopAction {
                *self.captured.lock().unwrap() = Some(ctx.stop_reason.clone());
                StopAction::Pass
            }
        }

        let captured = Arc::new(Mutex::new(None));
        let mut agent = test_agent();
        agent.set_stop_hook(Box::new(ReasonCapture {
            captured: captured.clone(),
        }));

        let ctx = StopContext {
            stop_reason: StopReason::ToolUse,
            session_id: "s".into(),
            task_id: "t".into(),
            turn_count: 4,
            agent_name: "a".into(),
            model: "m".into(),
            last_assistant_message: "Using tool...".into(),
        };
        agent.call_stop_hook(&ctx).await;
        let reason = captured.lock().unwrap().take().unwrap();
        assert!(matches!(reason, StopReason::ToolUse));
    }

    // ── State combination: stateless behavior verification ────────────────────

    #[tokio::test]
    async fn test_stop_hook_is_stateless_multiple_calls_same_result() {
        // call_stop_hook must return a consistent result across multiple calls
        // with the same hook installed (hook is not consumed after one call).
        let mut agent = test_agent();
        agent.set_stop_hook(Box::new(PassHook));
        let ctx = test_stop_context();

        let action1 = agent.call_stop_hook(&ctx).await;
        let action2 = agent.call_stop_hook(&ctx).await;
        let action3 = agent.call_stop_hook(&ctx).await;

        assert!(
            matches!(action1, StopAction::Pass),
            "first call must be Pass"
        );
        assert!(
            matches!(action2, StopAction::Pass),
            "second call must be Pass (hook not consumed)"
        );
        assert!(
            matches!(action3, StopAction::Pass),
            "third call must be Pass (hook reusable)"
        );
    }

    #[tokio::test]
    async fn test_call_count_tracked_by_stateful_hook() {
        // A stateful hook tracks how many times it's been called.
        // This verifies call_stop_hook does not consume or reset the hook.
        use std::sync::{
            Arc,
            atomic::{AtomicU32, Ordering},
        };

        struct CountingHook {
            count: Arc<AtomicU32>,
        }

        #[async_trait::async_trait]
        impl StopHook for CountingHook {
            async fn on_stop(&self, _ctx: &StopContext) -> StopAction {
                self.count.fetch_add(1, Ordering::SeqCst);
                StopAction::Pass
            }
        }

        let count = Arc::new(AtomicU32::new(0));
        let mut agent = test_agent();
        agent.set_stop_hook(Box::new(CountingHook {
            count: count.clone(),
        }));

        let ctx = test_stop_context();
        agent.call_stop_hook(&ctx).await;
        agent.call_stop_hook(&ctx).await;
        agent.call_stop_hook(&ctx).await;

        assert_eq!(
            count.load(Ordering::SeqCst),
            3,
            "hook must be called exactly as many times as call_stop_hook is invoked"
        );
    }

    #[tokio::test]
    async fn test_stop_hook_replace_and_call_cycle() {
        // State machine: install hook A → call → replace with hook B → call.
        // Verifies hook lifecycle management (install, call, replace, call).
        let mut agent = test_agent();

        // Phase 1: ContinueHook installed and called
        agent.set_stop_hook(Box::new(ContinueHook {
            message: "phase1_feedback".into(),
        }));
        let ctx = test_stop_context();
        let action1 = agent.call_stop_hook(&ctx).await;
        assert!(matches!(action1, StopAction::Continue(_)));

        // Phase 2: replaced with PassHook — previous hook fully discarded
        agent.set_stop_hook(Box::new(PassHook));
        let action2 = agent.call_stop_hook(&ctx).await;
        assert!(
            matches!(action2, StopAction::Pass),
            "after replacement, new hook must be called, not the previous one"
        );
    }

    // ── Hook panic safety ─────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_continue_action_message_is_non_empty() {
        // When a Continue is returned, the injected message must be a non-empty
        // string — an empty feedback message would silently fail the harness.
        let mut agent = test_agent();
        agent.set_stop_hook(Box::new(ContinueHook {
            message: "meaningful feedback".into(),
        }));
        let ctx = test_stop_context();
        let action = agent.call_stop_hook(&ctx).await;
        match action {
            StopAction::Continue(msg) => {
                assert!(
                    !msg.is_empty(),
                    "Continue message must not be empty when hook provides feedback"
                );
                assert_eq!(msg, "meaningful feedback");
            }
            other => panic!("expected Continue, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_fail_action_reason_is_non_empty() {
        // A Fail action must carry a non-empty reason — empty string would make
        // test failure silent.
        let mut agent = test_agent();
        agent.set_stop_hook(Box::new(FailHook {
            reason: "assertion failed: output did not match expected".into(),
        }));
        let ctx = test_stop_context();
        let action = agent.call_stop_hook(&ctx).await;
        match action {
            StopAction::Fail(reason) => {
                assert!(
                    !reason.is_empty(),
                    "Fail reason must not be empty"
                );
                assert!(reason.contains("assertion failed"));
            }
            other => panic!("expected Fail, got {other:?}"),
        }
    }
}
